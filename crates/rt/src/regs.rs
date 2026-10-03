// SPDX-License-Identifier: GPL-3.0-or-later

//! The PowerPC register file. Ported code only touches it at call boundaries; the dev
//! interpreter uses all of it.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use gekko_fp::Ps;

/// SPR numbers the runtime names.
pub mod spr {
    pub const XER: u32 = 1;
    pub const LR: u32 = 8;
    pub const CTR: u32 = 9;
    pub const DSISR: u32 = 18;
    pub const DAR: u32 = 19;
    pub const DEC: u32 = 22;
    pub const SRR0: u32 = 26;
    pub const SRR1: u32 = 27;
    pub const SPRG0: u32 = 272;
    pub const TBL_R: u32 = 268;
    pub const TBU_R: u32 = 269;
    pub const TBL_W: u32 = 284;
    pub const TBU_W: u32 = 285;
    pub const GQR0: u32 = 912;
    pub const HID2: u32 = 920;
    pub const DMA_U: u32 = 922;
    pub const DMA_L: u32 = 923;
}

#[derive(Default)]
pub struct Regs {
    pub gpr: [Cell<u32>; 32],
    pub fpr: [Cell<Ps>; 32],
    pub cr: Cell<u32>,
    pub lr: Cell<u32>,
    pub ctr: Cell<u32>,
    pub xer: Cell<u32>,
    pub fpscr: Cell<u32>,
    pub msr: Cell<u32>,
    pub gqr: [Cell<u32>; 8],
    /// The time base, which counts at a quarter of the bus clock (40.5 MHz).
    pub tb: Cell<u64>,
    /// Every other SPR by number, such as HID0, SRR0 or the DMA registers.
    pub spr: RefCell<BTreeMap<u32, u32>>,
}

/// A copy of the whole register file.
#[derive(Clone, Debug, PartialEq)]
pub struct RegsSnapshot {
    pub gpr: [u32; 32],
    pub fpr: [Ps; 32],
    pub cr: u32,
    pub lr: u32,
    pub ctr: u32,
    pub xer: u32,
    pub fpscr: u32,
    pub msr: u32,
    pub gqr: [u32; 8],
    pub tb: u64,
    pub spr: BTreeMap<u32, u32>,
}

impl Regs {
    #[inline]
    pub fn r(&self, i: usize) -> u32 {
        self.gpr[i].get()
    }

    #[inline]
    pub fn set_r(&self, i: usize, v: u32) {
        self.gpr[i].set(v);
    }

    /// ps0 of an FPR, which is what scalar code sees.
    #[inline]
    pub fn f(&self, i: usize) -> f64 {
        self.fpr[i].get().ps0
    }

    /// Sets ps0, leaving ps1, as double-precision results do.
    #[inline]
    pub fn set_f(&self, i: usize, v: f64) {
        let mut ps = self.fpr[i].get();
        ps.ps0 = v;
        self.fpr[i].set(ps);
    }

    pub fn get_spr(&self, n: u32) -> u32 {
        match n {
            spr::XER => self.xer.get(),
            spr::LR => self.lr.get(),
            spr::CTR => self.ctr.get(),
            912..=919 => self.gqr[(n - spr::GQR0) as usize].get(),
            spr::TBL_R => self.tb.get() as u32,
            spr::TBU_R => (self.tb.get() >> 32) as u32,
            _ => self.spr.borrow().get(&n).copied().unwrap_or(0),
        }
    }

    pub fn set_spr(&self, n: u32, v: u32) {
        match n {
            spr::XER => self.xer.set(v),
            spr::LR => self.lr.set(v),
            spr::CTR => self.ctr.set(v),
            912..=919 => self.gqr[(n - spr::GQR0) as usize].set(v),
            spr::TBL_W => self.tb.set((self.tb.get() & !0xFFFF_FFFF) | u64::from(v)),
            spr::TBU_W => self
                .tb
                .set((self.tb.get() & 0xFFFF_FFFF) | (u64::from(v) << 32)),
            _ => {
                self.spr.borrow_mut().insert(n, v);
            }
        }
    }

    pub fn snapshot(&self) -> RegsSnapshot {
        RegsSnapshot {
            gpr: std::array::from_fn(|i| self.gpr[i].get()),
            fpr: std::array::from_fn(|i| self.fpr[i].get()),
            cr: self.cr.get(),
            lr: self.lr.get(),
            ctr: self.ctr.get(),
            xer: self.xer.get(),
            fpscr: self.fpscr.get(),
            msr: self.msr.get(),
            gqr: std::array::from_fn(|i| self.gqr[i].get()),
            tb: self.tb.get(),
            spr: self.spr.borrow().clone(),
        }
    }

    pub fn restore(&self, s: &RegsSnapshot) {
        for i in 0..32 {
            self.gpr[i].set(s.gpr[i]);
            self.fpr[i].set(s.fpr[i]);
        }
        self.cr.set(s.cr);
        self.lr.set(s.lr);
        self.ctr.set(s.ctr);
        self.xer.set(s.xer);
        self.fpscr.set(s.fpscr);
        self.msr.set(s.msr);
        for i in 0..8 {
            self.gqr[i].set(s.gqr[i]);
        }
        self.tb.set(s.tb);
        *self.spr.borrow_mut() = s.spr.clone();
    }
}
