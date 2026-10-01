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
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any, resume_unwind};

use gekko_fp::Ps;

use crate::{Ctx, Hook, Mmio, Native, PAGE_SIZE, Pages};

/// Stack below the caller's r1 holds frames and scratch that ports need not reproduce.
pub const STACK_SCRATCH: u32 = 0x1_0000;

/// Stack below r1 cleared for the port. Ports lay out their frames differently, so bytes it
/// reads but never writes, such as a local struct's padding, would otherwise be leftovers from
/// other addresses than the original's.
const STACK_CLEARED: u32 = 0x1000;

/// What decides a branch to code a function's checks have not reached, which its mutated
/// checks change (see `tools/lockstep/targets.py`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// What a callee returns, which a stand-in returns instead.
    Call(u32),
    /// An argument register (r3 to r10), compared with `value`, or with its `bits` tested.
    Reg { reg: usize, value: u32, bits: bool },
    /// What the function's load at `pc` reads, `size` bytes, compared or tested likewise.
    Load { pc: u32, size: u32, value: u32, bits: bool },
}

/// Which registers hold a function's result, and so are compared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Returns {
    /// r3, r4 and f1, for functions whose signature is not known.
    #[default]
    Unknown,
    Nothing,
    /// r3.
    Int,
    /// r3's low byte or halfword: MWCC leaves the rest as it happens to be, and callers
    /// extend the value themselves.
    Int8,
    Int16,
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
    /// The first of the checked function's own calls that differs between the sides, as
    /// (callee, r3 to r6), when calls are traced.
    Call {
        index: usize,
        original: Option<(String, [u32; 4])>,
        port: Option<(String, [u32; 4])>,
    },
}

/// The calls one check's function makes itself, on each side, while calls are traced.
#[derive(Default)]
struct CallTrace {
    depth: u32,
    port_side: bool,
    /// Whether calls at every depth count, not only the function's own.
    deep: bool,
    original: Vec<(u32, [u32; 4])>,
    port: Vec<(u32, [u32; 4])>,
}

#[derive(Clone, Debug)]
pub struct Mismatch {
    pub function: u32,
    /// Which call of the function this was, counting from 1.
    pub call: u64,
    /// Whether its inputs were changed at random from a call's.
    pub mutated: bool,
    pub diffs: Vec<Diff>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub calls: u64,
    /// Instructions the original ran in these checks.
    pub cost: u64,
    pub mismatches: u64,
    /// Calls whose results differ only because the original reads stack memory it never
    /// wrote: run again on the port's cleared stack, it matches the port, or cannot follow
    /// its own log.
    pub uninitialized: u64,
    /// Mismatching calls whose inputs were changed at random from a call's.
    pub mutated_mismatches: u64,
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
    /// r1 when it happened: what a callee or interrupt handler wrote below it is dead stack.
    sp: u32,
    /// The original's return address then, which names where it happened.
    lr: u32,
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
    /// The functions whose checks are running, outermost first.
    checking: RefCell<Vec<u32>>,
    /// Mismatches kept per function; later ones only count in `stats`.
    pub keep_per_function: Cell<usize>,
    /// Calls to check per function, if limited: past them its port runs unchecked, so a long
    /// run spends its checks on what it has not checked yet.
    pub calls_per_function: Cell<Option<u64>>,
    /// Checks each port gets until its checks' originals have run this many instructions.
    pub budget_per_function: Cell<Option<u64>>,
    /// Further checks of each outermost check's function from the same call, with its
    /// arguments and what they point to changed at random, to reach more of its code.
    pub mutations: Cell<u32>,
    mutating: Cell<bool>,
    pub rng: Cell<u64>,
    /// What decides the branches to code each function's checks have not reached: half its
    /// mutated checks change one of these.
    pub targets: RefCell<BTreeMap<u32, Vec<Target>>>,
    /// The callee the running mutated check stands in for, and the r3 and f1 it returns.
    stub: Cell<Option<(u32, u32, f64)>>,
    /// While an outermost check's original runs, its function's loads that targets name, and
    /// where they read.
    watch: RefCell<Vec<u32>>,
    watching: Cell<bool>,
    loaded: RefCell<Vec<(u32, u32)>>,
    /// The game's code, from its first address to past its last: a mutated check whose
    /// original writes there goes on running code only it sees changed.
    pub code: Cell<(u32, u32)>,
    /// Words of main memory outside `code` that the interpreter has run, such as injected code
    /// playback placed in the heap, a bit per word: code too.
    ram_code: RefCell<Vec<u64>>,
    /// Read-only data, whose constants ports hold inline rather than read: mutated checks
    /// leave it as it is.
    pub constant: RefCell<Vec<(u32, u32)>>,
    /// Ports that run as ports even on a mutated check's original side, which otherwise runs
    /// original code throughout.
    pub always_native: RefCell<BTreeSet<u32>>,
    /// Functions that never return, such as `__assert` and `OSPanic`: a mutated check that
    /// calls one ends there, as one whose inputs fail an assertion.
    pub noreturn: RefCell<BTreeSet<u32>>,
    /// Calls a port under a mutated check may still make, which stops one that would never
    /// return.
    pub(crate) calls_left: Cell<Option<u64>>,
    /// Ports checked as often as that, to run unchecked once no check is running: a mode that
    /// changed between the sides of a check would give them different callees.
    checked_enough: RefCell<Vec<u32>>,
    /// Whether each check records the calls its function makes, to name the first that
    /// differs when it mismatches.
    pub trace_calls: Cell<bool>,
    /// A function whose checks trace calls at every depth.
    pub trace_deep: Cell<Option<u32>>,
    /// Whether a mismatch reports what differed in its first look, without the second.
    pub first_look_only: Cell<bool>,
    /// Whether a side that departs from the original's interactions prints those around it.
    pub trace_log: Cell<bool>,
    traces: RefCell<Vec<CallTrace>>,
    /// Whether a check is taking its second look at a mismatch, when the ports its port calls
    /// run unchecked: their own checks already ran.
    pub(crate) rechecking: Cell<bool>,
    /// How many argument registers each callee takes, as ports' calls pass them: registers
    /// past them hold leftovers, which differ between the sides without meaning anything.
    arity: RefCell<BTreeMap<u32, usize>>,
    pub mismatches: RefCell<Vec<Mismatch>>,
    pub stats: RefCell<BTreeMap<u32, Stats>>,
}

