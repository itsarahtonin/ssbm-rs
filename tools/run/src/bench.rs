// SPDX-License-Identifier: GPL-3.0-or-later

//! Benchmark matches for melee-hd's Dolphin comparison (melee-bench). `BENCH_CODES=FILE.ini`
//! boots straight into the match its Gecko codes describe (the codes melee-bench writes for
//! Dolphin, applied the same way), with Slippi's recording codes loaded beside them by Slippi's
//! bootloader, and `BENCH_SLP=OUT.slp` saves what they record as a replay.
//!
//! Each port's controller is played by one of:
//! - `BENCH_AI=PORT[,PORT...]` (0 to 3): the game's CPU AI, at the level and CPU kind the
//!   match's settings give the port, driving a human player's controller. Each tick the AI runs
//!   for the player as if it were a CPU, with the random seed put back afterwards, and what it
//!   asked for becomes the controller's next poll: sticks scaled to the controller's range and
//!   triggers to its, so everything the fighter does comes from a controller.
//! - `BENCH_PADS=REPLAY.slp`: the controllers a replay recorded, frame by frame (the pre-frame
//!   update's raw sticks, buttons and triggers), and the recording is compared with it as a
//!   replay run's is.
//!
//! Before frame 0 (the match's "GO") the controllers stay centred, so the polls that queue up
//! while the match loads hold nothing. The run stops a second after the game ends.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use ssbm_rt::Ctx;
use ssbm_sdk::{PadStatus, Sdk};
use ssbm_slippi::gecko::{self, Code};
use ssbm_slippi::{Device, Replay};

/// Where the boot codes' list goes: the low memory Dolphin's code handler uses, below the
/// bootloader's list.
const CODES_LIST: u32 = 0x8000_1800;
/// The scene controller: major scene, then (at +3) minor scene.
const SCENE: u32 = 0x8047_9D30;
/// Frames the scene has run (Slippi's IncrementFrameIndex reads it).
const SCENE_FRAMES: u32 = 0x8047_9D58;
/// Slippi's frame index, `frameIndex(r13)`.
const FRAME_INDEX: u32 = 0x804D_B6A0 - 0x49AC;
/// The match's scene: DebugMelee's game (SCENE_PLAYBACK_IN_GAME).
const MATCH_MAJOR: u8 = 0x0E;
const MATCH_MINOR: u8 = 1;
const FIRST_FRAME: i32 = -123;
/// How many polls HSD's controller queue holds (`HSD_PadLibData.qcount`).
const PAD_QUEUE_COUNT: u32 = 0x804C_1F7B;
const RANDOM_SEED: u32 = 0x804D_5F90;
const PLAYER_SLOTS: u32 = 0x8045_3080;
const PLAYER_SLOT_SIZE: u32 = 0xE90;
const PKIND_HUMAN: u32 = 0;
const PKIND_CPU: u32 = 1;
/// `Fighter_procCpu` and where its two paths meet before it returns.
const PROC_CPU: u32 = 0x8006_ABA0;
const PROC_CPU_END: u32 = 0x8006_ABD8;
/// Fighter offsets: player slot, whether it's a sub-fighter (Nana), and its `CpuFighter`.
const FP_SLOT: u32 = 0xC;
const FP_FLAGS_221F: u32 = 0x221F;
const FP_CPU: u32 = 0x1A88;
/// The buttons a controller has, without Start (the AI never pauses).
const PAD_BUTTONS: u16 = 0x0F7F;

/// Injection addresses of the Common codes from Slippi's playback set that the recording codes
/// rely on (the scene buffer, followers, the frame index and the compatibility hooks) and the
/// two that zero memory the game reads uninitialized, which Slippi tags as affecting gameplay.
const COMMON: &[u32] = &[
    0x801A_4CB4, // AllocSceneBuffer
    0x8000_55F8, // GetIsFollower
    0x8016_D294, // IncrementFrameIndex
    0x801C_154C, // Init Stage Data
    0x8006_8EEC, // Init Player Data
    0x8000_569C, // GetFighterNum
    0x8000_56A0, // GetSSMIndex
    0x8000_56A8, // RequestSSMLoad
];

pub struct Bench {
    codes: Vec<Code>,
    pub device: Rc<Device>,
    out: Option<String>,
    /// The replay whose controllers BENCH_PADS plays, to compare the recording with.
    pub pads_replay: Option<Replay>,
}

