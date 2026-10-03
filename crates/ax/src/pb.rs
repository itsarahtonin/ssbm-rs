// SPDX-License-Identifier: GPL-3.0-or-later
// The layout follows Dolphin's AXPB (AXStructs.h), GPL-2.0-or-later.

//! A voice's parameter block, as 16-bit words in Dolphin's AXPB layout. Updates and the mixer
//! index it so, even for the early microcode, whose PBs lack the low-pass filter in memory.

use crate::Bus;

pub const NEXT_PB: usize = 0;
pub const SRC_TYPE: usize = 4;
pub const COEF_SELECT: usize = 5;
pub const MIXER_CONTROL: usize = 6;
pub const RUNNING: usize = 7;
pub const IS_STREAM: usize = 8;
/// Volume and delta per bus.
pub const MIX_MAIN_L: usize = 9;
pub const MIX_MAIN_R: usize = 11;
pub const MIX_AUXA_L: usize = 13;
pub const MIX_AUXA_R: usize = 15;
pub const MIX_AUXB_L: usize = 17;
pub const MIX_AUXB_R: usize = 19;
pub const MIX_AUXB_S: usize = 21;
pub const MIX_MAIN_S: usize = 23;
pub const MIX_AUXA_S: usize = 25;
/// Updates: five counts (one per millisecond), then their address.
pub const UPDATES_NUM: usize = 34;
pub const UPDATES_DATA: usize = 39;
/// The last sample mixed into each bus.
pub const DPOP_MAIN_L: usize = 41;
pub const DPOP_AUXA_L: usize = 42;
pub const DPOP_AUXB_L: usize = 43;
pub const DPOP_MAIN_R: usize = 44;
pub const DPOP_AUXA_R: usize = 45;
pub const DPOP_AUXB_R: usize = 46;
pub const DPOP_MAIN_S: usize = 47;
pub const DPOP_AUXA_S: usize = 48;
pub const DPOP_AUXB_S: usize = 49;
/// Current volume, and its delta per sample.
pub const VOL_ENV: usize = 50;
pub const LOOPING: usize = 55;
pub const SAMPLE_FORMAT: usize = 56;
pub const LOOP_ADDR: usize = 57;
pub const END_ADDR: usize = 59;
pub const CUR_ADDR: usize = 61;
pub const ADPCM_COEFS: usize = 63;
pub const GAIN: usize = 79;
pub const PRED_SCALE: usize = 80;
pub const YN1: usize = 81;
pub const YN2: usize = 82;
pub const SRC_RATIO: usize = 83;
pub const SRC_FRAC: usize = 85;
pub const SRC_LAST: usize = 86;
pub const LOOP_PRED_SCALE: usize = 90;
pub const LOOP_YN1: usize = 91;
pub const LOOP_YN2: usize = 92;
/// On, yn1, a0, b0.
pub const LPF: usize = 93;
pub const LOOP_COUNTER: usize = 97;
pub const WORDS: usize = 122;

/// A parameter block.
#[derive(Clone)]
pub struct Pb {
    pub w: [u16; WORDS],
}

impl Pb {
    /// Reads the PB at `addr`; without `has_lpf`, memory skips the low-pass filter's words.
    pub fn read(bus: &dyn Bus, addr: u32, has_lpf: bool) -> Self {
        let mut w = [0u16; WORDS];
        let words = |addr: u32, out: &mut [u16]| {
            let mut b = vec![0; out.len() * 2];
            bus.read(addr, &mut b);
            for (i, o) in out.iter_mut().enumerate() {
                *o = u16::from_be_bytes([b[2 * i], b[2 * i + 1]]);
            }
        };
        if has_lpf {
            words(addr, &mut w);
        } else {
            words(addr, &mut w[..LPF]);
            words(addr + 2 * LPF as u32, &mut w[LOOP_COUNTER..]);
        }
        Pb { w }
    }

    pub fn write(&self, bus: &mut dyn Bus, addr: u32, has_lpf: bool) {
        let bytes = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        if has_lpf {
            bus.write(addr, &bytes(&self.w));
        } else {
            bus.write(addr, &bytes(&self.w[..LPF]));
            bus.write(addr + 2 * LPF as u32, &bytes(&self.w[LOOP_COUNTER..]));
        }
    }

    /// Millisecond `ms`'s updates from `updates` (32 pairs of word offset and value).
    pub fn apply_updates(&mut self, ms: usize, updates: &[u16]) {
        let start: usize = self.w[UPDATES_NUM..UPDATES_NUM + ms]
            .iter()
            .map(|&n| usize::from(n))
            .sum();
        if start >= 32 {
            return;
        }
        let count = usize::from(self.w[UPDATES_NUM + ms]);
        if count > 32 - start {
            return;
        }
        for i in start..start + count {
            let (off, val) = (usize::from(updates[2 * i]), updates[2 * i + 1]);
            if off < WORDS {
                self.w[off] = val;
            }
        }
    }
}
