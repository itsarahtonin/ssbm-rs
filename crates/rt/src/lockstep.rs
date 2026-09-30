// SPDX-License-Identifier: GPL-3.0-or-later

//! Function-level lockstep: run the original and the port from the same state, compare the
//! results, then continue with the original's results so one bug cannot cascade. The original
//! runs exactly as it would without the check, so the game goes on as it would.
//!
//! Only game memory and registers can be rolled back, so whatever reaches outside them happens
//! once, for the original: calls to external functions (the SDK layer's devices and services),
//! hardware register accesses, and interrupts, which are taken wherever the original lets them
//! in. Each is logged in order with the memory writes and time it caused. The port must make
//! the same interactions in the same order; each is replayed from the log instead of happening
//! again, and any other sequence is a mismatch.
//!
//! Checks nest: when a port under check calls another port, that call is checked too, against
//! its original run from the same state, replaying the same stretch of the log. So a mismatch
//! is reported at the innermost port that has it, and callers still see correct results.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any, resume_unwind};

use gekko_fp::Ps;

use crate::{Ctx, Hook, Mmio, Native, PAGE_SIZE, Pages};

/// Stack below the caller's r1 holds frames and scratch that ports need not reproduce.
pub const STACK_SCRATCH: u32 = 0x1_0000;

/// Stack below r1 cleared for the port. Ports lay out their frames differently, so bytes it
/// reads but never writes, such as a local struct's padding, would otherwise be leftovers from
/// other addresses than the original's.
const STACK_CLEARED: u32 = 0x1000;

/// Which registers hold a function's result, and so are compared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Returns {
    /// r3, r4 and f1, for functions whose signature is not known.
    #[default]
    Unknown,
    Nothing,
    /// r3.
    Int,
    /// r3 and r4.
    Int64,
    /// f1.
    Float,
}

/// One difference between the original's and the port's results.
#[derive(Clone, Debug, PartialEq)]
pub enum Diff {
    Reg {
        name: &'static str,
        original: u64,
        port: u64,
    },
    Mem {
        addr: u32,
        original: Vec<u8>,
        port: Vec<u8>,
    },
    Panic {
        original: Option<String>,
        port: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Mismatch {
    pub function: u32,
    /// Which call of the function this was, counting from 1.
    pub call: u64,
    pub diffs: Vec<Diff>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub calls: u64,
    pub mismatches: u64,
    /// Calls whose results differ only because the original reads stack memory it never
    /// wrote: run again on the port's cleared stack, it matches the port, or cannot follow
    /// its own log.
    pub uninitialized: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// The outermost original run: interactions happen and are logged.
    Original,
    /// The original run of a nested check: interactions replay from the log.
    Replay,
    Port,
}

/// What an interaction was, for matching the port's against the original's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Call(u32),
    Read {
        addr: u32,
        len: u32,
    },
    Write {
        addr: u32,
        len: u32,
        value: u32,
    },
    /// A point where pending interrupts could be taken.
    Interrupts,
    /// A hook reached by original code, such as the SDK layer's wait points.
    Hook(u32),
}

/// One interaction the original had with the world outside game memory.
struct Interaction {
    kind: Kind,
    /// Memory written meanwhile, by the callee or by interrupt handlers.
    writes: Vec<(u32, Vec<u8>)>,
    /// Results: a register read's value, or a call's return registers.
    value: u32,
    r4: u32,
    f1: Ps,
    cr: u32,
    /// The time base afterwards.
    tb: u64,
}

#[derive(Default)]
pub struct State {
    active: Cell<bool>,
    phase: Cell<Phase>,
    /// Nesting of interactions while the original runs; only the outermost is logged.
    depth: Cell<u32>,
    log: RefCell<Vec<Interaction>>,
    /// The next interaction to replay.
    cursor: Cell<usize>,
    /// Mismatches kept per function; later ones only count in `stats`.
    pub keep_per_function: Cell<usize>,
    pub mismatches: RefCell<Vec<Mismatch>>,
    pub stats: RefCell<BTreeMap<u32, Stats>>,
}

impl State {
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// Whether the original side of a check is running.
    pub fn in_original(&self) -> bool {
        matches!(self.phase.get(), Phase::Original | Phase::Replay)
    }

