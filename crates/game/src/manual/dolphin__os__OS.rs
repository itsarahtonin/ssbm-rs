// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/os/OS.c.

use ssbm_rt::{Ctx, spr};
use ssbm_types::fns;
use ssbm_types::records::OSContext;

use super::exception::save_context;

/// Saves the rest of the context and reports the exception as unhandled, with DSISR and DAR.
pub fn OSDefaultExceptionHandler<'a>(ctx: &'a Ctx, exception: u8, context: OSContext<'a>) {
    save_context(ctx, context);
    let (dsisr, dar) = (ctx.regs.get_spr(spr::DSISR), ctx.regs.get_spr(spr::DAR));
    fns::__OSUnhandledException(ctx, exception, context, dsisr, dar);
}

/// The debugger hook the OS copies to 0x60, which exception vectors jump to with translation
/// off. Nothing calls it as a function; OSInit copies its instructions.
pub fn __OSDBIntegrator(_ctx: &Ctx) {
    panic!("__OSDBIntegrator is exception vector code, not a function");
}

/// The end of __OSDBIntegrator's instructions, which OSInit also copies.
pub fn __OSDBJump(_ctx: &Ctx) {
    panic!("__OSDBJump is exception vector code, not a function");
}

/// The code OSInit copies to each exception vector. Nothing calls it as a function.
pub fn OSExceptionVector(_ctx: &Ctx) {
    panic!("OSExceptionVector is exception vector code, not a function");
}

/// Turns on paired singles and locked cache in HID2, and makes GQR0 the plain float format.
pub fn __OSPSInit(ctx: &Ctx) {
    let hid2: u32 = ctx.call(fns::addr::PPCMfhid2, ());
    fns::PPCMthid2(ctx, hid2 | 0xA000_0000);
    fns::ICFlashInvalidate(ctx);
    ctx.regs.gqr[0].set(0);
}
