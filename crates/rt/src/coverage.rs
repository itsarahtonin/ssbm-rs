// SPDX-License-Identifier: GPL-3.0-or-later

//! Which instructions of the original lockstep has verified: an instruction of a function
//! counts once the original ran it within a check of that function whose sides agreed.

use std::cell::{Cell, RefCell};

/// The code tracked: main memory's first 4 MiB, which hold the DOL's text.
pub const LO: u32 = 0x8000_0000;
pub const HI: u32 = 0x8040_0000;
const WORDS: usize = ((HI - LO) / 4 / 64) as usize;

/// Where a function starts and ends, as `[start, end)`, given its start.
pub type Bounds = Box<dyn Fn(u32) -> Option<(u32, u32)>>;

#[derive(Default)]
pub struct Coverage {
    /// Whether a check's original is running, and what it runs of the checked function counts.
    on: Cell<bool>,
    /// The checked function, as `[start, end)`.
    range: Cell<(u32, u32)>,
    /// What the running check's original has run of it, a bit per instruction.
    hits: RefCell<Vec<u64>>,
    /// The instructions verified so far, a bit per instruction from `LO`.
    covered: RefCell<Vec<u64>>,
    bounds: RefCell<Option<Bounds>>,
}

/// What a check's original ran of the checked function: its start and a bit per instruction.
pub(crate) struct Hits(u32, Vec<u64>);

/// The recording a check's original run replaced, restored when it ends.
pub(crate) struct Saved(bool, (u32, u32), Vec<u64>);

impl Coverage {
    pub fn set_bounds(&self, bounds: Bounds) {
        *self.bounds.borrow_mut() = Some(bounds);
    }

    /// Whether the interpreter should report each instruction it runs.
    #[inline]
    pub fn recording(&self) -> bool {
        self.on.get()
    }

    /// The original ran the instruction at `pc`.
    #[inline]
    pub fn hit(&self, pc: u32) {
        let (lo, hi) = self.range.get();
        if pc >= lo && pc < hi {
            let i = ((pc - lo) / 4) as usize;
            self.hits.borrow_mut()[i / 64] |= 1 << (i % 64);
        }
    }

    /// A check of the function at `addr` starts running its original.
    pub(crate) fn begin(&self, addr: u32) -> Saved {
        let range = self
            .bounds
            .borrow()
            .as_ref()
            .and_then(|b| b(addr))
            .filter(|&(lo, hi)| lo >= LO && hi <= HI && lo < hi)
            .unwrap_or((0, 0));
        let words = ((range.1 - range.0) / 4).div_ceil(64) as usize;
        Saved(
            self.on.replace(range.1 > range.0),
            self.range.replace(range),
            self.hits.replace(vec![0; words]),
        )
    }

    /// The check's original has run: what it ran.
    pub(crate) fn end(&self, saved: Saved) -> Hits {
        let lo = self.range.get().0;
        self.on.set(saved.0);
        self.range.set(saved.1);
        Hits(lo, self.hits.replace(saved.2))
    }

    /// The check's sides agreed: what its original ran is verified.
    pub(crate) fn verified(&self, hits: Hits) {
        let Hits(lo, bits) = hits;
        if bits.is_empty() {
            return;
        }
        let mut covered = self.covered.borrow_mut();
        if covered.is_empty() {
            covered.resize(WORDS, 0);
        }
        for (i, &w) in bits.iter().enumerate() {
            for b in 0..64 {
                if w & (1 << b) != 0 {
                    let at = ((lo - LO) / 4) as usize + i * 64 + b;
                    covered[at / 64] |= 1 << (at % 64);
                }
            }
        }
    }

    /// How many of the instructions in `hits` no check has verified yet.
    pub(crate) fn novel(&self, hits: &Hits) -> u32 {
        let Hits(lo, bits) = hits;
        let covered = self.covered.borrow();
        let mut n = 0;
        for (i, &w) in bits.iter().enumerate() {
            for b in 0..64 {
                if w & (1 << b) != 0 {
                    let at = ((lo - LO) / 4) as usize + i * 64 + b;
                    n += u32::from(covered.get(at / 64).is_none_or(|c| c >> (at % 64) & 1 == 0));
                }
            }
        }
        n
    }

    /// Whether the instruction at `pc` is verified.
    pub fn is_covered(&self, pc: u32) -> bool {
        let covered = self.covered.borrow();
        let at = (pc.wrapping_sub(LO) / 4) as usize;
        covered.get(at / 64).is_some_and(|w| w >> (at % 64) & 1 != 0)
    }

    /// The instructions verified so far, a bit per instruction from `LO`, as little-endian
    /// words: empty when none are.
    pub fn covered(&self) -> Vec<u64> {
        self.covered.borrow().clone()
    }
}
