// SPDX-License-Identifier: GPL-3.0-or-later

//! Slippi replay playback in the dev runtime, set up as Slippi Dolphin does it: Slippi's
//! bootloader codes load the playback code set, and the playback codes ask a Slippi EXI device
//! for the replay's game info, Gecko codes and inputs. The recording codes run too, so a run
//! produces a replay of its own to compare against the original.

use std::rc::Rc;

use ssbm_rt::Ctx;

pub mod compare;
pub mod denylist;
pub mod device;
pub mod gecko;
pub mod replay;

pub use device::Device;
pub use replay::Replay;

/// Slippi's playback code set (Slippi Dolphin's `GameSettings/Playback/GALE01r2.ini`).
pub const PLAYBACK_INI: &str = include_str!("../data/playback.ini");
/// Slippi's bootloader codes (slippi-ssbm-asm `Output/Bootloader/bootloader.txt`).
pub const BOOTLOADER: &str = include_str!("../data/bootloader.txt");

/// Where Slippi's codes put their EXI transfer function.
pub const EXI_TRANSFER_BUFFER: u32 = 0x8000_55F0;
/// Where the bootloader's code list goes, in the low memory the code handler would use.
const BOOTLOADER_LIST: u32 = 0x8000_2000;

/// Puts a Slippi device holding `replay` into `ctx`. Call `apply_bootloader` once the game is
/// loaded.
pub fn install(ctx: &Ctx, replay: Replay) -> Rc<Device> {
    let gct = gecko::gct(&gecko::parse_ini(PLAYBACK_INI));
    let device = ctx.set_ext(Device::new(replay, gct));
    ctx.register(EXI_TRANSFER_BUFFER, exi_transfer_buffer);
    device
}

/// Applies the bootloader codes, as Dolphin's code handler does before the game gets far.
pub fn apply_bootloader(ctx: &Ctx) {
    gecko::apply(ctx, &gecko::parse_list(BOOTLOADER), BOOTLOADER_LIST);
}

/// `ExiTransferBuffer(buffer, length, write)`, answered by the device directly.
fn exi_transfer_buffer(ctx: &Ctx) {
    let (buf, len, write) = (ctx.regs.r(3), ctx.regs.r(4) as usize, ctx.regs.r(5));
    let device = ctx.ext::<Device>();
    if write == 1 {
        let mut data = vec![0; len];
        for (i, b) in data.iter_mut().enumerate() {
            *b = ctx.read_u8(buf + i as u32);
        }
        device.dma_write(&data);
    } else if let Some(data) = device.dma_read(len) {
        for (i, b) in data.iter().enumerate() {
            ctx.write_u8(buf + i as u32, *b);
        }
    }
}
