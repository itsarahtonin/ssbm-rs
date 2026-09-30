// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of functions in src/MSL/string.c that the translator cannot get right.

use ssbm_rt::{Ctx, Handle, Val};

/// Also leaves the length in r4, as the original does (`mr r3, r4`): Slippi's playback codes
/// call `strlen` and read the length from r4.
pub fn strlen<'a>(ctx: &'a Ctx, s: Val<'a, i8>) -> u32 {
    let mut k: u32 = 0;
    let mut p = Handle::addr(s);
    while ctx.read_u8(p) != 0 {
        k = k.wrapping_add(1);
        p = p.wrapping_add(1);
    }
    ctx.regs.set_r(4, k);
    k
}
