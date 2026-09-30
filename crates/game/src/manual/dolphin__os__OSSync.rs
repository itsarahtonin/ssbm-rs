// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of the assembly functions of libs/dolphin/src/dolphin/os/OSSync.c.

use ssbm_rt::Ctx;

/// The `sc` handler __OSInitSystemCall copies to 0xC00: it sets and restores a HID0 bit around
/// a sync, which leaves nothing changed here, as the interpreter's `sc` does.
pub fn SystemCallVector(_ctx: &Ctx) {}
