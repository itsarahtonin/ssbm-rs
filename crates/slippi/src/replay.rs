// SPDX-License-Identifier: GPL-3.0-or-later
// Parsing follows Slippi Dolphin's SlippiGame (GPL-2.0-or-later).

//! Slippi replays (`.slp`): the raw event stream, and the game settings and per-frame data that
//! playback feeds the game.

use std::collections::BTreeMap;

/// Event command bytes.
pub mod event {
    pub const SPLIT_MESSAGE: u8 = 0x10;
    pub const PAYLOAD_SIZES: u8 = 0x35;
    pub const GAME_START: u8 = 0x36;
    pub const PRE_FRAME: u8 = 0x37;
    pub const POST_FRAME: u8 = 0x38;
    pub const GAME_END: u8 = 0x39;
    pub const FRAME_START: u8 = 0x3A;
    pub const ITEM: u8 = 0x3B;
    pub const FRAME_BOOKEND: u8 = 0x3C;
    pub const GECKO_LIST: u8 = 0x3D;
}

pub const GAME_INFO_HEADER_WORDS: usize = 78;
pub const UCF_TOGGLE_WORDS: usize = 8;
pub const NAMETAG_HALVES: usize = 8;
pub const DISPLAY_NAME_BYTES: usize = 31;
pub const GAME_FIRST_FRAME: i32 = -123;
const SPLIT_MESSAGE_DATA_LEN: usize = 512;
const GAME_SHEIK_INTERNAL: u8 = 0x07;

/// One event: its command byte and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub command: u8,
    pub payload: Vec<u8>,
}

/// What a pre-frame update records for one character: the inputs playback injects.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerFrame {
    pub random_seed: u32,
    pub animation: u16,
    pub location_x: f32,
    pub location_y: f32,
    pub facing: f32,
    pub joystick_x: f32,
    pub joystick_y: f32,
    pub cstick_x: f32,
    pub cstick_y: f32,
    pub trigger: f32,
    pub buttons: u32,
    pub joystick_x_raw: u8,
    pub percent: f32,
    pub joystick_y_raw: u8,
    pub cstick_x_raw: u8,
    pub cstick_y_raw: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub random_seed: Option<u32>,
    /// Leaders and followers (Nana) by port.
    pub players: [Option<PlayerFrame>; 4],
    pub followers: [Option<PlayerFrame>; 4],
    /// A post-frame update arrived, so every input of the frame is known.
    pub complete: bool,
}

/// A parsed replay.
#[derive(Clone, Debug, Default)]
pub struct Replay {
    /// Every event in order, split messages joined.
    pub events: Vec<Event>,
    pub version: [u8; 4],
    pub header: Vec<u32>,
    pub random_seed: u32,
    pub ucf_toggles: [u32; UCF_TOGGLE_WORDS],
    pub nametags: [[u16; NAMETAG_HALVES]; 4],
    pub is_pal: u8,
    pub is_frozen_ps: u8,
    pub display_names: [[u8; DISPLAY_NAME_BYTES]; 4],
    /// The Gecko code list the game ran with.
    pub gecko_codes: Vec<u8>,
    pub frames: BTreeMap<i32, Frame>,
    pub game_ended: bool,
    /// Ports whose character was Sheik on the first frame.
    pub sheik_start: [bool; 4],
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("not a Slippi replay")]
    NotSlippi,
    #[error("replay is truncated")]
    Truncated,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> u8 {
        let v = self.data.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        u16::from(self.u8()) << 8 | u16::from(self.u8())
    }
    fn u32(&mut self) -> u32 {
        u32::from(self.u16()) << 16 | u32::from(self.u16())
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    /// Like Slippi's readers: past the end, fields take `default`.
    fn f32_or(&mut self, default: f32) -> f32 {
        if self.at + 4 <= self.data.len() {
            self.f32()
        } else {
            self.at += 4;
            default
        }
    }
}

/// The raw event stream inside a `.slp` file.
pub fn raw_events(file: &[u8]) -> Result<&[u8], ReplayError> {
    if file.first() == Some(&event::PAYLOAD_SIZES) {
        return Ok(file);
    }
    // UBJSON: {U\x03raw[$U#l followed by a big-endian length.
    if file.len() < 15 || file[0] != b'{' || &file[3..6] != b"raw" {
        return Err(ReplayError::NotSlippi);
    }
    let len = u32::from_be_bytes(file[11..15].try_into().unwrap()) as usize;
    let end = (15 + len).min(file.len());
    Ok(&file[15..end])
}

