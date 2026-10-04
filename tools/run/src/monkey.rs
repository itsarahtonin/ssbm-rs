// SPDX-License-Identifier: GPL-3.0-or-later

//! Random controller input on every port, for exploring the game under lockstep: menus, modes
//! and matches the replays never reach. Deterministic for a seed.

use std::cell::Cell;
use std::rc::Rc;

use ssbm_sdk::{PadStatus, Sdk, hw};

// PAD button bits.
const LEFT: u16 = 0x0001;
const RIGHT: u16 = 0x0002;
const DOWN: u16 = 0x0004;
const UP: u16 = 0x0008;
const Z: u16 = 0x0010;
const R: u16 = 0x0020;
const L: u16 = 0x0040;
const A: u16 = 0x0100;
const B: u16 = 0x0200;
const X: u16 = 0x0400;
const Y: u16 = 0x0800;
const START: u16 = 0x1000;

/// xorshift64*: small, and the same everywhere.
pub struct Rng(Cell<u64>);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(Cell::new(seed.max(1)))
    }

    fn next(&self) -> u64 {
        let mut x = self.0.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0.set(x);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Whether an event of probability `p` percent happens.
    pub fn chance(&self, p: u64) -> bool {
        self.next() % 100 < p
    }

    /// A number below `n`.
    pub fn below(&self, n: u64) -> u64 {
        self.next() % n
    }

    fn stick(&self) -> i8 {
        match self.next() % 4 {
            0 => 0,
            1 => 127,
            2 => -128,
            _ => (self.next() % 256) as u8 as i8,
        }
    }
}

/// A new random state for one controller, B pressed with probability `b` percent.
fn press(rng: &Rng, b: u64) -> PadStatus {
    let mut button = 0;
    for (bit, p) in [
        (A, 30),
        (B, b),
        (X, 12),
        (Y, 12),
        (Z, 5),
        (L, 8),
        (R, 8),
        (LEFT, 4),
        (RIGHT, 4),
        (UP, 4),
        (DOWN, 4),
        (START, 2),
    ] {
        if rng.chance(p) {
            button |= bit;
        }
    }
    let (stick_x, stick_y) = if rng.chance(50) {
        (0, 0)
    } else {
        (rng.stick(), rng.stick())
    };
    let (substick_x, substick_y) = if rng.chance(20) {
        (rng.stick(), rng.stick())
    } else {
        (0, 0)
    };
    PadStatus {
        connected: true,
        button,
        stick_x,
        stick_y,
        substick_x,
        substick_y,
        trigger_l: if button & L != 0 { 255 } else { 0 },
        trigger_r: if button & R != 0 { 255 } else { 0 },
        analog_a: 0,
        analog_b: 0,
    }
}

/// Buttons port 1 holds, alone and with its sticks centered, over a range of fields.
#[derive(Clone, Copy)]
struct Hold {
    button: u16,
    from: u64,
    to: u64,
}

/// MONKEY_HOLD=BUTTONS@FROM-TO[,...], buttons named as a+b+x+y+z+l+r+start+up+down+left+right,
/// or none.
fn holds() -> Vec<Hold> {
    let Ok(v) = std::env::var("MONKEY_HOLD") else { return Vec::new() };
    v.split(',').map(parse_hold).collect()
}

fn parse_hold(v: &str) -> Hold {
    let (names, range) = v.split_once('@').expect("MONKEY_HOLD=BUTTONS@FROM-TO");
    let (from, to) = range.split_once('-').expect("MONKEY_HOLD=BUTTONS@FROM-TO");
    let mut button = 0;
    for name in names.split('+') {
        button |= match name {
            "none" => 0,
            "a" => A,
            "b" => B,
            "x" => X,
            "y" => Y,
            "z" => Z,
            "l" => L,
            "r" => R,
            "start" => START,
            "up" => UP,
            "down" => DOWN,
            "left" => LEFT,
            "right" => RIGHT,
            _ => panic!("MONKEY_HOLD: no button {name}"),
        };
    }
    Hold { button, from: from.parse().expect("FROM"), to: to.parse().expect("TO") }
}

/// Changes each controller's input every few fields, from field `from` on. MONKEY_B=N presses B
/// with probability N percent (20) instead: lower, input stays longer in the screens B backs out
/// of, such as a trophy being viewed or a character select screen. MONKEY_HOLD=BUTTONS@FROM-TO
/// has port 1 hold just those buttons over those fields, as a scene that reads a held button
/// as it starts needs (the trophy gallery's debug viewer: Z, at debug level 3 or more).
pub fn install(sdk: &Rc<Sdk>, seed: u64, from: u64) {
    let rng = Rc::new(Rng::new(seed));
    let b = std::env::var("MONKEY_B").map_or(20, |v| v.parse().expect("MONKEY_B=PERCENT"));
    schedule(sdk, rng, b, Rc::new(holds()), from);
}

fn schedule(sdk: &Rc<Sdk>, rng: Rc<Rng>, b: u64, holds: Rc<Vec<Hold>>, field: u64) {
    sdk.schedule(hw::field_start(field), move |ctx| {
        let sdk = ctx.ext::<Sdk>();
        {
            let mut pads = sdk.dev.pads.borrow_mut();
            for pad in pads.iter_mut() {
                if !pad.connected || rng.chance(15) {
                    *pad = press(&rng, b);
                }
            }
            if let Some(h) = holds.iter().find(|h| (h.from..=h.to).contains(&field)) {
                for pad in &mut pads[1..] {
                    pad.button = 0;
                    (pad.stick_x, pad.stick_y, pad.substick_x, pad.substick_y) = (0, 0, 0, 0);
                    (pad.trigger_l, pad.trigger_r, pad.analog_a, pad.analog_b) = (0, 0, 0, 0);
                }
                pads[0] = PadStatus {
                    connected: true,
                    button: h.button,
                    stick_x: 0,
                    stick_y: 0,
                    substick_x: 0,
                    substick_y: 0,
                    trigger_l: if h.button & L != 0 { 255 } else { 0 },
                    trigger_r: if h.button & R != 0 { 255 } else { 0 },
                    analog_a: 0,
                    analog_b: 0,
                };
            }
        }
        schedule(&sdk, rng, b, holds, field + 1);
    });
}
