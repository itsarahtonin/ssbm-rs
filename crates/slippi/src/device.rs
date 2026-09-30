// SPDX-License-Identifier: GPL-3.0-or-later
// Behavior follows Slippi Dolphin's EXI_DeviceSlippi (GPL-2.0-or-later).

//! The Slippi EXI device, for playback: it answers the playback codes' requests for game info,
//! Gecko codes and per-frame inputs, and records what the recording codes send.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::denylist::DENYLIST;
use crate::replay::{GAME_FIRST_FRAME, PlayerFrame, Replay};

pub mod cmd {
    pub const RECEIVE_COMMANDS: u8 = 0x35;
    pub const RECEIVE_GAME_END: u8 = 0x39;
    pub const FRAME_BOOKEND: u8 = 0x3C;
    pub const MENU_FRAME: u8 = 0x3E;
    pub const PREPARE_REPLAY: u8 = 0x75;
    pub const READ_FRAME: u8 = 0x76;
    pub const GET_LOCATION: u8 = 0x77;
    pub const IS_FILE_READY: u8 = 0x88;
    pub const IS_STOCK_STEAL: u8 = 0x89;
    pub const GET_GECKO_CODES: u8 = 0x8A;
    pub const LOG_MESSAGE: u8 = 0xD0;
    pub const FILE_LENGTH: u8 = 0xD1;
    pub const FILE_LOAD: u8 = 0xD2;
    pub const GCT_LENGTH: u8 = 0xD3;
    pub const GCT_LOAD: u8 = 0xD4;
}

const FRAME_RESP_WAIT: u8 = 0;
const FRAME_RESP_CONTINUE: u8 = 1;
const FRAME_RESP_TERMINATE: u8 = 2;
/// Size of one character's data in a frame response.
const CHARACTER_DATA_LEN: usize = 52;

pub struct Device {
    pub replay: Replay,
    /// The GCT the bootloader loads: the playback code set.
    gct: Vec<u8>,
    /// The replay's own codes, minus those playback leaves out.
    gecko_list: Vec<u8>,
    payload_sizes: RefCell<HashMap<u8, usize>>,
    read_queue: RefCell<Vec<u8>>,
    file_ready_sent: Cell<bool>,
    /// The raw event stream the recording codes sent, as a replay would store it.
    pub recorded: RefCell<Vec<u8>>,
    /// Frames the playback codes asked for.
    pub frames_read: Cell<u32>,
    pub terminated: Cell<bool>,
    pub log: RefCell<Vec<String>>,
}

impl Device {
    pub fn new(replay: Replay, gct: Vec<u8>) -> Self {
        let gecko_list = filter_codes(&replay.gecko_codes);
        let sizes = [
            (cmd::RECEIVE_COMMANDS, 1),
            (cmd::PREPARE_REPLAY, 0xFFFF),
            (cmd::READ_FRAME, 4),
            (cmd::IS_STOCK_STEAL, 5),
            (cmd::GET_LOCATION, 6),
            (cmd::IS_FILE_READY, 0),
            (cmd::GET_GECKO_CODES, 0),
            (cmd::LOG_MESSAGE, 0xFFFF),
            (cmd::FILE_LENGTH, 0x40),
            (cmd::FILE_LOAD, 0x40),
            (cmd::GCT_LENGTH, 0),
            (cmd::GCT_LOAD, 4),
        ];
        Self {
            replay,
            gct,
            gecko_list,
            payload_sizes: RefCell::new(sizes.into_iter().collect()),
            read_queue: RefCell::default(),
            file_ready_sent: Cell::new(false),
            recorded: RefCell::default(),
            frames_read: Cell::new(0),
            terminated: Cell::new(false),
            log: RefCell::default(),
        }
    }

