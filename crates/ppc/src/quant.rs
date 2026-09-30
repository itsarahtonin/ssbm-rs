// SPDX-License-Identifier: GPL-3.0-or-later
// Quantization semantics follow Dolphin's Interpreter_LoadStorePaired.cpp (GPL-2.0-or-later).

//! `psq_l`/`psq_st`: paired-single loads and stores converted through a GQR's type and scale.

use gekko_fp::{self as fp, Ps};
use ssbm_rt::Ctx;

/// 2^-scale for loads, with scale as a signed 6-bit number.
fn dequant_scale(s: u32) -> f32 {
    if s < 32 {
        1.0 / (1u64 << s) as f32
    } else {
        (1u64 << (64 - s)) as f32
    }
}

/// 2^scale for stores.
fn quant_scale(s: u32) -> f32 {
    if s < 32 {
        (1u64 << s) as f32
    } else {
        1.0 / (1u64 << (64 - s)) as f32
    }
}

pub(crate) fn load(ctx: &Ctx, ea: u32, fd: usize, single: bool, i: usize) {
    let gqr = ctx.regs.gqr[i].get();
    let (ty, scale) = ((gqr >> 16) & 7, (gqr >> 24) & 0x3F);
    let (size, conv): (u32, fn(u32, f32) -> f64) = match ty {
        0 => (4, |v, _| fp::lfs(v)),
        4 => (1, |v, s| f64::from(v as u8 as f32 * s)),
        5 => (2, |v, s| f64::from(v as u16 as f32 * s)),
        6 => (1, |v, s| f64::from(v as u8 as i8 as f32 * s)),
        7 => (2, |v, s| f64::from(v as u16 as i16 as f32 * s)),
        _ => panic!("psq_l with invalid GQR{i} load type {ty}"),
    };
    let s = dequant_scale(scale);
    let read = |addr: u32| ctx.read_be(addr, size) as u32;
    let ps0 = conv(read(ea), s);
    let ps1 = if single {
        1.0
    } else {
        conv(read(ea.wrapping_add(size)), s)
    };
    ctx.regs.fpr[fd].set(Ps::new(ps0, ps1));
}

pub(crate) fn store(ctx: &Ctx, ea: u32, fs: usize, single: bool, i: usize) {
    let gqr = ctx.regs.gqr[i].get();
    let (ty, scale) = (gqr & 7, (gqr >> 8) & 0x3F);
    let v = ctx.regs.fpr[fs].get();
    let q = quant_scale(scale);
    let clamp = |x: f64, min: f32, max: f32| (x as f32 * q).clamp(min, max);
    let (size, conv): (u32, Box<dyn Fn(f64) -> u32>) = match ty {
        0 => (4, Box::new(fp::stfs_ftz)),
        4 => (1, Box::new(|x| clamp(x, 0.0, 255.0) as u8 as u32)),
        5 => (2, Box::new(|x| clamp(x, 0.0, 65535.0) as u16 as u32)),
        6 => (1, Box::new(|x| clamp(x, -128.0, 127.0) as i8 as u8 as u32)),
        7 => (
            2,
            Box::new(|x| clamp(x, -32768.0, 32767.0) as i16 as u16 as u32),
        ),
        _ => panic!("psq_st with invalid GQR{i} store type {ty}"),
    };
    ctx.write_be(ea, size, u64::from(conv(v.ps0)));
    if !single {
        ctx.write_be(ea.wrapping_add(size), size, u64::from(conv(v.ps1)));
    }
}
