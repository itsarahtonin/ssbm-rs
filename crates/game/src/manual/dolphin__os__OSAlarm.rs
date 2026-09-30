// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/os/OSAlarm.c.

use ssbm_rt::Ctx;
use ssbm_types::records::OSContext;
use ssbm_types::tu as statics;

use super::exception::save_context;

/// Saves the rest of the context, then runs the alarms that are due.
pub fn DecrementerExceptionHandler<'a>(ctx: &'a Ctx, exception: u8, context: OSContext<'a>) {
    save_context(ctx, context);
    statics::dolphin__os__OSAlarm::DecrementerExceptionCallback(ctx, exception, context);
}
