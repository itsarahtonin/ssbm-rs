// SPDX-License-Identifier: GPL-3.0-or-later
// The boot sequence follows Dolphin's EmulatedBS2_GC (GPL-2.0-or-later).

//! Boots the disc as the GameCube's IPL would: low memory and registers first, then the disc's
//! own apploader loads the executable and file system table. Needs the interpreter, since the
//! apploader is code on the disc.

use ssbm_rt::{Ctx, spr};

use crate::printf::{self, VaArgs};
use crate::{Sdk, TB_HZ};

/// Seconds from 2000-01-01 to 2001-12-03, Melee's North American release, used as the
/// console's clock so runs do not depend on the date.
pub const DEFAULT_CLOCK: u64 = 702 * 86_400;

const APPLOADER: u64 = 0x2440;
const APPLOADER_LOAD: u32 = 0x8120_0000;
const APPLOADER_FUNCS: u32 = 0x8000_3100;
const REPORT: u32 = 0x8130_0000;
const MAIN_ARGS: u32 = 0x8130_0004;

/// Loads the game into memory and returns its entry point, ready for `ctx.invoke`.
pub fn boot(ctx: &Ctx, clock_seconds: u64) -> u32 {
    let sdk = ctx.ext::<Sdk>();

    // Registers as the IPL leaves them.
    ctx.regs.msr.set(0x0000_2032);
    ctx.regs.set_spr(1008, 0x0011_C464); // HID0
    ctx.regs.set_spr(spr::HID2, 0xE000_0000);
    for (n, v) in [
        (528, 0x8000_1FFF), // IBAT0U
        (529, 0x0000_0002),
        (536, 0x8000_1FFF), // DBAT0U
        (537, 0x0000_0002),
        (538, 0xC000_1FFF), // DBAT1U
        (539, 0x0000_002A),
    ] {
        ctx.regs.set_spr(n, v);
    }

    // Low memory.
    ctx.write_u32(0x8000_0020, 0x0D15_EA5E); // booted from the boot ROM
    ctx.write_u32(0x8000_0028, 0x0180_0000); // 24 MB of memory
    ctx.write_u32(0x8000_002C, 0x1000_0006); // console type, as Dolphin reports it
    ctx.write_u32(0x8000_00CC, 0); // NTSC
    ctx.write_u32(0x8000_00D0, 0x0100_0000); // 16 MB of ARAM
    ctx.write_u32(0x8000_00F8, 0x09A7_EC80); // bus clock
    ctx.write_u32(0x8000_00FC, 0x1CF7_C580); // CPU clock
    for handler in [0x8000_0300, 0x8000_0800, 0x8000_0C00] {
        ctx.write_u32(handler, 0x4C00_0064); // rfi
    }
    ctx.write_u64(0x8000_30D8, clock_seconds * TB_HZ);

    let mut id = [0u8; 0x20];
    sdk.read_disc(0, &mut id);
    let _ = ctx.mem.write_bytes(0x8000_0000, &id);

    ctx.regs.set_r(1, 0x8156_6550);
    ctx.regs.set_r(2, 0x8146_5CC0);
    ctx.regs.set_r(13, 0x8146_5320);

    // The apploader: entry returns its init, main and close functions.
    let mut header = [0u8; 0x20];
    sdk.read_disc(APPLOADER, &mut header);
    let word = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap());
    let (entry, size, trailer) = (word(0x10), word(0x14), word(0x18));
    let mut code = vec![0u8; (size + trailer) as usize];
    sdk.read_disc(APPLOADER + 0x20, &mut code);
    let _ = ctx.mem.write_bytes(APPLOADER_LOAD, &code);

    ctx.regs.set_r(3, APPLOADER_FUNCS);
    ctx.regs.set_r(4, APPLOADER_FUNCS + 4);
    ctx.regs.set_r(5, APPLOADER_FUNCS + 8);
    ctx.run_original(entry);
    let init = ctx.read_u32(APPLOADER_FUNCS);
    let main = ctx.read_u32(APPLOADER_FUNCS + 4);
    let close = ctx.read_u32(APPLOADER_FUNCS + 8);

    ctx.write_u32(REPORT, 0x4E80_0020); // blr
    ctx.register(REPORT, apploader_report);
    ctx.regs.set_r(3, REPORT);
    ctx.run_original(init);

    // Each call to main asks for one read, until it returns 0.
    loop {
        ctx.regs.set_r(3, MAIN_ARGS);
        ctx.regs.set_r(4, MAIN_ARGS + 4);
        ctx.regs.set_r(5, MAIN_ARGS + 8);
        ctx.run_original(main);
        if ctx.regs.r(3) == 0 {
            break;
        }
        let ram = ctx.read_u32(MAIN_ARGS);
        let len = ctx.read_u32(MAIN_ARGS + 4);
        let offset = ctx.read_u32(MAIN_ARGS + 8);
        let mut data = vec![0u8; len as usize];
        sdk.read_disc(u64::from(offset), &mut data);
        let _ = ctx
            .mem
            .write_bytes(0x8000_0000 | (ram & 0x01FF_FFFF), &data);
    }

    ctx.run_original(close);
    ctx.unregister(REPORT);
    ctx.regs.r(3)
}

fn apploader_report(ctx: &Ctx) {
    let text = printf::format(ctx, ctx.regs.r(3), &mut VaArgs::new(ctx, 4));
    (ctx.ext::<Sdk>().report.borrow_mut())(&format!("apploader: {text}"));
}
