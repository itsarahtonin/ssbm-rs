// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/db/db.c.

use ssbm_rt::Ctx;
use ssbm_types::fns;

/// Turns address translation back on, then reports the exception to the debugger.
pub fn __DBExceptionDestination(ctx: &Ctx) {
    ctx.set_msr(ctx.regs.msr.get() | 0x30);
    fns::__DBExceptionDestinationAux(ctx);
}
