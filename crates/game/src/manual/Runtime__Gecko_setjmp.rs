// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of src/Runtime/Gecko_setjmp.c, whose functions are assembly.

use ssbm_rt::{Ctx, Handle};
use ssbm_types::records::jmp_buf;

/// Saves where to return and the registers a call preserves, for original code: ports'
/// `setjmp`s go through `Ctx::setjmp`.
pub fn __setjmp<'a>(ctx: &'a Ctx, env: jmp_buf<'a>) -> i32 {
    ctx.setjmp_original(Handle::addr(env));
    0
}

/// Jumps back to the `setjmp` that saved `env`, which returns `val`, or 1 for 0.
pub fn __longjmp<'a>(ctx: &'a Ctx, env: jmp_buf<'a>, val: i32) {
    ctx.longjmp(Handle::addr(env), val);
}
