// SPDX-License-Identifier: GPL-3.0-or-later

//! Hand ports of the SDK's assembly functions (MSR and special register moves, the time base,
//! cache control) against the original code, run by the interpreter from the same states.
//! Runs only when `SSBM_DISC` points at a disc image.

use std::fmt::Debug;

use ssbm_disc::Disc;
use ssbm_game::manual::{
    dolphin__base__PPCArch as arch, dolphin__os__OSCache as cache, dolphin__os__OSInterrupt as irq,
    dolphin__os__OSTime as time, sysdolphin__baselib__hsd_397E as hsd,
};
use ssbm_ppc::Interpreter;
use ssbm_rt::{Ctx, RegsSnapshot, spr};
use ssbm_types::fns::addr;

fn machine() -> Option<Ctx> {
    let path = std::env::var_os("SSBM_DISC")?;
    let disc = Disc::open(path).expect("SSBM_DISC should be Melee NTSC 1.02");
    let ctx = Ctx::new();
    ctx.set_backend(Box::new(Interpreter::default()));
    disc.main_dol().unwrap().load_into(&ctx.mem).unwrap();
    ctx.regs.set_r(1, 0x8100_0000);
    Some(ctx)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn word(&mut self) -> u32 {
        self.next() as u32
    }
}

/// A scratch area both sides may write, compared after each call.
const SCRATCH: u32 = 0x8080_0000;
const SCRATCH_LEN: u32 = 0x200;

/// The state the ports may change, besides memory and results. CR0, which the assembly's
/// compares leave behind, is volatile.
fn state(r: &RegsSnapshot) -> impl PartialEq + Debug {
    (r.msr, r.tb, r.gqr, r.spr.clone())
}

fn scratch(ctx: &Ctx) -> Vec<u8> {
    (0..SCRATCH_LEN).map(|i| ctx.read_u8(SCRATCH + i)).collect()
}

/// Runs `original` and `port` from the same registers and memory and checks they agree.
fn same<R: PartialEq + Debug>(ctx: &Ctx, what: &str, original: impl Fn(&Ctx) -> R, port: impl Fn(&Ctx) -> R) {
    let regs = ctx.regs.snapshot();
    let mem = scratch(ctx);
    let want = original(ctx);
    let (want_regs, want_mem) = (ctx.regs.snapshot(), scratch(ctx));
    ctx.regs.restore(&regs);
    for (i, b) in mem.iter().enumerate() {
        ctx.write_u8(SCRATCH + i as u32, *b);
    }
    let got = port(ctx);
    assert_eq!(got, want, "{what}: result");
    assert_eq!(state(&ctx.regs.snapshot()), state(&want_regs), "{what}: registers");
    assert!(scratch(ctx) == want_mem, "{what}: memory");
}

#[test]
fn register_moves_match_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..200 {
        let v = rng.word();
        ctx.regs.msr.set(rng.word() & !0x8000);
        ctx.regs.set_spr(1008, rng.word());
        ctx.regs.set_spr(spr::HID2, rng.word());
        same(&ctx, "PPCMfmsr", |c| c.call::<_, u32>(addr::PPCMfmsr, ()), |c| arch::PPCMfmsr(c));
        same(&ctx, "PPCMtmsr", |c| c.call::<_, ()>(addr::PPCMtmsr, (v & !0x8000,)), |c| arch::PPCMtmsr(c, v & !0x8000));
        same(&ctx, "PPCMfhid0", |c| c.call::<_, u32>(addr::PPCMfhid0, ()), |c| arch::PPCMfhid0(c));
        same(&ctx, "PPCMfl2cr", |c| c.call::<_, u32>(addr::PPCMfl2cr, ()), |c| arch::PPCMfl2cr(c));
        same(&ctx, "PPCMtl2cr", |c| c.call::<_, ()>(addr::PPCMtl2cr, (v,)), |c| arch::PPCMtl2cr(c, v));
        same(&ctx, "PPCMtdec", |c| c.call::<_, ()>(addr::PPCMtdec, (v,)), |c| arch::PPCMtdec(c, v));
        same(&ctx, "PPCSync", |c| c.call::<_, ()>(addr::PPCSync, ()), |c| arch::PPCSync(c));
        same(&ctx, "PPCMfhid2", |c| c.call::<_, u32>(addr::PPCMfhid2, ()), |c| arch::PPCMfhid2(c));
        same(&ctx, "PPCMthid2", |c| c.call::<_, ()>(addr::PPCMthid2, (v,)), |c| arch::PPCMthid2(c, v));
        same(&ctx, "PPCMtwpar", |c| c.call::<_, ()>(addr::PPCMtwpar, (v,)), |c| arch::PPCMtwpar(c, v));
        // Numbers it has an `mfspr` for; others make it report, which needs the OS running.
        // LR (8) is the caller's return address, which a direct call does not set up.
        let n = [1, 9, 18, 19, 22, 26, 27, 272, 287, 530, 543, 912, 915, 920, 922, 953, 1008, 1017, 1022]
            [(rng.word() % 19) as usize];
        same(&ctx, "baselib_mfspr", |c| c.call::<_, i32>(addr::baselib_mfspr, (n,)), |c| hsd::baselib_mfspr(c, n));
    }
}