    /// The game sends `data`.
    pub fn dma_write(&self, data: &[u8]) {
        let mut at = 0;
        if data.first() == Some(&cmd::RECEIVE_COMMANDS) && data.len() > 1 {
            let n = usize::from(data[1]);
            let table = &data[2..(1 + n).min(data.len())];
            let mut sizes = self.payload_sizes.borrow_mut();
            for entry in table.as_chunks::<3>().0 {
                sizes.insert(
                    entry[0],
                    usize::from(u16::from_be_bytes([entry[1], entry[2]])),
                );
            }
            drop(sizes);
            self.recorded
                .borrow_mut()
                .extend_from_slice(&data[..(n + 1).min(data.len())]);
            at = n + 1;
        }
        if data.first() == Some(&cmd::MENU_FRAME) {
            return;
        }
        while at < data.len() {
            let command = data[at];
            let Some(size) = self.payload_sizes.borrow().get(&command).copied() else {
                self.log
                    .borrow_mut()
                    .push(format!("unknown Slippi command {command:#04X}"));
                return;
            };
            let payload = &data[(at + 1).min(data.len())..(at + 1 + size).min(data.len())];
            if std::env::var_os("SLIPPI_TRACE").is_some() {
                eprintln!("slippi: command {command:#04X}, {} bytes", payload.len());
            }
            match command {
                cmd::PREPARE_REPLAY => self.prepare_game_info(),
                cmd::READ_FRAME => self.prepare_frame(word(payload, 0) as i32),
                cmd::IS_STOCK_STEAL => {
                    self.prepare_is_stock_steal(word(payload, 0) as i32, payload[4])
                }
                cmd::IS_FILE_READY => self.prepare_is_file_ready(),
                cmd::GET_GECKO_CODES => *self.read_queue.borrow_mut() = self.gecko_list.clone(),
                cmd::GCT_LENGTH => {
                    *self.read_queue.borrow_mut() = (self.gct.len() as u32).to_be_bytes().to_vec()
                }
                cmd::GCT_LOAD => *self.read_queue.borrow_mut() = self.gct.clone(),
                cmd::LOG_MESSAGE => {
                    let text: String = payload
                        .iter()
                        .take_while(|&&b| b != 0)
                        .map(|&b| char::from(b))
                        .collect();
                    self.log.borrow_mut().push(text);
                }
                cmd::GET_LOCATION | cmd::FILE_LENGTH | cmd::FILE_LOAD => {
                    self.log
                        .borrow_mut()
                        .push(format!("unsupported Slippi command {command:#04X}"));
                    self.read_queue.borrow_mut().clear();
                }
                _ => {
                    // Everything else is replay data from the recording codes.
                    let end = (at + 1 + size).min(data.len());
                    self.recorded.borrow_mut().extend_from_slice(&data[at..end]);
                    if command == cmd::RECEIVE_GAME_END {
                        self.log.borrow_mut().push("game end recorded".to_owned());
                    }
                }
            }
            at += size + 1;
        }
    }

    /// The game reads `len` bytes of the current response.
    pub fn dma_read(&self, len: usize) -> Option<Vec<u8>> {
        let queue = self.read_queue.borrow();
        if queue.is_empty() {
            return None;
        }
        let mut out = queue.clone();
        out.resize(len, 0);
        Some(out)
    }

    fn prepare_is_file_ready(&self) {
        let ready = !self.file_ready_sent.replace(true);
        *self.read_queue.borrow_mut() = vec![u8::from(ready)];
    }

    fn prepare_game_info(&self) {
        let r = &self.replay;
        let mut q = vec![1u8];
        q.extend_from_slice(&r.random_seed.to_be_bytes());
        let mut header = r.header.clone();
        for port in 0..4 {
            if !r.player_exists(port) {
                continue;
            }
            let pos = 24 + 9 * port;
            let character = header[pos] >> 24;
            // Players who start as Sheik keep Sheik; Zelda stays Zelda.
            if (character == 0x12 || character == 0x13) && r.sheik_start[port] {
                header[pos] = (header[pos] & 0x00FF_FFFF) | (0x13 << 24);
            }
        }
        for w in header {
            q.extend_from_slice(&w.to_be_bytes());
        }
        for t in r.ucf_toggles {
            q.extend_from_slice(&t.to_be_bytes());
        }
        for tag in r.nametags {
            for h in tag {
                q.extend_from_slice(&h.to_be_bytes());
            }
        }
        q.push(r.is_pal);
        q.push(u8::from(r.version_at_least(1, 3))); // preload Pokemon Stadium
        q.push(r.is_frozen_ps);
        q.push(0); // never resync: divergence is what we check
        for name in r.display_names {
            q.extend_from_slice(&name);
        }
        q.extend_from_slice(&(self.gecko_list.len() as u32).to_be_bytes());
        *self.read_queue.borrow_mut() = q;
    }