/// Splits a raw event stream into events, joining split messages.
pub fn parse_events(raw: &[u8]) -> Result<Vec<Event>, ReplayError> {
    if raw.first() != Some(&event::PAYLOAD_SIZES) || raw.len() < 2 {
        return Err(ReplayError::NotSlippi);
    }
    let mut sizes = [None::<usize>; 256];
    let n = usize::from(raw[1]);
    sizes[usize::from(event::PAYLOAD_SIZES)] = Some(n);
    let table = raw.get(2..1 + n).ok_or(ReplayError::Truncated)?;
    for entry in table.as_chunks::<3>().0 {
        sizes[usize::from(entry[0])] = Some(usize::from(u16::from_be_bytes([entry[1], entry[2]])));
    }
    let mut events = Vec::new();
    let mut split = Vec::new();
    let mut at = 0;
    while at < raw.len() {
        let command = raw[at];
        let Some(size) = sizes[usize::from(command)] else {
            break; // unknown command: Slippi stops here too
        };
        let Some(payload) = raw.get(at + 1..at + 1 + size) else {
            break; // a truncated last event, as in replays of games still in progress
        };
        at += 1 + size;
        if command == event::SPLIT_MESSAGE {
            let block = usize::from(u16::from_be_bytes([
                payload[SPLIT_MESSAGE_DATA_LEN],
                payload[SPLIT_MESSAGE_DATA_LEN + 1],
            ]));
            split.extend_from_slice(&payload[..block]);
            if payload[SPLIT_MESSAGE_DATA_LEN + 3] != 0 {
                events.push(Event {
                    command: payload[SPLIT_MESSAGE_DATA_LEN + 2],
                    payload: std::mem::take(&mut split),
                });
            }
            continue;
        }
        events.push(Event {
            command,
            payload: payload.to_vec(),
        });
    }
    Ok(events)
}

impl Replay {
    pub fn parse(file: &[u8]) -> Result<Self, ReplayError> {
        let events = parse_events(raw_events(file)?)?;
        let mut replay = Replay {
            header: vec![0; GAME_INFO_HEADER_WORDS],
            ..Replay::default()
        };
        for e in &events {
            let mut r = Reader {
                data: &e.payload,
                at: 0,
            };
            match e.command {
                event::GAME_START => replay.game_start(&mut r),
                event::GECKO_LIST => replay.gecko_codes = e.payload.clone(),
                event::FRAME_START => {
                    let frame = r.u32() as i32;
                    let seed = r.u32();
                    replay.frames.entry(frame).or_default().random_seed = Some(seed);
                }
                event::PRE_FRAME => {
                    let frame = r.u32() as i32;
                    let port = usize::from(r.u8() & 3);
                    let follower = r.u8() != 0;
                    let p = pre_frame(&mut r);
                    let f = replay.frames.entry(frame).or_default();
                    if follower {
                        f.followers[port] = Some(p);
                    } else {
                        f.players[port] = Some(p);
                    }
                }
                event::POST_FRAME => {
                    let frame = r.u32() as i32;
                    let port = usize::from(r.u8() & 3);
                    let _follower = r.u8();
                    if frame == GAME_FIRST_FRAME && r.u8() == GAME_SHEIK_INTERNAL {
                        replay.sheik_start[port] = true;
                    }
                    replay.frames.entry(frame).or_default().complete = true;
                }
                event::GAME_END => replay.game_ended = true,
                _ => {}
            }
        }
        replay.events = events;
        Ok(replay)
    }

    fn game_start(&mut self, r: &mut Reader) {
        for v in &mut self.version {
            *v = r.u8();
        }
        for w in &mut self.header {
            *w = r.u32();
        }
        self.random_seed = r.u32();
        let has_ucf = self.version[0] >= 1;
        for t in &mut self.ucf_toggles {
            *t = if has_ucf { r.u32() } else { 0 };
        }
        for tag in &mut self.nametags {
            for h in tag.iter_mut() {
                *h = r.u16();
            }
        }
        self.is_pal = r.u8();
        self.is_frozen_ps = r.u8();
        let _minor_scene = r.u8();
        let _major_scene = r.u8();
        for name in &mut self.display_names {
            for b in name.iter_mut() {
                *b = r.u8();
            }
        }
    }

    pub fn player_exists(&self, port: usize) -> bool {
        self.header[24 + 9 * port] >> 16 & 0xFF != 3
    }

    pub fn version_at_least(&self, major: u8, minor: u8) -> bool {
        (self.version[0], self.version[1]) >= (major, minor)
    }
}

fn pre_frame(r: &mut Reader) -> PlayerFrame {
    let mut p = PlayerFrame {
        random_seed: r.u32(),
        animation: r.u16(),
        location_x: r.f32(),
        location_y: r.f32(),
        facing: r.f32(),
        joystick_x: r.f32(),
        joystick_y: r.f32(),
        cstick_x: r.f32(),
        cstick_y: r.f32(),
        trigger: r.f32(),
        buttons: r.u32(),
        ..PlayerFrame::default()
    };
    let _physical_buttons = r.u16();
    let _l = r.f32();
    let _r = r.f32();
    p.joystick_x_raw = r.u8();
    p.percent = r.f32_or(f32::from_bits(0xFFFF_FFFF));
    p.joystick_y_raw = r.u8();
    p.cstick_x_raw = r.u8();
    p.cstick_y_raw = r.u8();
    p
}