#[test]
fn interrupt_switches_match_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let mut rng = Rng(0x0F0F_1234_ABCD_9876);
    for _ in 0..200 {
        ctx.regs.msr.set(rng.word());
        let level = (rng.word() % 3) as i32;
        same(&ctx, "OSDisableInterrupts", |c| c.call::<_, i32>(addr::OSDisableInterrupts, ()), |c| irq::OSDisableInterrupts(c));
        same(&ctx, "OSEnableInterrupts", |c| c.call::<_, i32>(addr::OSEnableInterrupts, ()), |c| irq::OSEnableInterrupts(c));
        same(&ctx, "OSRestoreInterrupts", |c| c.call::<_, i32>(addr::OSRestoreInterrupts, (level,)), |c| irq::OSRestoreInterrupts(c, level));
    }
}

#[test]
fn time_base_reads_match_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let mut rng = Rng(0x5555_AAAA_1357_9BDF);
    ctx.regs.msr.set(0);
    for i in 0..200 {
        // Some reads straddle a carry into the high word, which makes OSGetTime read again.
        let tb = if i % 4 == 0 { (rng.next() & !0xFFFF_FFFF) | 0xFFFF_FFF8 } else { rng.next() };
        ctx.regs.tb.set(tb);
        same(&ctx, "OSGetTime", |c| c.call::<_, i64>(addr::OSGetTime, ()), |c| time::OSGetTime(c));
        same(&ctx, "OSGetTick", |c| c.call::<_, u32>(addr::OSGetTick, ()), |c| time::OSGetTick(c));
    }
}

#[test]
fn cache_control_matches_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let mut rng = Rng(0x2468_ACE0_1357_9BDF);
    for _ in 0..200 {
        for i in 0..SCRATCH_LEN {
            ctx.write_u8(SCRATCH + i, rng.word() as u8);
        }
        ctx.regs.set_spr(1008, rng.word());
        let start = SCRATCH + 0x40 + rng.word() % 0x100;
        let n = rng.word() % 0x80;
        let p = ssbm_game::support::ptr(&ctx, start);
        same(&ctx, "DCZeroRange", |c| c.call::<_, ()>(addr::DCZeroRange, (start, n)), |c| cache::DCZeroRange(c, p, n));
        same(&ctx, "DCFlushRange", |c| c.call::<_, ()>(addr::DCFlushRange, (start, n)), |c| cache::DCFlushRange(c, p, n));
        same(&ctx, "DCStoreRange", |c| c.call::<_, ()>(addr::DCStoreRange, (start, n)), |c| cache::DCStoreRange(c, p, n));
        same(&ctx, "DCInvalidateRange", |c| c.call::<_, ()>(addr::DCInvalidateRange, (start, n)), |c| cache::DCInvalidateRange(c, p, n));
        same(&ctx, "DCFlushRangeNoSync", |c| c.call::<_, ()>(addr::DCFlushRangeNoSync, (start, n)), |c| cache::DCFlushRangeNoSync(c, p, n));
        same(&ctx, "ICInvalidateRange", |c| c.call::<_, ()>(addr::ICInvalidateRange, (start, n)), |c| cache::ICInvalidateRange(c, p, n));
        same(&ctx, "DCEnable", |c| c.call::<_, ()>(addr::DCEnable, ()), |c| cache::DCEnable(c));
        same(&ctx, "ICEnable", |c| c.call::<_, ()>(addr::ICEnable, ()), |c| cache::ICEnable(c));
        same(&ctx, "ICFlashInvalidate", |c| c.call::<_, ()>(addr::ICFlashInvalidate, ()), |c| cache::ICFlashInvalidate(c));
        same(&ctx, "LCDisable", |c| c.call::<_, ()>(addr::LCDisable, ()), |c| cache::LCDisable(c));
        for i in 0..0x200 {
            ctx.write_u8(0xE000_0000 + i, rng.word() as u8);
        }
        let (dest, tag) = (SCRATCH + ((rng.word() % 0x100) & !31), 0xE000_0000 + ((rng.word() % 0x100) & !31));
        let blocks = 1 + rng.word() % 8;
        let (d, t) = (ssbm_game::support::ptr(&ctx, dest), ssbm_game::support::ptr(&ctx, tag));
        same(&ctx, "LCStoreBlocks", |c| c.call::<_, ()>(addr::LCStoreBlocks, (dest, tag, blocks)), |c| cache::LCStoreBlocks(c, d, t, blocks));
    }
}