impl State {
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// The next number of mutations' random sequence.
    fn random(&self) -> u64 {
        let mut x = self.rng.get().max(1);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng.set(x);
        x
    }

    /// The r3 and f1 a call to `addr` returns without running, when a mutated check stands in
    /// for it.
    pub(crate) fn stubbed(&self, addr: u32) -> Option<(u32, f64)> {
        self.stub
            .get()
            .filter(|s| s.0 == addr && self.mutating.get())
            .map(|s| (s.1, s.2))
    }

    /// Whether the interpreter should report the loads it runs (see `note_load`).
    #[inline]
    pub fn watching(&self) -> bool {
        self.watching.get()
    }

    /// The interpreter is about to run instruction `w` at `pc`: if it is a load a target of the
    /// function under check names, note where it reads.
    pub fn note_load(&self, ctx: &Ctx, pc: u32, w: u32) {
        if !self.watch.borrow().contains(&pc) {
            return;
        }
        let (op, ra, rb) = (w >> 26, ((w >> 16) & 31) as usize, ((w >> 11) & 31) as usize);
        let base = if ra == 0 { 0 } else { ctx.regs.r(ra) };
        let ea = match op {
            // lwz, lwzu, lbz, lbzu, lhz, lhzu, lha, lhau
            32..=35 | 40..=43 => base.wrapping_add(w as u16 as i16 as u32),
            // lwzx, lwzux, lbzx, lbzux, lhzx, lhzux, lhax, lhaux
            31 if matches!((w >> 1) & 0x3FF, 23 | 55 | 87 | 119 | 279 | 311 | 343 | 375) => {
                base.wrapping_add(ctx.regs.r(rb))
            }
            _ => return,
        };
        let mut loaded = self.loaded.borrow_mut();
        if loaded.len() < 256 {
            loaded.push((pc, ea));
        }
    }

    /// Starts noting where the loads `addr`'s targets name read.
    fn watch_loads(&self, addr: u32) {
        let targets = self.targets.borrow();
        let pcs: Vec<u32> = targets
            .get(&addr)
            .into_iter()
            .flatten()
            .filter_map(|t| match *t {
                Target::Load { pc, .. } => Some(pc),
                _ => None,
            })
            .collect();
        self.loaded.borrow_mut().clear();
        self.watching.set(!pcs.is_empty());
        *self.watch.borrow_mut() = pcs;
    }

    /// Stops noting loads; returns where each read, as (load, address).
    fn end_watch(&self) -> Vec<(u32, u32)> {
        self.watching.set(false);
        self.watch.borrow_mut().clear();
        std::mem::take(&mut *self.loaded.borrow_mut())
    }

    /// The interpreter runs the instruction at `pc`: outside the game's code, that word is code
    /// as well.
    #[inline]
    pub(crate) fn note_run(&self, pc: u32) {
        let (lo, hi) = self.code.get();
        if hi != 0 && !(lo..hi).contains(&pc) && (RAM_LO..RAM_HI).contains(&pc) {
            let i = ((pc - RAM_LO) / 4) as usize;
            let mut bits = self.ram_code.borrow_mut();
            if bits.is_empty() {
                bits.resize(((RAM_HI - RAM_LO) / 4 / 64) as usize, 0);
            }
            bits[i / 64] |= 1 << (i % 64);
        }
    }

    /// Whether the word at `addr` is code the interpreter ran outside the game's code.
    fn is_ram_code(&self, addr: u32) -> bool {
        let bits = self.ram_code.borrow();
        let i = (addr.wrapping_sub(RAM_LO) / 4) as usize;
        (RAM_LO..RAM_HI).contains(&addr) && !bits.is_empty() && bits[i / 64] >> (i % 64) & 1 != 0
    }

    /// Whether code changed from `before`, the pages a run wrote as they were.
    fn wrote_code(&self, ctx: &Ctx, before: &Pages) -> bool {
        let (lo, hi) = self.code.get();
        before.iter().any(|(&page, old)| {
            let at = 0x8000_0000 + page * ssbm_mem::PAGE_SIZE;
            if at < hi && at + ssbm_mem::PAGE_SIZE > lo {
                return true;
            }
            (0..ssbm_mem::PAGE_SIZE).step_by(4).any(|off| {
                self.is_ram_code(at + off) && {
                    let mut now = [0; 4];
                    let _ = ctx.mem.read_bytes(at + off, &mut now);
                    now[..] != old[off as usize..off as usize + 4]
                }
            })
        })
    }

