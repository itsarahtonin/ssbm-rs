// SPDX-License-Identifier: GPL-3.0-or-later

//! What the OS's exception handlers do before handing an exception to C code.

use ssbm_rt::{Ctx, Handle};
use ssbm_types::records::OSContext;

/// Saves the registers the exception vector left unsaved into `context`: r0 to r2, r6 to r31,
/// and GQR1 to GQR7. The vector itself saved the others.
pub(crate) fn save_context(ctx: &Ctx, context: OSContext<'_>) {
    let base = Handle::addr(context);
    for r in 0..3 {
        ctx.write_u32(base + 4 * r as u32, ctx.regs.r(r));
    }
    for r in 6..32 {
        ctx.write_u32(base + 0x18 + 4 * (r as u32 - 6), ctx.regs.r(r));
    }
    for q in 1..8 {
        ctx.write_u32(base + 0x1A8 + 4 * (q as u32 - 1), ctx.regs.gqr[q].get());
    }
}
