// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/ai/ai.c.

use ssbm_rt::{Addr, Ctx, Handle};
use ssbm_types::tu as statics;

use crate::support::ptr;

/// Runs the audio DMA callback `cb` on the stack AIRegisterDMACallback was given, and comes
/// back to this one after.
pub fn __AICallbackStackSwitch<'a>(ctx: &'a Ctx, cb: Addr<'a>) {
    let _frame = ctx.stack_frame(0x18);
    let old = statics::dolphin__ai__ai::__OldStack(ctx);
    old.set(ptr(ctx, ctx.regs.r(1)));
    let stack = Handle::addr(statics::dolphin__ai__ai::__CallbackStack(ctx).get());
    ctx.regs.set_r(1, stack.wrapping_sub(8));
    ctx.call::<_, ()>(Handle::addr(cb), ());
    ctx.regs.set_r(1, Handle::addr(old.get()));
}
