// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/os/OSInterrupt.c: MSR[EE]
// moves, and the external interrupt handler.

use ssbm_rt::{Ctx, MSR_EE};
use ssbm_types::fns;
use ssbm_types::records::OSContext;

use super::exception::save_context;

/// MSR[EE] as the 0 or 1 `extrwi r3, r3, 1, 16` makes of it.
fn ee(msr: u32) -> i32 {
    ((msr & MSR_EE) >> 15) as i32
}

/// Turns interrupts off; returns whether they were on.
pub fn OSDisableInterrupts(ctx: &Ctx) -> i32 {
    let msr = ctx.regs.msr.get();
    ctx.set_msr(msr & !MSR_EE);
    ee(msr)
}

/// Turns interrupts on, taking any that are pending; returns whether they were on.
pub fn OSEnableInterrupts(ctx: &Ctx) -> i32 {
    let msr = ctx.regs.msr.get();
    ctx.set_msr(msr | MSR_EE);
    ee(msr)
}

/// Turns interrupts on or off as `level` says. The assembly computes the old state into r4,
/// so it returns `level` itself, still in r3.
pub fn OSRestoreInterrupts(ctx: &Ctx, level: i32) -> i32 {
    let msr = ctx.regs.msr.get();
    ctx.set_msr(if level != 0 { msr | MSR_EE } else { msr & !MSR_EE });
    level
}

/// Saves the rest of the interrupted context, then dispatches the interrupt. The SDK layer
/// delivers interrupts straight to their handlers, so only code that calls this runs it.
pub fn ExternalInterruptHandler<'a>(ctx: &'a Ctx, exception: u8, context: OSContext<'a>) {
    save_context(ctx, context);
    fns::__OSDispatchInterrupt(ctx, exception, context);
}
