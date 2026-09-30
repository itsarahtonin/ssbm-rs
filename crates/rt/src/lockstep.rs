// SPDX-License-Identifier: GPL-3.0-or-later

//! Function-level lockstep: run the original and the port from the same state, compare the
//! results, then continue with the original's results so one bug cannot cascade.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{Ctx, Native, PAGE_SIZE, Pages};

/// Stack below the caller's r1 holds frames and scratch that ports need not reproduce.
pub const STACK_SCRATCH: u32 = 0x1_0000;

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

#[derive(Default)]
pub struct State {
    active: Cell<bool>,
    /// Mismatches kept per function; later ones only count in `stats`.
    pub keep_per_function: Cell<usize>,
    pub mismatches: RefCell<Vec<Mismatch>>,
    pub stats: RefCell<BTreeMap<u32, Stats>>,
}

impl State {
    pub fn is_active(&self) -> bool {
        self.active.get()
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

pub(crate) fn run(ctx: &Ctx, addr: u32, native: Native) {
    let state = &ctx.lockstep;
    state.active.set(true);
    let regs0 = ctx.regs.snapshot();
    let sp = regs0.gpr[1];

    ctx.mem.begin_journal();
    let original = catch_unwind(AssertUnwindSafe(|| ctx.run_original(addr)))
        .err()
        .map(panic_text);
    let j1 = ctx.mem.end_journal();
    let regs1 = ctx.regs.snapshot();
    let s1 = ctx.mem.capture(j1.keys());

    ctx.mem.restore(&j1);
    ctx.regs.restore(&regs0);
    ctx.mem.begin_journal();
    let port = catch_unwind(AssertUnwindSafe(|| native(ctx)))
        .err()
        .map(panic_text);
    let j2 = ctx.mem.end_journal();
    let regs2 = ctx.regs.snapshot();

    let mut diffs = Vec::new();
    if original.is_some() || port.is_some() {
        diffs.push(Diff::Panic {
            original: original.clone(),
            port,
        });
    }
    let regs = [
        ("r1", u64::from(regs1.gpr[1]), u64::from(regs2.gpr[1])),
        ("r3", u64::from(regs1.gpr[3]), u64::from(regs2.gpr[3])),
        ("r4", u64::from(regs1.gpr[4]), u64::from(regs2.gpr[4])),
        ("f1", regs1.fpr[1].ps0.to_bits(), regs2.fpr[1].ps0.to_bits()),
    ];
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

fn compare_pages(ctx: &Ctx, j1: &Pages, s1: &Pages, j2: &Pages, sp: u32, diffs: &mut Vec<Diff>) {
    let scratch = (sp & 0x3FFF_FFFF).saturating_sub(STACK_SCRATCH)..(sp & 0x3FFF_FFFF);
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
