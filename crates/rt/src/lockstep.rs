// SPDX-License-Identifier: GPL-3.0-or-later

//! Function-level lockstep: run the original and the port from the same state, compare the
//! results, then continue with the original's results so one bug cannot cascade.
//!
//! Only game memory and registers can be rolled back, so whatever reaches outside them happens
//! once, for the original: calls to external functions (the SDK layer's devices and services),
//! hardware register accesses, and interrupts, which are taken wherever the original lets them
//! in. Each is logged in order with the memory writes and time it caused. The port must make
//! the same interactions in the same order; each is replayed from the log instead of happening
//! again, and any other sequence is a mismatch.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any, resume_unwind};

use gekko_fp::Ps;

use crate::{Ctx, Hook, Mmio, Native, PAGE_SIZE, Pages};

/// Stack below the caller's r1 holds frames and scratch that ports need not reproduce.
pub const STACK_SCRATCH: u32 = 0x1_0000;

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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Original,
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
    log: RefCell<VecDeque<Interaction>>,
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
        self.phase.get() == Phase::Original
    }

    pub fn clear(&self) {
        self.mismatches.borrow_mut().clear();
        self.stats.borrow_mut().clear();
    }
}

fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
    if let Some(f) = p.downcast_ref::<crate::Fault>() {
        f.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_owned()
    } else {
        "panic".to_owned()
    }
}

pub(crate) fn run(ctx: &Ctx, addr: u32, native: Native, returns: Returns) {
    let state = &ctx.lockstep;
    state.active.set(true);
    state.log.borrow_mut().clear();
    let regs0 = ctx.regs.snapshot();
    let sp = regs0.gpr[1];

    state.phase.set(Phase::Original);
    ctx.mem.begin_journal();
    let original = catch_unwind(AssertUnwindSafe(|| ctx.run_original(addr)))
        .err()
        .map(panic_text);
    let j1 = ctx.mem.end_journal();
    let regs1 = ctx.regs.snapshot();
    let s1 = ctx.mem.capture(j1.keys());

    ctx.mem.restore(&j1);
    ctx.regs.restore(&regs0);
    state.phase.set(Phase::Port);
    ctx.mem.begin_journal();
    let mut port = catch_unwind(AssertUnwindSafe(|| native(ctx)))
        .err()
        .map(panic_text);
    let j2 = ctx.mem.end_journal();
    let regs2 = ctx.regs.snapshot();
    state.phase.set(Phase::Idle);
    let left = state.log.borrow_mut().pop_front();
    if let (None, None, Some(x)) = (&original, &port, left) {
        port = Some(format!(
            "the port stopped before the original's {}",
            describe(ctx, x.kind)
        ));
    }

    let mut diffs = Vec::new();
    if original.is_some() || port.is_some() {
        diffs.push(Diff::Panic {
            original: original.clone(),
            port,
        });
    }
    let mut regs = vec![("r1", u64::from(regs1.gpr[1]), u64::from(regs2.gpr[1]))];
    let r3 = ("r3", u64::from(regs1.gpr[3]), u64::from(regs2.gpr[3]));
    let r4 = ("r4", u64::from(regs1.gpr[4]), u64::from(regs2.gpr[4]));
    let f1 = ("f1", regs1.fpr[1].ps0.to_bits(), regs2.fpr[1].ps0.to_bits());
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
    compare_pages(ctx, &j1, &s1, &j2, sp, &mut diffs);

    // Continue from the original's results.
    ctx.mem.restore(&j2);
    ctx.mem.restore(&s1);
    ctx.regs.restore(&regs1);
    state.active.set(false);

    let mut stats = state.stats.borrow_mut();
    let entry = stats.entry(addr).or_default();
    entry.calls += 1;
    if !diffs.is_empty() {
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
    if let Some(msg) = original {
        panic!(
            "original {} panicked under lockstep: {msg}",
            ctx.name_of(addr)
        );
    }
}

fn describe(ctx: &Ctx, kind: Kind) -> String {
    match kind {
        Kind::Call(addr) => format!("call to {}", ctx.name_of(addr)),
        Kind::Read { addr, len } => format!("{len}-byte read of {addr:#010X}"),
        Kind::Write { addr, len, value } => {
            format!("{len}-byte write of {value:#X} to {addr:#010X}")
        }
        Kind::Interrupts => "interrupt point".to_owned(),
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
            state.log.borrow_mut().push_back(Interaction {
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
        Phase::Port => {
            let next = state.log.borrow_mut().pop_front();
            match next {
                Some(x) if x.kind == kind => {
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
                other => panic_any(format!(
                    "the port made a {} where the original made {}",
                    describe(ctx, kind),
                    other.map_or_else(|| "none".to_owned(), |x| describe(ctx, x.kind))
                )),
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

/// A point where pending interrupts may be taken.
pub(crate) fn interrupts(ctx: &Ctx, check: &Hook) {
    interact(ctx, Kind::Interrupts, || {
        check(ctx);
        0
    });
}

fn compare_pages(ctx: &Ctx, j1: &Pages, s1: &Pages, j2: &Pages, sp: u32, diffs: &mut Vec<Diff>) {
    // Frames below the caller's stack pointer, plus the back chain and saved LR words the
    // callee writes into the caller's frame, are linkage rather than results.
    let sp = sp & 0x3FFF_FFFF;
    let scratch = sp.saturating_sub(STACK_SCRATCH)..sp + 8;
    let mut pages: Vec<u32> = j1.keys().chain(j2.keys()).copied().collect();
    pages.sort_unstable();
    pages.dedup();
    for page in pages {
        let expected = s1.get(&page).or_else(|| j2.get(&page)).unwrap();
        let actual = ctx.mem.capture([page].iter());
        let actual = &actual[&page];
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
}