    pub fn clear(&self) {
        self.mismatches.borrow_mut().clear();
        self.stats.borrow_mut().clear();
    }
}

/// Panic payload: a check's side made a different interaction than the log holds.
struct Diverged(String);

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(d) = p.downcast_ref::<Diverged>() {
        d.0.clone()
    } else if let Some(f) = p.downcast_ref::<crate::Fault>() {
        f.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_owned()
    } else {
        "panic".to_owned()
    }
}

/// Checks a call to the port of `addr`: the outermost check when none is running, or a nested
/// one when a port under check calls another.
pub(crate) fn run(ctx: &Ctx, addr: u32, native: Native, returns: Returns) {
    let state = &ctx.lockstep;
    let outermost = !state.active.get();
    let enclosing = state.phase.get();
    if outermost {
        state.active.set(true);
        CHECKING.with(|c| c.set(true));
        state.log.borrow_mut().clear();
        state.cursor.set(0);
    }
    let regs0 = ctx.regs.snapshot();
    let sp = regs0.gpr[1];
    let start = state.cursor.get();
    let natives = ctx.natives_depth();

    state.phase.set(if outermost {
        Phase::Original
    } else {
        Phase::Replay
    });
    ctx.mem.begin_journal();
    let original_panic = catch_unwind(AssertUnwindSafe(|| ctx.run_original(addr))).err();
    let original = original_panic.as_ref().map(|p| panic_text(p.as_ref()));
    let j1 = ctx.mem.end_journal();
    let regs1 = ctx.regs.snapshot();
    let s1 = ctx.mem.capture(j1.keys());
    let end = if outermost {
        state.log.borrow().len()
    } else {
        state.cursor.get()
    };

    ctx.mem.restore(&j1);
    ctx.regs.restore(&regs0);
    state.phase.set(Phase::Port);
    state.cursor.set(start);
    ctx.mem.begin_journal();
    clear_stack(ctx, sp);
    let mut port = catch_unwind(AssertUnwindSafe(|| ctx.run_native(addr, native)))
        .err()
        .map(|p| {
            // Name the ports that were running when it failed, innermost first.
            let inner: Vec<String> = ctx.native_stack()[natives..]
                .iter()
                .rev()
                .take(4)
                .map(|a| ctx.name_of(*a))
                .collect();
            format!("{} (in {})", panic_text(p.as_ref()), inner.join(" < "))
        });
    ctx.truncate_natives(natives);
    let j2 = ctx.mem.end_journal();
    let regs2 = ctx.regs.snapshot();
    let s2 = ctx.mem.capture(j2.keys());
    let at = state.cursor.get();
    if original.is_none() && port.is_none() && at < end {
        port = Some(format!(
            "the port stopped before the original's {}",
            describe(ctx, state.log.borrow()[at].kind)
        ));
    }
    let ported = Outcome {
        before: &j2,
        after: &s2,
        regs: &regs2,
        panic: port,
    };
    let mut diffs = compare(
        &Outcome {
            before: &j1,
            after: &s1,
            regs: &regs1,
            panic: original.clone(),
        },
        &ported,
        returns,
        sp,
    );

    // The port saw a cleared stack. Run the original again on one: if it cannot follow its
    // own log then, or matches the port, it reads stack it never wrote, which the port
    // cannot reproduce. Otherwise the port differs from it given the same stack.
    let mut uninitialized = false;
    if !diffs.is_empty() && original.is_none() {
        ctx.mem.restore(&j2);
        ctx.regs.restore(&regs0);
        state.phase.set(Phase::Replay);
        state.cursor.set(start);
        ctx.mem.begin_journal();
        clear_stack(ctx, sp);
        let again = catch_unwind(AssertUnwindSafe(|| ctx.run_original(addr)));
        let j3 = ctx.mem.end_journal();
        let regs3 = ctx.regs.snapshot();
        if again.is_ok() && state.cursor.get() == end {
            let s3 = ctx.mem.capture(j3.keys());
            let cleared = Outcome {
                before: &j3,
                after: &s3,
                regs: &regs3,
                panic: None,
            };
            diffs = compare(&cleared, &ported, returns, sp);
            uninitialized = diffs.is_empty();
        } else {
            uninitialized = true;
        }
        ctx.mem.restore(&j3);
    }

    // Continue from the original's results.
    ctx.mem.restore(&j2);
    ctx.mem.restore(&s1);
    ctx.regs.restore(&regs1);
    state.cursor.set(end);
    state.phase.set(enclosing);
    if outermost {
        state.active.set(false);
        CHECKING.with(|c| c.set(false));
    }

    // A nested original that cannot follow the enclosing log, or faults, was called with
    // different inputs: the enclosing port is at fault, and its own check reports it.
    if !outermost && let Some(p) = original_panic {
        resume_unwind(p);
    }
    let mut stats = state.stats.borrow_mut();
    let entry = stats.entry(addr).or_default();
    entry.calls += 1;
    if uninitialized {
        entry.uninitialized += 1;
    } else if !diffs.is_empty() {
        entry.mismatches += 1;
        let keep = state.keep_per_function.get().max(1) as u64;
        if entry.mismatches <= keep {
            state.mismatches.borrow_mut().push(Mismatch {
                function: addr,
                call: entry.calls,
                diffs,
            });
        }
    }
    drop(stats);
    // The original's panic is the run's to handle, such as the end of a run.
    if let Some(p) = original_panic {
        resume_unwind(p);
    }
}

