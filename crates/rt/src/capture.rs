// SPDX-License-Identifier: GPL-3.0-or-later

//! Single calls saved from a run, to check again apart from it: the memory and registers a
//! check started from, whether its inputs were mutated and what callee a mutated check stood in
//! for. Loading one puts the machine back as the call found it, so the check runs again in
//! milliseconds instead of a whole run that reaches it. A call holds captured game memory, so
//! files of them stay out of the repository.

use std::collections::BTreeMap;

use gekko_fp::Ps;
use ssbm_mem::{LOCKED_CACHE, LOCKED_CACHE_SIZE, MEM1_SIZE};

use crate::{Ctx, RegsSnapshot};

const MAGIC: &[u8; 8] = b"SSBMCALL";
const VERSION: u32 = 1;

pub struct Call {
    /// The function called.
    pub addr: u32,
    /// Whether this was a mutated check, which meets null hardware and may not be mutated again.
    pub mutated: bool,
    /// The callee a mutated check stood in for, with what it returned (r3, f1).
    pub stub: Option<(u32, u32, f64)>,
    pub regs: RegsSnapshot,
    /// Main memory and the locked cache.
    pub mem1: Vec<u8>,
    pub locked: Vec<u8>,
}

impl Call {
    /// The call to `addr` from the machine as it is, with registers `regs`.
    pub fn take(ctx: &Ctx, addr: u32, regs: &RegsSnapshot) -> Self {
        let mut mem1 = vec![0; MEM1_SIZE as usize];
        let mut locked = vec![0; LOCKED_CACHE_SIZE as usize];
        let _ = ctx.mem.read_bytes(0x8000_0000, &mut mem1);
        let _ = ctx.mem.read_bytes(LOCKED_CACHE, &mut locked);
        Self {
            addr,
            mutated: ctx.lockstep.is_mutating(),
            stub: ctx.lockstep.stub_now(),
            regs: regs.clone(),
            mem1,
            locked,
        }
    }

    /// Puts the machine as the call found it.
    pub fn load(&self, ctx: &Ctx) {
        let _ = ctx.mem.write_bytes(0x8000_0000, &self.mem1);
        let _ = ctx.mem.write_bytes(LOCKED_CACHE, &self.locked);
        ctx.regs.restore(&self.regs);
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.mem1.len() + self.locked.len() + 1024);
        out.extend_from_slice(MAGIC);
        let u32s =
            |out: &mut Vec<u8>, v: &[u32]| v.iter().for_each(|x| out.extend(x.to_le_bytes()));
        u32s(&mut out, &[VERSION, self.addr, u32::from(self.mutated)]);
        let (callee, r3, f1) = self.stub.unwrap_or((0, 0, 0.0));
        u32s(&mut out, &[u32::from(self.stub.is_some()), callee, r3]);
        out.extend(f1.to_bits().to_le_bytes());
        let r = &self.regs;
        u32s(&mut out, &r.gpr);
        for p in &r.fpr {
            out.extend(p.ps0.to_bits().to_le_bytes());
            out.extend(p.ps1.to_bits().to_le_bytes());
        }
        u32s(&mut out, &[r.cr, r.lr, r.ctr, r.xer, r.fpscr, r.msr]);
        u32s(&mut out, &r.gqr);
        out.extend(r.tb.to_le_bytes());
        u32s(&mut out, &[r.spr.len() as u32]);
        for (&k, &v) in &r.spr {
            u32s(&mut out, &[k, v]);
        }
        u32s(&mut out, &[self.mem1.len() as u32, self.locked.len() as u32]);
        out.extend_from_slice(&self.mem1);
        out.extend_from_slice(&self.locked);
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let mut r = Reader { b, at: 0 };
        if r.bytes(8)? != MAGIC || r.u32()? != VERSION {
            return None;
        }
        let addr = r.u32()?;
        let mutated = r.u32()? != 0;
        let (has_stub, callee, r3) = (r.u32()? != 0, r.u32()?, r.u32()?);
        let f1 = f64::from_bits(r.u64()?);
        let mut gpr = [0; 32];
        for g in &mut gpr {
            *g = r.u32()?;
        }
        let mut fpr = [Ps::default(); 32];
        for p in &mut fpr {
            *p = Ps::new(f64::from_bits(r.u64()?), f64::from_bits(r.u64()?));
        }
        let (cr, lr, ctr) = (r.u32()?, r.u32()?, r.u32()?);
        let (xer, fpscr, msr) = (r.u32()?, r.u32()?, r.u32()?);
        let mut gqr = [0; 8];
        for g in &mut gqr {
            *g = r.u32()?;
        }
        let tb = r.u64()?;
        let mut spr = BTreeMap::new();
        for _ in 0..r.u32()? {
            let k = r.u32()?;
            spr.insert(k, r.u32()?);
        }
        let (n1, n2) = (r.u32()? as usize, r.u32()? as usize);
        let mem1 = r.bytes(n1)?.to_vec();
        let locked = r.bytes(n2)?.to_vec();
        Some(Self {
            addr,
            mutated,
            stub: has_stub.then_some((callee, r3, f1)),
            regs: RegsSnapshot { gpr, fpr, cr, lr, ctr, xer, fpscr, msr, gqr, tb, spr },
            mem1,
            locked,
        })
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }
}
