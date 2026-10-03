// SPDX-License-Identifier: GPL-3.0-or-later
// Register layouts follow Dolphin's BPMemory.h (GPL-2.0-or-later).

//! The TEV (texture environment) combiners: which components of its color and konst registers
//! a draw's stages read. Every pixel starts the stages with the color registers as BP holds
//! them; a component a stage writes before any stage reads it, or that no stage reads, doesn't
//! decide what is drawn.

use crate::bits;

pub const COLOR_ENV: usize = 0xC0;
pub const ALPHA_ENV: usize = 0xC1;
pub const KSEL: usize = 0xF6;

/// Component masks.
pub const R: u8 = 1;
pub const G: u8 = 2;
pub const B: u8 = 4;
pub const A: u8 = 8;
pub const RGB: u8 = R | G | B;

/// Components of the four color registers (PREV, C0, C1, C2) and the four konst registers that
/// the active stages read from their initial values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub color: [u8; 4],
    pub konst: [u8; 4],
}

/// The konst register and components a konst selection reads (none for the fixed fractions).
fn konst_sel(sel: u32, alpha: bool) -> Option<(usize, u8)> {
    let reg = (sel & 3) as usize;
    match sel {
        0x0C..=0x0F if !alpha => Some((reg, RGB)),
        0x10..=0x13 => Some((reg, R)),
        0x14..=0x17 => Some((reg, G)),
        0x18..=0x1B => Some((reg, B)),
        0x1C..=0x1F => Some((reg, A)),
        _ => None,
    }
}

/// What the active stages read, from GENMODE's stage count and their combiner settings.
pub fn usage(bp: &[u32; 256]) -> Usage {
    let stages = bits(bp[0x00], 10, 4) as usize + 1;
    let mut written = [0u8; 4];
    let mut out = Usage::default();
    let read = |out: &mut Usage, written: &[u8; 4], reg: usize, comps: u8| {
        out.color[reg] |= comps & !written[reg];
    };
    for s in 0..stages {
        let cc = bp[COLOR_ENV + 2 * s];
        let ac = bp[ALPHA_ENV + 2 * s];
        let ksel = bp[KSEL + s / 2];
        let (kcsel, kasel) = if s % 2 == 0 {
            (bits(ksel, 4, 5), bits(ksel, 9, 5))
        } else {
            (bits(ksel, 14, 5), bits(ksel, 19, 5))
        };
        // Color inputs a, b, c, d.
        for input in [
            bits(cc, 12, 4),
            bits(cc, 8, 4),
            bits(cc, 4, 4),
            bits(cc, 0, 4),
        ] {
            match input {
                0..=7 => {
                    let reg = (input / 2) as usize;
                    read(
                        &mut out,
                        &written,
                        reg,
                        if input % 2 == 0 { RGB } else { A },
                    );
                }
                14 => {
                    if let Some((reg, comps)) = konst_sel(kcsel, false) {
                        out.konst[reg] |= comps;
                    }
                }
                _ => {}
            }
        }
        // Alpha inputs a, b, c, d.
        for input in [
            bits(ac, 13, 3),
            bits(ac, 10, 3),
            bits(ac, 7, 3),
            bits(ac, 4, 3),
        ] {
            match input {
                0..=3 => read(&mut out, &written, input as usize, A),
                6 => {
                    if let Some((reg, comps)) = konst_sel(kasel, true) {
                        out.konst[reg] |= comps;
                    }
                }
                _ => {}
            }
        }
        written[bits(cc, 22, 2) as usize] |= RGB;
        written[bits(ac, 22, 2) as usize] |= A;
    }
    out
}

/// The components `comps` of a register held as its RA and BG halves (11 bits each, red or
/// blue low, alpha or green from bit 12), the rest cleared.
pub fn masked(ra: u32, bg: u32, comps: u8) -> (u32, u32) {
    let field = |v: u32, low: bool, high: bool| {
        (if low { v & 0x7FF } else { 0 }) | (if high { v & (0x7FF << 12) } else { 0 })
    };
    (
        field(ra, comps & R != 0, comps & A != 0),
        field(bg, comps & B != 0, comps & G != 0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stage that outputs `color_in` (as d) to PREV, and alpha input `alpha_in`.
    fn stage(bp: &mut [u32; 256], s: usize, color_in: u32, alpha_in: u32) {
        // a = b = c = ZERO (15 / 7), d = the input, dest PREV.
        bp[COLOR_ENV + 2 * s] = (15 << 12) | (15 << 8) | (15 << 4) | color_in;
        bp[ALPHA_ENV + 2 * s] = (7 << 13) | (7 << 10) | (7 << 7) | (alpha_in << 4);
    }

    #[test]
    fn a_register_read_for_its_alpha_only_needs_no_rgb() {
        let mut bp = [0; 256];
        // One stage: color from the rasterized color, alpha from A0.
        stage(&mut bp, 0, 10, 1);
        let u = usage(&bp);
        assert_eq!(u.color, [0, A, 0, 0]);
    }

    #[test]
    fn a_component_written_before_it_is_read_needs_no_initial_value() {
        let mut bp = [0; 256];
        bp[0x00] = 1 << 10; // two stages
        // Stage 0 writes C0 (dest 1) from ONE; stage 1 reads C0 and A0.
        bp[COLOR_ENV] = (15 << 12) | (15 << 8) | (15 << 4) | 12 | (1 << 22);
        bp[ALPHA_ENV] = (7 << 13) | (7 << 10) | (7 << 7) | (7 << 4);
        stage(&mut bp, 1, 2, 1);
        let u = usage(&bp);
        assert_eq!(
            u.color[1], A,
            "C0's color comes from stage 0; its alpha from BP"
        );
    }

    #[test]
    fn konst_selections_read_the_components_they_name() {
        let mut bp = [0; 256];
        stage(&mut bp, 0, 14, 6);
        // Stage 0: kcsel K2 RGB (0x0E), kasel K1 alpha (0x1D).
        bp[KSEL] = (0x0E << 4) | (0x1D << 9);
        let u = usage(&bp);
        assert_eq!(u.konst, [0, A, RGB, 0]);
    }

    #[test]
    fn masking_keeps_only_the_components_read() {
        let (ra, bg) = masked(0x0FF_0B3 | (1 << 23), 0x040_055, A);
        assert_eq!((ra, bg), (0x0FF << 12, 0));
    }
}
