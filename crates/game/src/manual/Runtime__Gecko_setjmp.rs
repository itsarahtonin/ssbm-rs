// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of src/Runtime/Gecko_setjmp.c, whose functions are assembly.

use ssbm_rt::{Ctx, Handle};
use ssbm_types::records::jmp_buf;

const GPRS: u32 = 0x14;
const FPRS: u32 = 0x60;
const FPSCR: u32 = 0xF0;

/// Saves where to return and the registers a call preserves, as the original does, for original
/// code: ports' `setjmp`s go through `Ctx::setjmp`.
pub fn __setjmp<'a>(ctx: &'a Ctx, env: jmp_buf<'a>) -> i32 {
    let r = &ctx.regs;
    let at = Handle::addr(env);
    env.set_pc(r.lr.get());
    env.set_cr(r.cr.get());
    env.set_sp(r.r(1));
    env.set_rtoc(r.r(2));
    for i in 13..32 {
        ctx.write_u32(at + GPRS + 4 * (i - 13), r.r(i as usize));
    }
    for i in 14..32 {
        ctx.write_u64(at + FPRS + 8 * (i - 14), r.fpr[i as usize].get().ps0.to_bits());
    }
    // `mffs` leaves FPSCR in the low word.
    ctx.write_u64(at + FPSCR, 0xFFF8_0000_0000_0000 | u64::from(r.fpscr.get()));
    0
}

/// Jumps back to the `setjmp` that saved `env`, which returns `val`, or 1 for 0. To a port's,
/// it unwinds; to original code's, it restores what that saved and resumes there.
pub fn __longjmp<'a>(ctx: &'a Ctx, env: jmp_buf<'a>, val: i32) {
    let at = Handle::addr(env);
    ctx.longjmp(at, val);
    let r = &ctx.regs;
    r.cr.set(env.cr());
    r.set_r(1, env.sp());
    r.set_r(2, env.rtoc());
    for i in 13..32 {
        r.set_r(i as usize, ctx.read_u32(at + GPRS + 4 * (i - 13)));
    }
    for i in 14..32 {
        let mut ps = r.fpr[i as usize].get();
        ps.ps0 = f64::from_bits(ctx.read_u64(at + FPRS + 8 * (i - 14)));
        r.fpr[i as usize].set(ps);
    }
    r.fpscr.set(ctx.read_u64(at + FPSCR) as u32);
    // `cmpwi val, 0` after the CR is restored.
    let cr0 = match val {
        v if v < 0 => 8,
        v if v > 0 => 4,
        _ => 2,
    } | (r.xer.get() >> 31);
    r.cr.set((r.cr.get() & 0x0FFF_FFFF) | (cr0 << 28));
    r.set_r(3, if val == 0 { 1 } else { val as u32 });
    ctx.resume_at(env.pc());
}