    /// A call to `addr` starts: returns which check's trace it belongs to, if traced.
    pub(crate) fn call_starts(&self, addr: u32, args: [u32; 4]) -> Option<usize> {
        let mut traces = self.traces.borrow_mut();
        let at = traces.len().checked_sub(1)?;
        let t = &mut traces[at];
        t.depth += 1;
        if t.depth == 1 || t.deep {
            // Deeper calls are told apart by their depth, in the address's low bits.
            let addr = if t.deep { addr | (t.depth - 1).min(3) } else { addr };
            if t.port_side {
                t.port.push((addr, args));
            } else {
                t.original.push((addr, args));
            }
        }
        Some(at)
    }

    /// A port calls `addr` with `n` argument registers.
    pub(crate) fn note_arity(&self, addr: u32, n: usize) {
        self.arity.borrow_mut().insert(addr, n.min(4));
    }

    pub(crate) fn call_ends(&self, trace: Option<usize>) {
        if let Some(at) = trace
            && let Some(t) = self.traces.borrow_mut().get_mut(at)
        {
            t.depth -= 1;
        }
    }

    /// Whether a check of the port of `addr` is running.
    pub fn is_checking(&self, addr: u32) -> bool {
        self.checking.borrow().contains(&addr)
    }

    /// Whether the original side of a check is running.
    pub fn in_original(&self) -> bool {
        matches!(self.phase.get(), Phase::Original | Phase::Replay)
    }

    pub fn clear(&self) {
        self.mismatches.borrow_mut().clear();
        self.stats.borrow_mut().clear();
    }

    /// Whether a check of inputs no real call gave is running: a mutated check or a probe.
    pub fn is_mutating(&self) -> bool {
        self.mutating.get()
    }

    /// How far checks have got through the world's interactions: it grows while the original
    /// makes them and while the port replays them, where video fields do not advance.
    pub fn progress(&self) -> usize {
        match self.phase.get() {
            Phase::Original => self.log.borrow().len(),
            _ => self.cursor.get(),
        }
    }
}