    fn prepare_frame(&self, frame_idx: i32) {
        self.frames_read.set(self.frames_read.get() + 1);
        let mut q = Vec::new();
        let r = &self.replay;
        let frame = r.frames.get(&frame_idx).filter(|f| f.complete);
        let Some(frame) = frame else {
            let code = if r.game_ended || r.frames.keys().next_back() < Some(&frame_idx) {
                self.terminated.set(true);
                FRAME_RESP_TERMINATE
            } else {
                FRAME_RESP_WAIT
            };
            *self.read_queue.borrow_mut() = vec![code];
            return;
        };
        q.push(FRAME_RESP_CONTINUE);
        q.push(0); // not a rollback
        // Online games set the seed from the frame number at the start of every frame, with
        // codes playback does not run, so the frame's starting seed is input like the pads'.
        // Seeds within the frame (pre-frame updates) still check the game's own RNG use.
        q.push(u8::from(frame.random_seed.is_some()));
        q.extend_from_slice(&frame.random_seed.unwrap_or(0).to_be_bytes());
        for port in 0..4 {
            character_data(&mut q, frame.players[port].as_ref());
            character_data(&mut q, frame.followers[port].as_ref());
        }
        *self.read_queue.borrow_mut() = q;
    }

    fn prepare_is_stock_steal(&self, frame_idx: i32, port: u8) {
        let back = self
            .replay
            .frames
            .get(&frame_idx)
            .is_some_and(|f| f.players[usize::from(port & 3)].is_some());
        *self.read_queue.borrow_mut() = vec![u8::from(back)];
    }

    /// The first frame the replay has data for.
    pub fn first_frame(&self) -> i32 {
        self.replay
            .frames
            .keys()
            .next()
            .copied()
            .unwrap_or(GAME_FIRST_FRAME)
    }
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn character_data(q: &mut Vec<u8>, p: Option<&PlayerFrame>) {
    let Some(p) = p else {
        q.extend_from_slice(&[0; CHARACTER_DATA_LEN]);
        return;
    };
    for w in [
        p.random_seed,
        p.joystick_x.to_bits(),
        p.joystick_y.to_bits(),
        p.cstick_x.to_bits(),
        p.cstick_y.to_bits(),
        p.trigger.to_bits(),
        p.buttons,
        p.location_x.to_bits(),
        p.location_y.to_bits(),
        p.facing.to_bits(),
        u32::from(p.animation),
    ] {
        q.extend_from_slice(&w.to_be_bytes());
    }
    q.push(p.joystick_x_raw);
    q.push(p.joystick_y_raw);
    q.extend_from_slice(&p.percent.to_bits().to_be_bytes());
    q.push(p.cstick_x_raw);
    q.push(p.cstick_y_raw);
}

/// The replay's Gecko code list without the codes playback leaves out, as
/// `CEXISlippi::prepareGeckoList` builds it.
fn filter_codes(source: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= source.len() {
        let kind = source[at] & 0xFE;
        let address = (word(source, at) & 0x01FF_FFFF) | 0x8000_0000;
        let len = match kind {
            0xC0 | 0xC2 => 8 + 8 * word(source, at + 4) as usize,
            0x08 => 16,
            0x06 => 8 + ((word(source, at + 4) as usize + 7) & !7),
            _ => 8,
        };
        let end = (at + len).min(source.len());
        if DENYLIST.binary_search(&address).is_err() {
            out.extend_from_slice(&source[at..end]);
        }
        at += len;
    }
    out.extend_from_slice(&[0xFF, 0, 0, 0, 0, 0, 0, 0]);
    out
}