fn clear_stack(ctx: &Ctx, sp: u32) {
    let start = sp.saturating_sub(STACK_CLEARED);
    let _ = ctx.mem.write_bytes(start, &[0; STACK_CLEARED as usize]);
}

fn describe(ctx: &Ctx, kind: Kind) -> String {
    match kind {
        Kind::Call(addr) => format!("call to {}", ctx.name_of(addr)),
        Kind::Read { addr, len } => format!("{len}-byte read of {addr:#010X}"),
        Kind::Write { addr, len, value } => {
            format!("{len}-byte write of {value:#X} to {addr:#010X}")
        }
        Kind::Interrupts => "interrupt point".to_owned(),
        Kind::Hook(addr) => format!("hook at {}", ctx.name_of(addr)),
    }
}

/// Runs `f` as one interaction: logged for the original, replayed for the port. `f` returns
/// the value a register read produced.
fn interact(ctx: &Ctx, kind: Kind, f: impl FnOnce() -> u32) -> u32 {
    let state = &ctx.lockstep;
    match state.phase.get() {
        Phase::Original if state.depth.get() == 0 => {
            state.depth.set(1);
            ctx.mem.begin_log();
            let result = catch_unwind(AssertUnwindSafe(f));
            let writes = ctx.mem.end_log();
            state.depth.set(0);
            let value = match result {
                Ok(v) => v,
                Err(p) => resume_unwind(p),
            };
            let regs = &ctx.regs;
            state.log.borrow_mut().push(Interaction {
                kind,
                writes,
                value,
                r4: regs.r(4),
                f1: regs.fpr[1].get(),
                cr: regs.cr.get(),
                tb: regs.tb.get(),
            });
            value
        }
        Phase::Port | Phase::Replay => {
            let at = state.cursor.get();
            let log = state.log.borrow();
            match log.get(at) {
                Some(x) if x.kind == kind => {
                    state.cursor.set(at + 1);
                    for (at, bytes) in &x.writes {
                        // The original's writes were to mapped memory, so these succeed.
                        let _ = ctx.mem.write_bytes(*at, bytes);
                    }
                    ctx.regs.tb.set(x.tb);
                    if let Kind::Call(_) = kind {
                        ctx.regs.set_r(3, x.value);
                        ctx.regs.set_r(4, x.r4);
                        ctx.regs.fpr[1].set(x.f1);
                        ctx.regs.cr.set(x.cr);
                    }
                    x.value
                }
                other => {
                    let other = other.map(|x| x.kind);
                    drop(log);
                    panic_any(Diverged(format!(
                        "the port made a {} where the original made {}",
                        describe(ctx, kind),
                        other.map_or_else(|| "none".to_owned(), |k| describe(ctx, k))
                    )))
                }
            }
        }
        _ => f(),
    }
}

/// Calls an external function.
pub(crate) fn external(ctx: &Ctx, addr: u32, native: Native) {
    interact(ctx, Kind::Call(addr), || {
        native(ctx);
        ctx.regs.r(3)
    });
}

