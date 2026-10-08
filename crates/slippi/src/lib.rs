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

/// The playback codes a run applies: the set's enabled codes, without its optional Show Player
/// Names when `SLIPPI_PLAYER_NAMES=0` (Slippi Dolphin's setting for it). That code draws the
/// players' names over the match and makes HUD objects for them, which melee-hd's oracle
/// leaves to the presentation.
pub fn playback_codes() -> Vec<gecko::Code> {
    let mut codes = gecko::parse_ini(PLAYBACK_INI);
    if std::env::var("SLIPPI_PLAYER_NAMES").is_ok_and(|v| v == "0") {
        codes.retain(|c| c.name != "Optional: Show Player Names");
    }
    codes
}

/// Puts a Slippi device holding `replay` into `ctx`. Call `apply_bootloader` once the game is
/// loaded.
pub fn install(ctx: &Ctx, replay: Replay) -> Rc<Device> {
    let gct = gecko::gct(&playback_codes());
    let device = ctx.set_ext(Device::new(replay, gct));
    ctx.register(EXI_TRANSFER_BUFFER, exi_transfer_buffer);
    ctx.mark_external(EXI_TRANSFER_BUFFER);
    device
}

/// Game addresses that the codes playback applies write to: the bootloader, the playback
/// code set and the replay's own codes. Code there differs from the game's, so a port of a
/// function containing one would not do what playback does.
pub fn patched(device: &Device) -> Vec<(u32, u32)> {
    gecko::targets(&applied_lines(device))
}

/// Addresses of the injected codes playback applies that return past the instructions after
/// the call to the function they are in: its caller does not run as the game's code does.
pub fn returns_past_caller(device: &Device) -> Vec<u32> {
    gecko::returns_past_caller(&applied_lines(device))
}

/// The codes playback applies, in order: the bootloader's, playback's own and the replay's.
pub fn applied_codes(device: &Device) -> Vec<gecko::Parsed> {
    gecko::parse_codes(&applied_lines(device))
}

fn applied_lines(device: &Device) -> Vec<(u32, u32)> {
    let mut lines: Vec<(u32, u32)> = gecko::parse_list(BOOTLOADER)
        .iter()
        .chain(playback_codes().iter())
        .flat_map(|c| c.lines.clone())
        .collect();
    lines.extend(gecko::lines(device.gecko_list()));
    lines
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