/// Sets up a benchmark match run if `BENCH_CODES` names one. Call `apply` once the game is
/// loaded.
pub fn install(ctx: &Ctx, sdk: &Sdk) -> Option<Rc<Bench>> {
    let path = std::env::var("BENCH_CODES").ok()?;
    let ini = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let codes = gecko::parse_ini(&ini);
    eprintln!("bench: {} codes from {path}", codes.len());
    let device = ctx.set_ext(Device::new(
        Replay::default(),
        gecko::gct(&recording_codes()),
    ));
    ctx.register(ssbm_slippi::EXI_TRANSFER_BUFFER, exi_transfer_buffer);
    ctx.mark_external(ssbm_slippi::EXI_TRANSFER_BUFFER);

    let ai: Vec<usize> = std::env::var("BENCH_AI")
        .map(|v| {
            v.split(',')
                .map(|p| p.trim().parse().expect("BENCH_AI=PORT[,PORT...]"))
                .collect()
        })
        .unwrap_or_default();
    let pads_replay = std::env::var("BENCH_PADS").ok().map(|p| {
        let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
        Replay::parse(&bytes).unwrap_or_else(|e| panic!("{p}: {e}"))
    });
    let next = Rc::new(RefCell::new([PadStatus::default(); 4]));
    if !ai.is_empty() {
        install_ai(ctx, &ai, next.clone());
    }
    let recorded = pads_replay.as_ref().map(pads_by_frame);
    sdk.dev.tap_pads(Box::new(move |ctx, pads| {
        let frame = frame_polled(ctx);
        for p in pads.iter_mut() {
            *p = PadStatus {
                connected: true,
                ..PadStatus::default()
            };
        }
        if frame < 0 {
            return;
        }
        for &port in &ai {
            pads[port] = next.borrow()[port];
        }
        if let Some(rec) = &recorded
            && let Some(frame_pads) = rec.get(&frame)
        {
            for (port, pad) in frame_pads.iter().enumerate() {
                if let Some(pad) = pad {
                    pads[port] = *pad;
                }
            }
        }
    }));
    Some(Rc::new(Bench {
        codes,
        device,
        out: std::env::var("BENCH_SLP").ok(),
        pads_replay,
    }))
}

impl Bench {
    /// Applies the boot codes, as Dolphin's code handler would before the game reaches them,
    /// and Slippi's bootloader, which loads the recording codes.
    pub fn apply(&self, ctx: &Ctx) {
        gecko::apply(ctx, &self.codes, CODES_LIST);
        ssbm_slippi::apply_bootloader(ctx);
    }

    /// Writes what the recording codes sent as a replay, if BENCH_SLP asks.
    pub fn save(&self) {
        let Some(path) = &self.out else { return };
        let raw = self.device.recorded.borrow();
        std::fs::write(path, slp(&raw)).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("bench: wrote {path} ({} bytes of events)", raw.len());
    }
}

/// The match frame whose engine tick the poll being taken now goes to, or -1 before the match:
/// HSD's queue hands each tick the oldest poll, so it is the next tick after the polls queued
/// before it.
fn frame_polled(ctx: &Ctx) -> i32 {
    let in_match = ctx.read_u8(SCENE) == MATCH_MAJOR && ctx.read_u8(SCENE + 3) == MATCH_MINOR;
    if !in_match {
        return -1;
    }
    let next = if ctx.read_u32(SCENE_FRAMES) == 0 {
        FIRST_FRAME
    } else {
        ctx.read_u32(FRAME_INDEX) as i32 + 1
    };
    next + i32::from(ctx.read_u8(PAD_QUEUE_COUNT))
}

/// The CPU AI plays `ports`' human players: around `Fighter_procCpu` the player counts as a CPU
/// and the random seed is kept, and what the AI asked for becomes the port's next poll.
fn install_ai(ctx: &Ctx, ports: &[usize], next: Rc<RefCell<[PadStatus; 4]>>) {
    let ports: Rc<Vec<usize>> = Rc::new(ports.to_vec());
    let saved = Rc::new(Cell::new(None::<(u32, u32)>));
    let (entry_ports, entry_saved) = (ports.clone(), saved.clone());
    ctx.set_hook(
        PROC_CPU,
        Rc::new(move |ctx| {
            let fp = ctx.read_u32(ctx.regs.r(3) + 0x2C);
            let slot = ctx.read_u8(fp + FP_SLOT) as usize;
            let sub = ctx.read_u8(fp + FP_FLAGS_221F) >> 3 & 1 != 0;
            if sub || !entry_ports.contains(&slot) {
                return;
            }
            let pkind = PLAYER_SLOTS + PLAYER_SLOT_SIZE * slot as u32 + 8;
            entry_saved.set(Some((fp, ctx.read_u32(RANDOM_SEED))));
            ctx.write_u32(pkind, PKIND_CPU);
        }),
    );
    ctx.set_hook(
        PROC_CPU_END,
        Rc::new(move |ctx| {
            let Some((fp, seed)) = saved.take() else {
                return;
            };
            let slot = ctx.read_u8(fp + FP_SLOT) as usize;
            ctx.write_u32(
                PLAYER_SLOTS + PLAYER_SLOT_SIZE * slot as u32 + 8,
                PKIND_HUMAN,
            );
            ctx.write_u32(RANDOM_SEED, seed);
            let cpu = fp + FP_CPU;
            let buttons = ctx.read_u32(cpu) as u16 & PAD_BUTTONS;
            let stick = |at: u32| scale_stick(ctx.read_u8(cpu + at) as i8);
            let trigger = |at: u32| scale_trigger(ctx.read_u8(cpu + at));
            next.borrow_mut()[slot] = PadStatus {
                connected: true,
                button: buttons,
                stick_x: stick(4),
                stick_y: stick(5),
                substick_x: stick(6),
                substick_y: stick(7),
                trigger_l: trigger(8),
                trigger_r: trigger(9),
                analog_a: 0,
                analog_b: 0,
            };
        }),
    );
}

