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

/// A new random state for one controller.
fn press(rng: &Rng) -> PadStatus {
    let mut button = 0;
    for (bit, p) in [
        (A, 30),
        (B, 20),
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

/// Changes each controller's input every few fields, from field `from` on.
pub fn install(sdk: &Rc<Sdk>, seed: u64, from: u64) {
    let rng = Rc::new(Rng::new(seed));
    schedule(sdk, rng, from);
}

fn schedule(sdk: &Rc<Sdk>, rng: Rc<Rng>, field: u64) {
    sdk.schedule(hw::field_start(field), move |ctx| {
        let sdk = ctx.ext::<Sdk>();
        {
            let mut pads = sdk.dev.pads.borrow_mut();
            for pad in pads.iter_mut() {
                if !pad.connected || rng.chance(15) {
                    *pad = press(&rng);
                }
            }
        }
        schedule(&sdk, rng, field + 1);
    });
}
