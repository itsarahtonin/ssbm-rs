// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/os/OSTime.c.

use ssbm_rt::Ctx;

/// The 64-bit time base, read high, low, high until the high word holds still.
pub fn OSGetTime(ctx: &Ctx) -> i64 {
    loop {
        let hi = (ctx.regs.tb.get() >> 32) as u32;
        let lo = ctx.read_tbl();
        if (ctx.regs.tb.get() >> 32) as u32 == hi {
            return ((u64::from(hi) << 32) | u64::from(lo)) as i64;
        }
    }
}

/// The time base's low word.
pub fn OSGetTick(ctx: &Ctx) -> u32 {
    ctx.read_tbl()
}