/// A CPU stick axis (the AI's -128 to 127, which it reads as v/127 or v/128) on a controller,
/// whose axis Melee clamps to 80 and reads as r/80.
fn scale_stick(v: i8) -> i8 {
    let scale = if v > 0 { 80.0 / 127.0 } else { 80.0 / 128.0 };
    (f64::from(v) * scale).round().clamp(-80.0, 80.0) as i8
}

/// A CPU trigger (the AI's 0 to 255, read as t/255) on a controller, whose trigger Melee clamps
/// to 140 and reads as a/140.
fn scale_trigger(t: u8) -> u8 {
    (f64::from(t) * 140.0 / 255.0).round() as u8
}

/// Each frame's controllers as a replay's pre-frame updates recorded them: the raw sticks the
/// game polled, the buttons (`HSD_PadMasterStatus`) and the triggers it read, which Melee reads
/// as raw/140.
fn pads_by_frame(replay: &Replay) -> BTreeMap<i32, [Option<PadStatus>; 4]> {
    let mut out: BTreeMap<i32, [Option<PadStatus>; 4]> = BTreeMap::new();
    for e in &replay.events {
        if e.command != ssbm_slippi::replay::event::PRE_FRAME || e.payload.len() < 0x42 {
            continue;
        }
        let p = &e.payload;
        let frame = i32::from_be_bytes(p[0..4].try_into().unwrap());
        let (port, follower) = (usize::from(p[4] & 3), p[5] != 0);
        if follower {
            continue;
        }
        let f32_at = |at: usize| f32::from_be_bytes(p[at..at + 4].try_into().unwrap());
        let trigger = |v: f32| (v * 140.0).round().clamp(0.0, 255.0) as u8;
        out.entry(frame).or_default()[port] = Some(PadStatus {
            connected: true,
            button: u16::from_be_bytes([p[0x30], p[0x31]]) & 0x1F7F,
            stick_x: p[0x3A] as i8,
            stick_y: p[0x3F] as i8,
            substick_x: p[0x40] as i8,
            substick_y: p[0x41] as i8,
            trigger_l: trigger(f32_at(0x32)),
            trigger_r: trigger(f32_at(0x36)),
            analog_a: 0,
            analog_b: 0,
        });
    }
    out
}

/// The codes the bootloader loads: the Common codes recording relies on and Slippi's recording
/// codes, from Slippi's playback set.
fn recording_codes() -> Vec<Code> {
    let all = gecko::parse_ini(ssbm_slippi::PLAYBACK_INI);
    let mut common = Code {
        name: "Common".into(),
        lines: Vec::new(),
    };
    for code in &all {
        if code.name == "Required: Slippi Playback" {
            let mut i = 0;
            while i < code.lines.len() {
                let (a, d) = code.lines[i];
                let n = match a >> 24 & 0xFE {
                    0xC0 | 0xC2 => d as usize,
                    0x06 => (d as usize).div_ceil(8),
                    0x08 => 1,
                    _ => 0,
                };
                let addr = 0x8000_0000 | (a & 0x01FF_FFFF);
                if COMMON.contains(&addr) {
                    common.lines.extend_from_slice(&code.lines[i..i + 1 + n]);
                }
                i += 1 + n;
            }
        }
    }
    let recording = all
        .into_iter()
        .find(|c| c.name == "Recommended: Slippi Recording")
        .expect("Slippi's recording codes");
    vec![common, recording]
}

/// `ExiTransferBuffer(buffer, length, write)`, answered by the device directly.
fn exi_transfer_buffer(ctx: &Ctx) {
    let (buf, len, write) = (ctx.regs.r(3), ctx.regs.r(4) as usize, ctx.regs.r(5));
    let device = ctx.ext::<Device>();
    if write == 1 {
        let data: Vec<u8> = (0..len).map(|i| ctx.read_u8(buf + i as u32)).collect();
        device.dma_write(&data);
    } else if let Some(data) = device.dma_read(len) {
        for (i, b) in data.iter().enumerate() {
            ctx.write_u8(buf + i as u32, *b);
        }
    }
}

/// A `.slp` file holding the raw event stream `raw`: UBJSON `{"raw": [bytes], "metadata": {}}`.
fn slp(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 64);
    out.extend_from_slice(b"{U\x03raw[$U#l");
    out.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    out.extend_from_slice(raw);
    out.extend_from_slice(b"U\x08metadata{U\x08playedOnSU\x07ssbm-rs}}");
    out
}
