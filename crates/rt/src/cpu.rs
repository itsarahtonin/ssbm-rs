// SPDX-License-Identifier: GPL-3.0-or-later
// Instruction semantics follow Dolphin's interpreter (GPL-2.0-or-later).

//! The Gekko's register-level semantics that both the interpreter and ports transliterated from
//! assembly use: condition and fixed-point exception registers, compares, and paired-single
//! loads and stores converted through a GQR.

use gekko_fp::{self as fp, Ps};

use crate::Ctx;

pub const XER_SO: u32 = 1 << 31;
pub const XER_OV: u32 = 1 << 30;
pub const XER_CA: u32 = 1 << 29;

#[inline]
pub fn cr_bit(ctx: &Ctx, b: u32) -> bool {
    (ctx.regs.cr.get() >> (31 - b)) & 1 != 0
}

#[inline]
pub fn set_cr_bit(ctx: &Ctx, b: u32, v: bool) {
    let m = 1 << (31 - b);
    let cr = ctx.regs.cr.get();
    ctx.regs.cr.set(if v { cr | m } else { cr & !m });
}

#[inline]
pub fn set_cr_field(ctx: &Ctx, field: u32, v: u32) {
    let sh = 28 - 4 * field;
    let cr = ctx.regs.cr.get();
    ctx.regs.cr.set((cr & !(0xF << sh)) | ((v & 0xF) << sh));
}

#[inline]
pub fn cr_field(ctx: &Ctx, field: u32) -> u32 {
    (ctx.regs.cr.get() >> (28 - 4 * field)) & 0xF
}

/// XER[SO], which compares copy into their field.
#[inline]
pub fn so(ctx: &Ctx) -> u32 {
    ctx.regs.xer.get() >> 31
}

/// CR0 from a result's sign, for the dotted forms.
#[inline]
pub fn update_cr0(ctx: &Ctx, v: u32) {
    let s = v as i32;
    let f = if s < 0 {
        8
    } else if s > 0 {
        4
    } else {
        2
    };
    set_cr_field(ctx, 0, f | so(ctx));
}

/// CR1 from FPSCR's exception summary, for the dotted floating-point forms.
#[inline]
pub fn update_cr1(ctx: &Ctx) {
    set_cr_field(ctx, 1, ctx.regs.fpscr.get() >> 28);
}

#[inline]
pub fn ca(ctx: &Ctx) -> u32 {
    (ctx.regs.xer.get() >> 29) & 1
}

#[inline]
pub fn set_ca(ctx: &Ctx, c: bool) {
    let x = ctx.regs.xer.get();
    ctx.regs.xer.set(if c { x | XER_CA } else { x & !XER_CA });
}

#[inline]
pub fn set_ov(ctx: &Ctx, o: bool) {
    let x = ctx.regs.xer.get();
    ctx.regs
        .xer
        .set(if o { x | XER_OV | XER_SO } else { x & !XER_OV });
}

/// `a + b + c` with carry out and signed overflow.
#[inline]
pub fn add3(a: u32, b: u32, c: u32) -> (u32, bool, bool) {
    let sum = u64::from(a) + u64::from(b) + u64::from(c);
    let d = sum as u32;
    (d, sum >> 32 != 0, ((a ^ d) & (b ^ d)) >> 31 != 0)
}

/// The mask of `rlwinm` and friends: bits mb through me, wrapping around.
#[inline]
pub fn rot_mask(mb: u32, me: u32) -> u32 {
    let begin = u32::MAX >> mb;
    let end = u32::MAX << (31 - me);
    if mb <= me { begin & end } else { begin | end }
}

/// Sets a CR field from a comparison's outcome.
#[inline]
pub fn compare(ctx: &Ctx, field: u32, lt: bool, gt: bool) {
    let f = if lt {
        8
    } else if gt {
        4
    } else {
        2
    };
    set_cr_field(ctx, field, f | so(ctx));
}

/// `fcmpu`/`fcmpo`: the CR field and FPSCR[FPCC].
pub fn fp_compare(ctx: &Ctx, field: u32, a: f64, b: f64) {
    let f = match fp::fcmp(a, b) {
        fp::FpCompare::Less => 8,
        fp::FpCompare::Greater => 4,
        fp::FpCompare::Equal => 2,
        fp::FpCompare::Unordered => 1,
    };
    set_cr_field(ctx, field, f);
    let fpscr = ctx.regs.fpscr.get();
    ctx.regs.fpscr.set((fpscr & !0xF000) | (f << 12));
}

/// A conditional branch's condition part.
#[inline]
pub fn cond_ok(ctx: &Ctx, bo: u32, bi: u32) -> bool {
    bo & 0x10 != 0 || cr_bit(ctx, bi) == (bo & 0x08 != 0)
}

/// A conditional branch's counter part, which decrements CTR unless BO says not to.
#[inline]
pub fn ctr_ok(ctx: &Ctx, bo: u32) -> bool {
    if bo & 0x04 != 0 {
        return true;
    }
    let ctr = ctx.regs.ctr.get().wrapping_sub(1);
    ctx.regs.ctr.set(ctr);
    (ctr != 0) != (bo & 0x02 != 0)
}

/// Whether `tw`/`twi` with TO traps for these operands.
pub fn trap(to: u32, a: u32, b: u32) -> bool {
    let (sa, sb) = (a as i32, b as i32);
    (to & 16 != 0 && sa < sb)
        || (to & 8 != 0 && sa > sb)
        || (to & 4 != 0 && a == b)
        || (to & 2 != 0 && a < b)
        || (to & 1 != 0 && a > b)
}

/// A single-precision result, which fills both halves of the register.
#[inline]
pub fn fill(ctx: &Ctx, i: usize, v: f64) {
    ctx.regs.fpr[i].set(Ps::splat(v))
}

/// A `bl` from a port transliterated from assembly: the callee returns to `ret`, where the
/// port goes on.
pub fn call(ctx: &Ctx, target: u32, ret: u32) {
    ctx.regs.lr.set(ret);
    ctx.invoke(target);
    if let Some(to) = ctx.take_resume_at() {
        panic!("{} jumped to {} past a port", ctx.name_of(target), ctx.name_of(to));
    }
}

/// A jump from a port transliterated from assembly into another function, which returns to the
/// port's caller.
pub fn tail_call(ctx: &Ctx, target: u32) {
    ctx.invoke(target);
    if let Some(to) = ctx.take_resume_at() {
        panic!("{} jumped to {} past a port", ctx.name_of(target), ctx.name_of(to));
    }
}

// `psq_l`/`psq_st`: paired-single loads and stores converted through a GQR's type and scale.
// Quantization semantics follow Dolphin's Interpreter_LoadStorePaired.cpp (GPL-2.0-or-later).

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

/// `psq_l`: loads one or two values into FPR `fd` through GQR `i`.
pub fn psq_load(ctx: &Ctx, ea: u32, fd: usize, single: bool, i: usize) {
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

/// `psq_st`: stores one or two values from FPR `fs` through GQR `i`.
pub fn psq_store(ctx: &Ctx, ea: u32, fs: usize, single: bool, i: usize) {
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
