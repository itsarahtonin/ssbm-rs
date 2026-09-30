// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of libs/dolphin/src/dolphin/base/PPCArch.c, whose functions are assembly: moves
// to and from the CPU's special registers, as the interpreter executes them.

use ssbm_rt::{Ctx, spr};

const HID0: u32 = 1008;
const L2CR: u32 = 1017;
const WPAR: u32 = 921;

/// `mfmsr`.
pub fn PPCMfmsr(ctx: &Ctx) -> u32 {
    ctx.regs.msr.get()
}

/// `mtmsr`, which takes pending interrupts if it enables them.
pub fn PPCMtmsr(ctx: &Ctx, new_msr: u32) {
    ctx.set_msr(new_msr);
}

pub fn PPCMfhid0(ctx: &Ctx) -> u32 {
    ctx.regs.get_spr(HID0)
}

pub fn PPCMfl2cr(ctx: &Ctx) -> u32 {
    ctx.regs.get_spr(L2CR)
}

pub fn PPCMtl2cr(ctx: &Ctx, new_l2cr: u32) {
    ctx.regs.set_spr(L2CR, new_l2cr);
}

pub fn PPCMtdec(ctx: &Ctx, new_dec: u32) {
    ctx.regs.set_spr(spr::DEC, new_dec);
}

/// `sc`, whose handler only synchronizes the caches, which have nothing to synchronize here.
pub fn PPCSync(_ctx: &Ctx) {}

/// Spins forever, for a fatal error; a port stops instead.
pub fn PPCHalt(_ctx: &Ctx) {
    panic!("PPCHalt: the game halted");
}

pub fn PPCMfhid2(ctx: &Ctx) -> u32 {
    ctx.regs.get_spr(spr::HID2)
}

pub fn PPCMthid2(ctx: &Ctx, new_hid2: u32) {
    ctx.regs.set_spr(spr::HID2, new_hid2);
}

pub fn PPCMtwpar(ctx: &Ctx, new_wpar: u32) {
    ctx.regs.set_spr(WPAR, new_wpar);
}