/// Reads a hardware register.
pub(crate) fn mmio_read(ctx: &Ctx, mmio: &dyn Mmio, addr: u32, len: u32) -> u32 {
    interact(ctx, Kind::Read { addr, len }, || mmio.read(ctx, addr, len))
}

/// Writes a hardware register.
pub(crate) fn mmio_write(ctx: &Ctx, mmio: &dyn Mmio, addr: u32, len: u32, value: u32) {
    interact(ctx, Kind::Write { addr, len, value }, || {
        mmio.write(ctx, addr, len, value);
        0
    });
}

/// A hook that original code reached.
pub(crate) fn hook(ctx: &Ctx, addr: u32, hook: &Hook) {
    interact(ctx, Kind::Hook(addr), || {
        hook(ctx);
        0
    });
}

thread_local! {
    static CHECKING: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is inside a lockstep check, whose panics lockstep catches and reports.
pub fn checking() -> bool {
    CHECKING.with(Cell::get)
}

/// A point where pending interrupts may be taken.
pub(crate) fn interrupts(ctx: &Ctx, check: &Hook) {
    interact(ctx, Kind::Interrupts, || {
        check(ctx);
        0
    });
}

/// What one side of a check left: the original contents of the pages it wrote and their
/// contents afterwards, its registers, and how it failed, if it did.
struct Outcome<'a> {
    before: &'a Pages,
    after: &'a Pages,
    regs: &'a crate::RegsSnapshot,
    panic: Option<String>,
}

/// The differences between two runs from the same state: their failures, result registers
/// and memory.
fn compare(a: &Outcome, b: &Outcome, returns: Returns, sp: u32) -> Vec<Diff> {
    let mut diffs = Vec::new();
    if a.panic.is_some() || b.panic.is_some() {
        diffs.push(Diff::Panic {
            original: a.panic.clone(),
            port: b.panic.clone(),
        });
    }
    let (ra, rb) = (a.regs, b.regs);
    let mut regs = vec![("r1", u64::from(ra.gpr[1]), u64::from(rb.gpr[1]))];
    let r3 = ("r3", u64::from(ra.gpr[3]), u64::from(rb.gpr[3]));
    let r4 = ("r4", u64::from(ra.gpr[4]), u64::from(rb.gpr[4]));
    let f1 = ("f1", ra.fpr[1].ps0.to_bits(), rb.fpr[1].ps0.to_bits());
    match returns {
        Returns::Unknown => regs.extend([r3, r4, f1]),
        Returns::Nothing => {}
        Returns::Int => regs.push(r3),
        Returns::Int64 => regs.extend([r3, r4]),
        Returns::Float => regs.push(f1),
    }
    for (name, o, p) in regs {
        if o != p {
            diffs.push(Diff::Reg {
                name,
                original: o,
                port: p,
            });
        }
    }

    // Frames below the caller's stack pointer, plus the back chain and saved LR words the
    // callee writes into the caller's frame, are linkage rather than results.
    let sp = sp & 0x3FFF_FFFF;
    let scratch = sp.saturating_sub(STACK_SCRATCH)..sp + 8;
    let mut pages: Vec<u32> = a.after.keys().chain(b.after.keys()).copied().collect();
    pages.sort_unstable();
    pages.dedup();
    for page in pages {
        // A page one side did not write still holds what both started from.
        let base = a.before.get(&page).or_else(|| b.before.get(&page)).unwrap();
        let expected = a.after.get(&page).unwrap_or(base);
        let actual = b.after.get(&page).unwrap_or(base);
        let mut i = 0;
        while i < PAGE_SIZE as usize {
            let phys = page * PAGE_SIZE + i as u32;
            if expected[i] == actual[i] || scratch.contains(&phys) {
                i += 1;
                continue;
            }
            let start = i;
            while i < PAGE_SIZE as usize && expected[i] != actual[i] {
                i += 1;
            }
            diffs.push(Diff::Mem {
                addr: 0x8000_0000 | (page * PAGE_SIZE + start as u32),
                original: expected[start..i].to_vec(),
                port: actual[start..i].to_vec(),
            });
        }
    }
    diffs
}
