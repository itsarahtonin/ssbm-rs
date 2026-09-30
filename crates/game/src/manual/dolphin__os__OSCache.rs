// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/os/OSCache.c. Emulated memory
// has no caches, so flushes, stores and invalidations do nothing, as the interpreter's cache
// instructions do; what remains are the HID0 and HID2 bits, `dcbz`, and the locked cache.

use ssbm_rt::{Addr, Ctx, Handle, spr};

const HID0: u32 = 1008;
const DBAT3U: u32 = 542;
const DBAT3L: u32 = 543;
const DMA_U: u32 = 922;
const DMA_L: u32 = 923;
const LOCKED_CACHE: u32 = 0xE000_0000;

fn set_hid0_bits(ctx: &Ctx, bits: u32) {
    ctx.regs.set_spr(HID0, ctx.regs.get_spr(HID0) | bits);
}

pub fn DCEnable(ctx: &Ctx) {
    set_hid0_bits(ctx, 0x4000);
}

pub fn DCInvalidateRange<'a>(_ctx: &'a Ctx, _addr: Addr<'a>, _n_bytes: u32) {}

pub fn DCFlushRange<'a>(_ctx: &'a Ctx, _addr: Addr<'a>, _n_bytes: u32) {}

pub fn DCStoreRange<'a>(_ctx: &'a Ctx, _addr: Addr<'a>, _n_bytes: u32) {}

pub fn DCFlushRangeNoSync<'a>(_ctx: &'a Ctx, _addr: Addr<'a>, _n_bytes: u32) {}

/// Zeroes every 32-byte line the range touches, as `dcbz` does.
pub fn DCZeroRange<'a>(ctx: &'a Ctx, addr: Addr<'a>, n_bytes: u32) {
    if n_bytes == 0 {
        return;
    }
    let addr = Handle::addr(addr);
    let n = if addr & 31 != 0 { n_bytes.wrapping_add(32) } else { n_bytes };
    let lines = n.wrapping_add(31) >> 5;
    for i in 0..lines {
        ctx.fill(addr.wrapping_add(32 * i) & !31, 0, 32);
    }
}

pub fn ICInvalidateRange<'a>(_ctx: &'a Ctx, _addr: Addr<'a>, _n_bytes: u32) {}

pub fn ICFlashInvalidate(ctx: &Ctx) {
    set_hid0_bits(ctx, 0x800);
}

pub fn ICEnable(ctx: &Ctx) {
    set_hid0_bits(ctx, 0x8000);
}

/// Turns on the locked cache: maps it at 0xE0000000 through DBAT3 and zeroes it.
pub fn __LCEnable(ctx: &Ctx) {
    ctx.set_msr(ctx.regs.msr.get() | 0x1000);
    ctx.regs.set_spr(spr::HID2, ctx.regs.get_spr(spr::HID2) | 0x100F_0000);
    ctx.regs.set_spr(DBAT3L, 0xE000_0002);
    ctx.regs.set_spr(DBAT3U, 0xE000_01FE);
    ctx.fill(LOCKED_CACHE, 0, 0x200 * 32);
}

pub fn LCDisable(ctx: &Ctx) {
    ctx.regs.set_spr(spr::HID2, ctx.regs.get_spr(spr::HID2) & !0x1000_0000);
}

/// Queues a DMA of `num_blocks` 32-byte blocks between the locked cache and memory.
pub fn LCStoreBlocks<'a>(ctx: &'a Ctx, dest_addr: Addr<'a>, src_tag: Addr<'a>, num_blocks: u32) {
    let (dest, src) = (Handle::addr(dest_addr), Handle::addr(src_tag));
    ctx.regs.set_spr(DMA_U, ((num_blocks >> 2) & 0x1F) | (dest & 0x0FFF_FFFF));
    ctx.regs.set_spr(DMA_L, ((num_blocks & 3) << 2) | src | 2);
}

/// Waits until at most `len` DMAs are queued, as HID2 counts them.
pub fn LCQueueWait(ctx: &Ctx, len: u32) {
    let limit = len.wrapping_add(1) as i32;
    while ((ctx.regs.get_spr(spr::HID2) >> 24) & 0xF) as i32 >= limit {}
}
