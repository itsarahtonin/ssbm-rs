// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer's TextureEncoder (GPL-2.0-or-later).

//! Copies from the EFB to textures: the copied rectangle in a texture format's tiled layout,
//! as the copy writes it to memory.

use ssbm_gx::State;

use crate::{EFB_HEIGHT, EFB_WIDTH, bits};

/// A copy's bytes, by row of blocks: each at `address + row * stride`.
pub(crate) struct Encoded {
    pub address: u32,
    pub stride: u32,
    pub rows: Vec<Vec<u8>>,
}

fn intensity(r: u32, g: u32, b: u32) -> u32 {
    ((4096 + 66 * r + 129 * g + 25 * b) as u16 >> 8) as u32
}

/// Encodes the copy BP 0x52 value `value` asks for from `efb`, the EFB's color read back as
/// RGBA rows of 640. None for a format not encoded.
pub(crate) fn encode(state: &State, value: u32, efb: &[u8]) -> Option<Encoded> {
    let pixel_format = bits(state.bp[0x43], 0, 3);
    if pixel_format > 1 {
        return None;
    }
    let six = pixel_format == 1;
    let tp = bits(value, 3, 4);
    let format = tp / 2 + (tp & 1) * 8;
    let half = bits(value, 9, 1);
    let yuv = bits(value, 15, 1) != 0 && bits(value, 16, 1) != 0;
    let tl = state.bp[0x49];
    let wh = state.bp[0x4A];
    let (left, top) = (bits(tl, 0, 10), bits(tl, 10, 10));
    let (width, height) = (bits(wh, 0, 10) >> half, bits(wh, 10, 10) >> half);
    // Block width and height (log 2), and bytes per block.
    let (lw, lh, block) = match format {
        0 => (3, 3, 32),
        1 | 2 | 7 | 8 | 9 | 10 => (3, 2, 32),
        3 | 4 | 5 | 11 | 12 => (2, 2, 32),
        6 => (2, 2, 64),
        _ => return None,
    };
    let s_blocks = (width >> lw) + 1;
    let t_blocks = (height >> lh) + 1;
    let (bw, bh) = (1u32 << lw, 1u32 << lh);
    let stride = bits(state.bp[0x4D], 0, 10) << 5;
    let base = (top * EFB_WIDTH + left) as usize;
    let total = (EFB_WIDTH * EFB_HEIGHT) as usize;
    let pixel = |i: usize| -> [u32; 4] {
        if i >= total {
            return [0; 4];
        }
        let p = &efb[i * 4..i * 4 + 4];
        let a = if six { u32::from(p[3]) } else { 255 };
        [u32::from(p[0]), u32::from(p[1]), u32::from(p[2]), a]
    };
    // A texel's color: its pixel, or the 2x2 box at it at half scale.
    let texel = |s: u32, t: u32| -> [u32; 4] {
        let i = base + ((t << half) * EFB_WIDTH + (s << half)) as usize;
        if half == 0 {
            return pixel(i);
        }
        let px = [
            pixel(i),
            pixel(i + 1),
            pixel(i + EFB_WIDTH as usize),
            pixel(i + EFB_WIDTH as usize + 1),
        ];
        let mut out = [0; 4];
        for k in 0..4 {
            out[k] = if six {
                // Sums of six-bit values, widened.
                let sum: u32 = px.iter().map(|p| p[k] >> 2).sum();
                (sum + (sum >> 6)) & 0xFF
            } else {
                px.iter().map(|p| p[k]).sum::<u32>() >> 2
            };
        }
        if !six {
            out[3] = 255;
        }
        out
    };
    let mut rows = Vec::with_capacity(t_blocks as usize);
    for tb in 0..t_blocks {
        let mut row = vec![0u8; (s_blocks * block) as usize];
        for sb in 0..s_blocks {
            let dst = &mut row[(sb * block) as usize..((sb + 1) * block) as usize];
            for t in 0..bh {
                for s in 0..bw {
                    let [r, g, b, a] = texel(sb * bw + s, tb * bh + t);
                    let i = (t * bw + s) as usize;
                    let x = if yuv { intensity(r, g, b) } else { r };
                    match format {
                        0 => {
                            let v = (x & 0xF0) as u8;
                            dst[i / 2] |= if i % 2 == 0 { v } else { v >> 4 };
                        }
                        1 | 8 => dst[i] = x as u8,
                        2 => dst[i] = ((a & 0xF0) | (x >> 4)) as u8,
                        3 => {
                            dst[2 * i] = a as u8;
                            dst[2 * i + 1] = x as u8;
                        }
                        4 => {
                            let v = ((r << 8) & 0xF800) | ((g << 3) & 0x07E0) | (b >> 3);
                            dst[2 * i..2 * i + 2].copy_from_slice(&(v as u16).to_be_bytes());
                        }
                        5 => {
                            let v = if a >> 5 == 7 {
                                0x8000 | ((r << 7) & 0x7C00) | ((g << 2) & 0x03E0) | (b >> 3)
                            } else {
                                (a >> 5) << 12 | (r >> 4) << 8 | (g >> 4) << 4 | (b >> 4)
                            };
                            dst[2 * i..2 * i + 2].copy_from_slice(&(v as u16).to_be_bytes());
                        }
                        6 => {
                            dst[2 * i] = a as u8;
                            dst[2 * i + 1] = r as u8;
                            dst[32 + 2 * i] = g as u8;
                            dst[33 + 2 * i] = b as u8;
                        }
                        7 => dst[i] = a as u8,
                        9 => dst[i] = g as u8,
                        10 => dst[i] = b as u8,
                        11 => {
                            dst[2 * i] = g as u8;
                            dst[2 * i + 1] = r as u8;
                        }
                        _ => {
                            dst[2 * i] = b as u8;
                            dst[2 * i + 1] = g as u8;
                        }
                    }
                }
            }
        }
        rows.push(row);
    }
    Some(Encoded {
        address: (state.bp[0x4B] & 0x00FF_FFFF) << 5,
        stride,
        rows,
    })
}
