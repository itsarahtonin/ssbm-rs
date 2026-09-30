// SPDX-License-Identifier: GPL-3.0-or-later

//! The replay oracle: compares the events a run recorded with the original replay's, frame by
//! frame. Each event is compared over the bytes both versions of the format have.

use std::collections::BTreeMap;
use std::fmt;

use crate::replay::{Event, event, parse_events};

/// Which instance of an event: command, frame, and port and follower (or an item's spawn id).
type Key = (u8, i32, u32);

/// Field names by payload offset, for reports.
fn field(command: u8, offset: usize) -> &'static str {
    let fields: &[(usize, &str)] = match command {
        event::PRE_FRAME => &[
            (0x6, "random seed"),
            (0xA, "action state"),
            (0xC, "position x"),
            (0x10, "position y"),
            (0x14, "facing"),
            (0x18, "joystick x"),
            (0x1C, "joystick y"),
            (0x20, "c-stick x"),
            (0x24, "c-stick y"),
            (0x28, "trigger"),
            (0x2C, "buttons"),
            (0x30, "physical buttons"),
            (0x32, "physical L"),
            (0x36, "physical R"),
            (0x3A, "raw joystick x"),
            (0x3B, "percent"),
            (0x3F, "raw joystick y"),
            (0x40, "raw c-stick"),
        ],
        event::POST_FRAME => &[
            (0x6, "character"),
            (0x7, "action state"),
            (0x9, "position x"),
            (0xD, "position y"),
            (0x11, "facing"),
            (0x15, "percent"),
            (0x19, "shield"),
            (0x1D, "last attack landed"),
            (0x1E, "combo count"),
            (0x1F, "last hit by"),
            (0x20, "stocks"),
            (0x21, "action frame"),
            (0x25, "state flags"),
            (0x2A, "hitstun"),
            (0x2E, "airborne"),
            (0x2F, "ground"),
            (0x31, "jumps"),
            (0x32, "l-cancel"),
            (0x33, "hurtbox"),
            (0x34, "self air x speed"),
            (0x38, "self y speed"),
            (0x3C, "attack x speed"),
            (0x40, "attack y speed"),
            (0x44, "self ground x speed"),
            (0x48, "hitlag"),
            (0x4C, "animation"),
            (0x50, "hit by instance"),
            (0x52, "instance"),
        ],
        event::FRAME_START => &[(0x4, "random seed"), (0x8, "scene frame")],
        event::ITEM => &[
            (0x4, "type"),
            (0x6, "state"),
            (0x7, "facing"),
            (0xB, "velocity"),
            (0x13, "position"),
            (0x1B, "damage"),
            (0x1D, "timer"),
            (0x21, "spawn id"),
            (0x25, "misc"),
            (0x29, "owner"),
            (0x2A, "instance"),
        ],
        _ => &[],
    };
    fields
        .iter()
        .rev()
        .find(|(at, _)| *at <= offset)
        .map_or("header", |(_, name)| name)
}

/// Bytes that are not game state a replay can reproduce:
/// - pre-frame physical buttons and triggers: playback injects the processed inputs, so these
///   hold whatever controller is plugged in during playback;
/// - item misc bytes: item-specific scratch that most items never write, so they hold heap
///   leftovers from before the match (menus, matchmaking) that no replay records;
/// - frame end's latest finalized frame: online rollback bookkeeping.
fn ignored(command: u8, offset: usize) -> bool {
    match command {
        event::PRE_FRAME => (0x30..0x3A).contains(&offset),
        event::ITEM => (0x25..0x29).contains(&offset),
        event::FRAME_BOOKEND => offset >= 4,
        _ => false,
    }
}

fn event_name(command: u8) -> &'static str {
    match command {
        event::PRE_FRAME => "pre-frame",
        event::POST_FRAME => "post-frame",
        event::FRAME_START => "frame start",
        event::ITEM => "item",
        event::FRAME_BOOKEND => "frame end",
        event::GAME_END => "game end",
        _ => "event",
    }
}