/// Panic payload: a check's side made a different interaction than the log holds.
struct Diverged(String);

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(j) = crate::jump::describe(p) {
        j
    } else if let Some(d) = p.downcast_ref::<Diverged>() {
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
        state.checking.borrow_mut().clear();
    }
    state.checking.borrow_mut().push(addr);
    let traced = state.trace_calls.get();
    // A check of the function `trace_deep` names traces every call it makes at any depth, with
    // the port's callees running unchecked, and prints both sides' lists if they disagree.
    let deep = traced && state.trace_deep.get() == Some(addr);
    if traced {
        state.traces.borrow_mut().push(CallTrace {
            deep,
            ..CallTrace::default()
        });
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
    // The stack below r1 holds nothing the original wrote yet: reads of it before it writes
    // there take whatever an earlier call left.
    let shadowed = outermost && ctx.tracks_uninit();
    if shadowed {
        ctx.begin_stack_shadow(ctx.stack_floor(sp, STACK_SCRATCH), sp);
    }
    let recording = ctx.coverage.begin(addr);
    let executed = ctx.executed();
    // A mutated check's original may loop in ports it calls, where no heartbeat sees it.
    let calls_left = state.calls_left.replace(state.mutating.get().then_some(MUTATED_CALLS));
    let mutating = state.mutating.get();
    if mutating {
        ctx.check_conventions(true);
    }
    // What an outermost check's original reads, its mutated checks may change.
    let log_reads = outermost && !mutating && state.mutations.get() > 0;
    if log_reads {
        ctx.begin_read_log();
        state.watch_loads(addr);
    }
    let original_panic = passing_stop(catch_unwind(AssertUnwindSafe(|| ctx.run_original(addr))));
    let (reads, loaded) = if log_reads {
        (ctx.end_read_log(), state.end_watch())
    } else {
        (Vec::new(), Vec::new())
    };
    let broke_convention = mutating && ctx.check_conventions(false).is_some();
    state.calls_left.set(calls_left);
    let cost = ctx.executed() - executed;
    let hits = ctx.coverage.end(recording);
    if shadowed {
        ctx.end_stack_shadow();
    }
    let original = original_panic.as_ref().map(|p| panic_text(p.as_ref()));
    // Where the original asked its caller to continue, if not after the call.
    let original_resume = ctx.take_resume_at();
    let j1 = ctx.mem.end_journal();
    let wrote_code = state.mutating.get() && state.wrote_code(ctx, &j1);
    if wrote_code
        || broke_convention
        || original_panic.as_ref().is_some_and(|p| p.is::<Runaway>())
        || (outermost && mutating && original_panic.is_some())
    {
        // A mutated call that runs on and on may never return on the port's side. One that
        // writes over code then runs what ports never read, and one that writes over a frame's
        // saved registers or return address then returns with what ports never saved: drop
        // it. So with one whose original fails, through a bad pointer say: no call the game
        // makes has its inputs, and where the port fails on them instead depends on the order
        // of its loads, which ports needn't keep.
        ctx.mem.restore(&j1);
        ctx.regs.restore(&regs0);
        return drop_check(ctx, traced, enclosing, outermost);
    }
    let regs1 = ctx.regs.snapshot();
    let s1 = ctx.mem.capture(j1.keys());
    let end = if outermost {
        state.log.borrow().len()
    } else {
        state.cursor.get()
    };

    ctx.mem.restore(&j1);
    ctx.regs.restore(&regs0);
    // The original's caller is the interpreter's return sentinel: the port's is too, for code
    // that reads the link register as a value.
    ctx.regs.lr.set(crate::RETURN_SENTINEL);
    state.phase.set(Phase::Port);
    state.cursor.set(start);
    if traced && let Some(t) = state.traces.borrow_mut().last_mut() {
        t.port_side = true;
        t.depth = 0;
    }
    // The port runs on the stack the original found, whose frames it lays out as the original
    // does: what either reads of it before writing it is the same.
    ctx.mem.begin_journal();
    let rechecking = state.rechecking.replace(state.rechecking.get() || deep);
    let calls_left = state.calls_left.replace(state.mutating.get().then_some(MUTATED_CALLS));
    let port_failure = passing_stop(catch_unwind(AssertUnwindSafe(|| ctx.run_native(addr, native))));
    let port_dropped = port_failure.as_ref().is_some_and(|p| p.is::<Dropped>());
    let mut port = port_failure.map(|p| {
            if let Some(j) = crate::jump::describe(p.as_ref()) {
                return j;
            }
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
    state.rechecking.set(rechecking);
    state.calls_left.set(calls_left);
    let port_resume = ctx.take_resume_at();
    let j2 = ctx.mem.end_journal();
    if port_dropped {
        // A check of a port this one's port called had to be dropped: the call can't be
        // compared without it.
        ctx.mem.restore(&j2);
        ctx.regs.restore(&regs0);
        return drop_check(ctx, traced, enclosing, outermost);
    }
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
        ctx,
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

    // Look again, with both sides on a cleared stack whose frames start zeroed: if the original
    // cannot follow its own log then, or both sides agree, it reads stack it never wrote, whose
    // leftovers the port cannot reproduce. Otherwise they differ given the same stack, and the
    // differences are those of this second look.
    let mut uninitialized = false;
    let mut dropped = false;
    // An original that faults through a bad pointer, as mutated inputs make one, may also have
    // read stack it never wrote before: its second look faults likewise, if it agrees.
    let faults = |p: &Option<Box<dyn std::any::Any + Send>>| {
        p.as_ref().is_some_and(|p| panic_text(p.as_ref()).starts_with("unmapped "))
    };
    if !diffs.is_empty()
        && (original.is_none() || faults(&original_panic))
        && !state.first_look_only.get()
    {
        let zero = ctx.zero_frames.replace(true);
        let rechecking = state.rechecking.replace(true);
        ctx.mem.restore(&j2);
        ctx.regs.restore(&regs0);
        state.phase.set(Phase::Replay);
        state.cursor.set(start);
        // Each side's calls are now those of its second run.
        if traced && let Some(t) = state.traces.borrow_mut().last_mut() {
            t.port_side = false;
            t.depth = 0;
            t.original.clear();
        }
        ctx.mem.begin_journal();
        clear_stack(ctx, sp);
        let again = passing_stop(catch_unwind(AssertUnwindSafe(|| ctx.run_original(addr))));
        dropped |= again
            .as_ref()
            .is_some_and(|p| p.is::<Runaway>());
        let _ = ctx.take_resume_at();
        let j3 = ctx.mem.end_journal();
        let regs3 = ctx.regs.snapshot();
        if (again.is_none() || faults(&again)) && state.cursor.get() == end {
            let s3 = ctx.mem.capture(j3.keys());
            ctx.mem.restore(&j3);
            ctx.regs.restore(&regs0);
            ctx.regs.lr.set(crate::RETURN_SENTINEL);
            state.phase.set(Phase::Port);
            state.cursor.set(start);
            if traced && let Some(t) = state.traces.borrow_mut().last_mut() {
                t.port_side = true;
                t.depth = 0;
                t.port.clear();
            }
            ctx.mem.begin_journal();
            clear_stack(ctx, sp);
            let port_again =
                passing_stop(catch_unwind(AssertUnwindSafe(|| ctx.run_native(addr, native))));
            dropped |= port_again.as_ref().is_some_and(|p| p.is::<Dropped>());
            ctx.truncate_natives(natives);
            let _ = ctx.take_resume_at();
            let j4 = ctx.mem.end_journal();
            let regs4 = ctx.regs.snapshot();
            if (port_again.is_none() && state.cursor.get() == end) || faults(&port_again) {
                let s4 = ctx.mem.capture(j4.keys());
                let original = Outcome {
                    before: &j3,
                    after: &s3,
                    regs: &regs3,
                    panic: again.as_ref().map(|p| panic_text(p.as_ref())),
                };
                let port = Outcome {
                    before: &j4,
                    after: &s4,
                    regs: &regs4,
                    panic: port_again.as_ref().map(|p| panic_text(p.as_ref())),
                };
                diffs = compare(ctx, &original, &port, returns, sp);
                uninitialized = diffs.is_empty();
            }
            ctx.mem.restore(&j4);
        } else {
            uninitialized = true;
            ctx.mem.restore(&j3);
        }
        state.rechecking.set(rechecking);
        ctx.zero_frames.set(zero);
    }

    if traced && let Some(t) = state.traces.borrow_mut().pop() {
        if t.deep && !diffs.is_empty() {
            let show = |c: Option<&(u32, [u32; 4])>| {
                c.map(|&(a, x)| {
                    format!("{}{} {:08X?}", "  ".repeat((a & 3) as usize), ctx.name_of(a & !3), x)
                })
                .unwrap_or_default()
            };
            eprintln!("{} call trace, original | port:", ctx.name_of(addr));
            for i in 0..t.original.len().max(t.port.len()) {
                eprintln!("  {i:4} {:70} | {}", show(t.original.get(i)), show(t.port.get(i)));
            }
        }
        if !diffs.is_empty() {
            let n = t.original.len().max(t.port.len());
            let arity = state.arity.borrow();
            // Locals lie at different stack addresses on the two sides: pointers to them match.
            let stack = ctx.stack_floor(sp, STACK_SCRATCH)..sp;
            let same = |a: Option<&(u32, [u32; 4])>, b: Option<&(u32, [u32; 4])>| match (a, b) {
                (Some(&(fa, xa)), Some(&(fb, xb))) => {
                    let k = arity.get(&fa).copied().unwrap_or(4);
                    fa == fb
                        && (0..k).all(|i| {
                            xa[i] == xb[i] || (stack.contains(&xa[i]) && stack.contains(&xb[i]))
                        })
                }
                (a, b) => a == b,
            };
            if let Some(index) = (0..n).find(|&i| !same(t.original.get(i), t.port.get(i))) {
                let describe = |c: Option<&(u32, [u32; 4])>| c.map(|&(a, args)| (ctx.name_of(a), args));
                diffs.push(Diff::Call {
                    index,
                    original: describe(t.original.get(index)),
                    port: describe(t.port.get(index)),
                });
            }
        }
    }
    if original_resume != port_resume {
        diffs.push(Diff::Reg {
            name: "resume at",
            original: original_resume.map_or(0, u64::from),
            port: port_resume.map_or(0, u64::from),
        });
    }

    if dropped {
        ctx.mem.restore(&j2);
        ctx.regs.restore(&regs0);
        return drop_check(ctx, traced, enclosing, outermost);
    }
    // Continue from the original's results.
    ctx.mem.restore(&j2);
    ctx.mem.restore(&s1);
    ctx.regs.restore(&regs1);
    if let Some(at) = original_resume {
        ctx.resume_at(at);
    }
    state.cursor.set(end);
    state.phase.set(enclosing);
    state.checking.borrow_mut().pop();
    if outermost {
        state.active.set(false);
        CHECKING.with(|c| c.set(false));
    }

    // A nested original that cannot follow the enclosing log, or faults, was called with
    // different inputs: the enclosing port is at fault, and its own check reports it.
    if !outermost && let Some(p) = original_panic {
        resume_unwind(p);
    }
    if diffs.is_empty() {
        ctx.coverage.verified(hits);
    }
    let mut stats = state.stats.borrow_mut();
    let entry = stats.entry(addr).or_default();
    entry.calls += 1;
    let spent = entry.cost;
    entry.cost += cost;
    if state.calls_per_function.get().is_some_and(|n| entry.calls == n)
        || state
            .budget_per_function
            .get()
            .is_some_and(|b| spent < b && entry.cost >= b)
    {
        state.checked_enough.borrow_mut().push(addr);
    }
    if outermost {
        for addr in state.checked_enough.borrow_mut().drain(..) {
            ctx.set_mode(addr, crate::Mode::Native);
        }
    }
    if uninitialized {
        entry.uninitialized += 1;
    } else if !diffs.is_empty() {
        let mutated = state.mutating.get();
        let n = if mutated {
            &mut entry.mutated_mismatches
        } else {
            &mut entry.mismatches
        };
        *n += 1;
        if *n <= state.keep_per_function.get().max(1) as u64 {
            state.mismatches.borrow_mut().push(Mismatch {
                function: addr,
                call: entry.calls,
                mutated,
                diffs,
            });
        }
    }
    drop(stats);
    if outermost && original_panic.is_none() && !state.mutating.get() && state.mutations.get() > 0 {
        let inputs = Inputs {
            reads: &reads,
            loaded: &loaded,
        };
        mutated_checks(ctx, addr, native, returns, &j1, &regs0, &regs1, original_resume, &inputs);
    }
    // The original's panic is the run's to handle, such as the end of a run.
    if let Some(p) = original_panic {
        resume_unwind(p);
    }
}

/// Panic payload that ends the check enclosing a mutated one that had to be dropped.
struct Dropped;

/// Ends a check that can't be compared, its memory and registers already as it found them.
/// Nested in another check, it ends that one too.
fn drop_check(ctx: &Ctx, traced: bool, enclosing: Phase, outermost: bool) {
    let state = &ctx.lockstep;
    if traced {
        state.traces.borrow_mut().pop();
    }
    state.phase.set(enclosing);
    state.checking.borrow_mut().pop();
    if !outermost {
        panic_any(Dropped);
    }
    state.active.set(false);
    CHECKING.with(|c| c.set(false));
}

/// Panic payload that ends a mutated check whose original runs on and on, from the heartbeat or
/// the limit on its calls.
pub struct Runaway;

/// Checks the port of `addr` from a call no code made, whose arguments `setup` puts in place,
/// on the state the game is in: as a mutated check, counted apart and undone after. Only outside
/// any check; returns whether it ran.
pub fn probe(ctx: &Ctx, addr: u32, setup: impl FnOnce(&Ctx)) -> bool {
    let state = &ctx.lockstep;
    let Some(e) = ctx.entry(addr).filter(|e| !e.external) else {
        return false;
    };
    // A function that never returns, such as HSD_Panic, has nothing a probe could compare.
    if state.active.get() || state.mutating.get() || state.noreturn.borrow().contains(&addr) {
        return false;
    }
    let regs = ctx.regs.snapshot();
    let resume = ctx.take_resume_at();
    state.mutating.set(true);
    ctx.mem.begin_journal();
    setup(ctx);
    let result = catch_unwind(AssertUnwindSafe(|| run(ctx, addr, e.native, e.returns)));
    let _ = ctx.take_resume_at();
    let undo = ctx.mem.end_journal();
    ctx.mem.restore(&undo);
    ctx.regs.restore(&regs);
    if let Some(at) = resume {
        ctx.resume_at(at);
    }
    state.mutating.set(false);
    if let Err(p) = result
        && p.is::<crate::Stop>()
    {
        resume_unwind(p);
    }
    true
}

/// Calls a port under a mutated check may make: a runaway loop makes many more.
const MUTATED_CALLS: u64 = 10_000_000;

/// Interactions with the hardware and SDK layer that answer nothing a mutated check may make.
const NULL_LOG_MAX: usize = 1 << 16;

/// Main memory.
const RAM_LO: u32 = 0x8000_0000;
const RAM_HI: u32 = 0x8180_0000;

/// Checks the function again from the call just checked, as many times as `mutations` says,
/// each with the call's arguments and what they point to changed at random; then goes on
/// from the call's results.
#[allow(clippy::too_many_arguments)]
fn mutated_checks(
    ctx: &Ctx,
    addr: u32,
    native: Native,
    returns: Returns,
    before: &Pages,
    regs0: &crate::RegsSnapshot,
    regs1: &crate::RegsSnapshot,
    resume: Option<u32>,
    inputs: &Inputs,
) {
    let state = &ctx.lockstep;
    state.mutating.set(true);
    // The pages the call wrote, as it left them; `before` has them as it found them.
    let after = ctx.mem.capture(before.keys());
    let _ = ctx.take_resume_at();
    let mut stop = None;
    for _ in 0..state.mutations.get() {
        ctx.mem.begin_journal();
        ctx.mem.restore(before);
        ctx.regs.restore(regs0);
        let kept = state.mismatches.borrow().len();
        let mut changes = mutate(ctx, inputs.reads);
        changes.extend(change_target(ctx, addr, inputs.loaded));
        let result = catch_unwind(AssertUnwindSafe(|| run(ctx, addr, native, returns)));
        state.stub.set(None);
        // Name what changed for the mismatches the report will show.
        for m in &state.mismatches.borrow()[kept..] {
            eprintln!(
                "  {} call {} had its inputs changed: {}",
                ctx.name_of(m.function),
                m.call,
                changes.join(", ")
            );
        }
        let _ = ctx.take_resume_at();
        let undo = ctx.mem.end_journal();
        ctx.mem.restore(&undo);
        ctx.mem.restore(&after);
        if let Err(p) = result
            && p.is::<crate::Stop>()
        {
            stop = Some(p);
            break;
        }
    }
    ctx.regs.restore(regs1);
    if let Some(at) = resume {
        ctx.resume_at(at);
    }
    state.mutating.set(false);
    if let Some(p) = stop {
        resume_unwind(p);
    }
}

/// What the original of an outermost check read that its mutated checks may change: every
/// word, and where the loads its function's targets name read.
struct Inputs<'a> {
    reads: &'a [u32],
    loaded: &'a [(u32, u32)],
}

/// Half the time, changes one of the targets of the function at `addr`: has a callee return a
/// small number, as flags, counts, kinds and null pointers are, or sets an argument or a word
/// a load read (`loaded`) to the value its compare looks for or a neighbor of it, or toggles
/// the bits its test looks at. Returns what it changed.
fn change_target(ctx: &Ctx, addr: u32, loaded: &[(u32, u32)]) -> Option<String> {
    let state = &ctx.lockstep;
    let targets = state.targets.borrow();
    let all = targets.get(&addr).filter(|t| !t.is_empty())?;
    if state.random().is_multiple_of(2) {
        return None;
    }
    // A value near the one a compare looks for, or the bits a test looks at toggled.
    let near = |old: u32, value: u32, bits: bool| {
        if bits {
            old ^ value
        } else {
            match state.random() % 3 {
                0 => value.wrapping_sub(1),
                1 => value.wrapping_add(1),
                _ => value,
            }
        }
    };
    match all[(state.random() % all.len() as u64) as usize] {
        Target::Call(callee) => {
            let r3 = match state.random() % 6 {
                0 => 0,
                1 => 1,
                2 => u32::MAX,
                3 => 2,
                4 => (state.random() % 16) as u32,
                _ => (state.random() % 256) as u32,
            };
            let f1 = match state.random() % 5 {
                0 => 0.0,
                1 => 1.0,
                2 => -1.0,
                3 => 0.5,
                _ => (state.random() % 200) as f64 - 100.0,
            };
            state.stub.set(Some((callee, r3, f1)));
            Some(format!("{} returns r3 {r3:#X}, f1 {f1}", ctx.name_of(callee)))
        }
        Target::Reg { reg, value, bits } => {
            let old = ctx.regs.r(reg);
            let new = near(old, value, bits);
            ctx.regs.set_r(reg, new);
            Some(format!("r{reg} {old:#X}->{new:#X}"))
        }
        Target::Load { pc, size, value, bits } => {
            let at: Vec<u32> = loaded.iter().filter(|l| l.0 == pc).map(|l| l.1).collect();
            let ea = *at.get((state.random() % at.len().max(1) as u64) as usize)?;
            let (code_start, code_end) = state.code.get();
            if !(RAM_LO..RAM_HI).contains(&ea)
                || (code_start..code_end).contains(&ea)
                || state.is_ram_code(ea & !3)
                || state.constant.borrow().iter().any(|&(lo, hi)| (lo..hi).contains(&ea))
            {
                return None;
            }
            let old = match size {
                1 => u32::from(ctx.read_u8(ea)),
                2 => u32::from(ctx.read_u16(ea)),
                _ => ctx.read_u32(ea),
            };
            let new = near(old, value, bits);
            match size {
                1 => ctx.write_u8(ea, new as u8),
                2 => ctx.write_u16(ea, new as u16),
                _ => ctx.write_u32(ea, new),
            }
            Some(format!("{ea:#010X} {old:#X}->{new:#X}"))
        }
    }
}

/// Changes a call's inputs at random: some of the bytes its pointer arguments reach, some of
/// what the original read (`reads`, words), such as global state it branches on, and some of
/// its other integer and float arguments. Code, read-only data and words that hold pointers
/// stay as they are: only the original runs code from memory or reads the constants ports
/// hold inline, a pointer changed sends both sides through memory where they differ only in
/// what ports keep out of it, and counts, flags and floats are what reach the branches real
/// calls miss.
fn mutate(ctx: &Ctx, reads: &[u32]) -> Vec<String> {
    let state = &ctx.lockstep;
    let next = || state.random();
    let ram = |a: u32| (RAM_LO..RAM_HI).contains(&a);
    let (code_start, code_end) = state.code.get();
    let constant = state.constant.borrow();
    let changes = std::cell::RefCell::new(Vec::new());
    let change_byte = |at: u32| {
        if ram(at)
            && !(code_start..code_end).contains(&at)
            && !state.is_ram_code(at & !3)
            && !constant.iter().any(|&(lo, hi)| (lo..hi).contains(&at))
            && !ram(ctx.read_u32(at & !3))
        {
            let old = ctx.read_u8(at);
            let new = if next() % 2 == 0 {
                old ^ (1 << (next() % 8))
            } else {
                next() as u8
            };
            ctx.write_u8(at, new);
            // Nor does a word become a pointer to hardware registers: both sides would then
            // read and write registers through it, in whichever order each reads the fields.
            if (0xCC00_0000..0xCC01_0000).contains(&ctx.read_u32(at & !3)) {
                ctx.write_u8(at, old);
                return;
            }
            changes.borrow_mut().push(format!("{at:#010X} {old:02X}->{new:02X}"));
        }
    };
    for r in 3..=10 {
        let v = ctx.regs.r(r);
        if ram(v) {
            for _ in 0..=next() % 3 {
                change_byte(v.wrapping_add((next() % 0x80) as u32));
            }
        } else if (0xCC00_0000..=0xCC01_0000).contains(&v) {
            // A pointer to the hardware registers, such as the write-gather pipe's base that
            // MWCC keeps 0x8000 past it, stays as it is.
        } else if next() % 3 == 0 {
            let new = match next() % 6 {
                0 => 0,
                1 => 1,
                2 => v.wrapping_add(1),
                3 => v.wrapping_sub(1),
                4 => v ^ (1 << (next() % 8)),
                _ => (next() % 64) as u32,
            };
            // Extended as the value was, as a caller passes a narrow argument: a byte stays a
            // byte, whose upper bits the original may rely on and the port's adapter drops.
            let new = match v {
                0..=0xFF => new & 0xFF,
                0x100..=0xFFFF => new & 0xFFFF,
                0xFFFF_FF80.. => new as u8 as i8 as u32,
                0xFFFF_8000.. => new as u16 as i16 as u32,
                _ => new,
            };
            ctx.regs.set_r(r, new);
            changes.borrow_mut().push(format!("r{r} {v:#X}->{new:#X}"));
        }
    }
    if !reads.is_empty() {
        for _ in 0..=next() % 3 {
            let word = reads[(next() % reads.len() as u64) as usize];
            change_byte(word + (next() % 4) as u32);
        }
    }
    for f in 1..=8 {
        if next() % 3 == 0 {
            let v = ctx.regs.f(f);
            let new = match next() % 5 {
                0 => 0.0,
                1 => 1.0,
                2 => -v,
                3 => v * 2.0,
                _ => v * 0.5,
            };
            ctx.regs.set_f(f, new);
            changes.borrow_mut().push(format!("f{f} {v}->{new}"));
        }
    }
    changes.into_inner()
}

fn clear_stack(ctx: &Ctx, sp: u32) {
    let start = ctx.stack_floor(sp, STACK_CLEARED);
    let _ = ctx.mem.write_bytes(start, &vec![0; (sp - start) as usize]);
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

/// A side's failure, if it failed; the run ending on purpose ends it here too.
fn passing_stop(result: std::thread::Result<()>) -> Option<Box<dyn std::any::Any + Send>> {
    match result {
        Ok(()) => None,
        Err(p) if p.is::<crate::Stop>() => resume_unwind(p),
        Err(p) => Some(p),
    }
}

/// Runs `f` as one interaction: logged for the original, replayed for the port. `f` returns
/// the value a register read produced.
fn interact(ctx: &Ctx, kind: Kind, f: impl FnOnce() -> u32) -> u32 {
    let state = &ctx.lockstep;
    match state.phase.get() {
        Phase::Original if state.depth.get() == 0 => {
            let sp = ctx.regs.r(1);
            let lr = ctx.regs.lr.get();
            let (value, writes) = if state.mutating.get() {
                // A mutated check's calls must change nothing outside memory, which its
                // journals undo: its original meets hardware that reads as 0 and takes no
                // writes, stand-ins that return 0 without running, and no interrupts. The port
                // then replays the same. A hook stands in for a wait, which would never end,
                // and so would a loop polling hardware for what it never answers.
                if matches!(kind, Kind::Hook(_)) || state.log.borrow().len() >= NULL_LOG_MAX {
                    panic_any(Runaway);
                }
                if let Kind::Call(_) = kind {
                    ctx.regs.set_r(3, 0);
                    ctx.regs.set_r(4, 0);
                    ctx.regs.set_f(1, 0.0);
                }
                (0, Vec::new())
            } else {
                state.depth.set(1);
                ctx.mem.begin_log();
                let result = catch_unwind(AssertUnwindSafe(f));
                let writes = ctx.mem.end_log();
                state.depth.set(0);
                match result {
                    Ok(v) => (v, writes),
                    Err(p) => resume_unwind(p),
                }
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
                sp,
                lr,
            });
            value
        }
        Phase::Port | Phase::Replay => {
            let at = state.cursor.get();
            let log = state.log.borrow();
            match log.get(at) {
                Some(x) if x.kind == kind => {
                    state.cursor.set(at + 1);
                    // Frames below the original's r1 are dead once the callee or handler
                    // returns, and the side replaying may be using that stack. So is the word
                    // at 4(r1), where the callee saved its return address.
                    let dead = ctx.stack_floor(x.sp, STACK_SCRATCH)..x.sp.wrapping_add(8);
                    for (at, bytes) in &x.writes {
                        if dead.contains(at) {
                            continue;
                        }
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
                    if state.trace_log.get() {
                        // The interactions around it, with where the original made them.
                        for (i, x) in log.iter().enumerate().take(at + 4).skip(at.saturating_sub(12)) {
                            let mark = if i == at { ">" } else { " " };
                            eprintln!(
                                "  {mark} {i}: {} (LR {})",
                                describe(ctx, x.kind),
                                ctx.name_of(x.lr)
                            );
                        }
                    }
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
        let original = ctx.running_original.replace(false);
        native(ctx);
        ctx.running_original.set(original);
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
fn compare(ctx: &Ctx, a: &Outcome, b: &Outcome, returns: Returns, sp: u32) -> Vec<Diff> {
    let mut diffs = Vec::new();
    // The same panic on both sides, such as the same longjmp, is the same behavior; the ports a
    // port's names as running then are only where it happened.
    // A fault through a bad pointer ends both alike, whichever access of the same object each
    // side made first.
    let cause = |p: &Option<String>| {
        p.as_ref().map(|t| {
            let t = t.split(" (in ").next().unwrap_or(t);
            if t.starts_with("unmapped ") { "memory fault" } else { t }.to_owned()
        })
    };
    if cause(&a.panic) != cause(&b.panic) {
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
        Returns::Int8 => regs.push(("r3", r3.1 & 0xFF, r3.2 & 0xFF)),
        Returns::Int16 => regs.push(("r3", r3.1 & 0xFFFF, r3.2 & 0xFFFF)),
        Returns::Int64 => regs.extend([r3, r4]),
        Returns::Float => regs.push(f1),
    }
    // Where a side stopped, such as at a longjmp, its registers are not results.
    if a.panic.is_some() || b.panic.is_some() {
        regs.clear();
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
    // callee writes into the caller's frame, are linkage rather than results. So are jump
    // buffers: a port's `setjmp` saves no registers there, and only `longjmp` reads them.
    let sp = sp & 0x3FFF_FFFF;
    let scratch = ctx.stack_floor(sp, STACK_SCRATCH)..sp + 8;
    let jump_buffers: Vec<std::ops::Range<u32>> = ctx
        .jump_buffers
        .borrow()
        .iter()
        .map(|&env| (env & 0x3FFF_FFFF)..(env & 0x3FFF_FFFF) + crate::jump::JMP_BUF_SIZE)
        .collect();
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
            if expected[i] == actual[i]
                || scratch.contains(&phys)
                || jump_buffers.iter().any(|r| r.contains(&phys))
            {
                i += 1;
                continue;
            }
            let start = i;
            while i < PAGE_SIZE as usize && expected[i] != actual[i] {
                i += 1;
            }
            // Whole words, so a value that differs reads as one, such as a float's sign.
            let (lo, hi) = (start & !3, ((i + 3) & !3).min(PAGE_SIZE as usize));
            diffs.push(Diff::Mem {
                addr: 0x8000_0000 | (page * PAGE_SIZE + lo as u32),
                original: expected[lo..hi].to_vec(),
                port: actual[lo..hi].to_vec(),
            });
            i = hi;
        }
    }
    diffs
}