/// The events of the timeline the game settled on, by key. Online games roll back: a frame
/// start for a frame already seen means the game simulated again from there, so whatever was
/// recorded for that frame and later belongs to a discarded timeline.
fn keyed(events: &[Event]) -> BTreeMap<Key, &[u8]> {
    let mut out = BTreeMap::new();
    let mut latest = i32::MIN;
    for e in events {
        let p = &e.payload;
        let frame = |at: usize| i32::from_be_bytes(p[at..at + 4].try_into().unwrap());
        let key = match e.command {
            event::PRE_FRAME | event::POST_FRAME if p.len() >= 6 => {
                (e.command, frame(0), u32::from(p[4]) << 8 | u32::from(p[5]))
            }
            event::FRAME_START | event::FRAME_BOOKEND if p.len() >= 4 => (e.command, frame(0), 0),
            // Items are keyed by their spawn id within a frame.
            event::ITEM if p.len() >= 0x25 => (e.command, frame(0), frame(0x21) as u32),
            event::GAME_END => (e.command, 0, 0),
            _ => continue,
        };
        if e.command == event::FRAME_START {
            if key.1 <= latest {
                out.retain(|k: &Key, _| k.1 < key.1 || k.0 == event::GAME_END);
            }
            latest = latest.max(key.1);
        }
        out.insert(key, p.as_slice());
    }
    out
}

/// One differing field.
#[derive(Clone, Debug)]
pub struct Divergence {
    pub event: &'static str,
    pub frame: i32,
    pub port: u8,
    pub follower: bool,
    /// An item's spawn id.
    pub id: u32,
    pub field: &'static str,
    pub offset: usize,
    pub original: Vec<u8>,
    pub ours: Vec<u8>,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frame {}: {}", self.frame, self.event)?;
        match self.event {
            "pre-frame" | "post-frame" => write!(
                f,
                " port {}{}",
                self.port + 1,
                if self.follower { " follower" } else { "" }
            )?,
            "item" => write!(f, " spawn {}", self.id)?,
            _ => {}
        }
        write!(
            f,
            ": {} (+{:#x}) original {:02X?}, ours {:02X?}",
            self.field, self.offset, self.original, self.ours
        )
    }
}

/// How a run compares with the original replay.
#[derive(Debug, Default)]
pub struct Report {
    pub compared: usize,
    /// Events the original has and the run does not.
    pub missing: usize,
    /// The first few of them: event, frame and port.
    pub first_missing: Vec<(&'static str, i32, u8)>,
    /// Every divergence, in frame order.
    pub divergences: Vec<Divergence>,
    pub last_frame_compared: Option<i32>,
}

impl Report {
    pub fn first(&self) -> Option<&Divergence> {
        self.divergences.first()
    }
}

/// Compares the events a run recorded (a raw event stream) with the original replay's.
pub fn compare(original: &[Event], recorded: &[u8]) -> Report {
    let mut report = Report::default();
    let Ok(ours) = parse_events(recorded) else {
        report.missing = original.len();
        return report;
    };
    let ours = keyed(&ours);
    let theirs = keyed(original);
    let mut divergences = Vec::new();
    for (key, orig) in &theirs {
        let Some(mine) = ours.get(key) else {
            report.missing += 1;
            if report.first_missing.len() < 4 {
                report
                    .first_missing
                    .push((event_name(key.0), key.1, (key.2 >> 8) as u8));
            }
            continue;
        };
        report.compared += 1;
        if key.1 > report.last_frame_compared.unwrap_or(i32::MIN) && key.0 != event::GAME_END {
            report.last_frame_compared = Some(key.1);
        }
        let n = orig.len().min(mine.len());
        if let Some(at) = (0..n).find(|&i| orig[i] != mine[i] && !ignored(key.0, i)) {
            // Report the whole field the first differing byte is in.
            let name = field(key.0, at);
            let end = (at + 4).min(n);
            divergences.push(Divergence {
                event: event_name(key.0),
                frame: key.1,
                port: (key.2 >> 8) as u8,
                follower: key.2 & 0xFF != 0,
                id: key.2,
                field: name,
                offset: at,
                original: orig[at..end].to_vec(),
                ours: mine[at..end].to_vec(),
            });
        }
    }
    divergences.sort_by_key(|d| d.frame);
    report.divergences = divergences;
    report
}
