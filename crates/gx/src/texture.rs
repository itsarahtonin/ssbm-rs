// SPDX-License-Identifier: GPL-3.0-or-later
// Formats follow Dolphin's TextureDecoder (GPL-2.0-or-later).

//! Texture registers and the memory a texture's data takes: GameCube textures are stored in
//! tiles (blocks) of 32 bytes, 64 for RGBA8, whose dimensions depend on the format.

use crate::bits;

pub const SETMODE0: usize = 0x80;
pub const SETMODE1: usize = 0x84;
pub const SETIMAGE0: usize = 0x88;
pub const SETIMAGE3: usize = 0x94;
pub const SETTLUT: usize = 0x98;
/// Texture maps 4-7 have their registers 0x20 further on.
pub const SECOND_FOUR: usize = 0x20;

/// The BP register `reg` (one of the SET* bases) of texture map `map`.
pub fn reg(reg: usize, map: usize) -> usize {
    reg + (map & 3) + if map >= 4 { SECOND_FOUR } else { 0 }
}

/// Block width, height and bytes of format `format`, or None for an unknown one.
pub fn block(format: u32) -> Option<(u32, u32, u32)> {
    Some(match format {
        0 | 8 | 14 => (8, 8, 32), // I4, C4, CMPR
        1 | 2 | 9 => (8, 4, 32),  // I8, IA4, C8
        3..=5 | 10 => (4, 4, 32), // IA8, RGB565, RGB5A3, C14X2
        6 => (4, 4, 64),          // RGBA8
        _ => return None,
    })
}

/// Whether a format reads a palette (TLUT), and its entries.
pub fn palette_entries(format: u32) -> Option<u32> {
    match format {
        8 => Some(16),
        9 => Some(256),
        10 => Some(16384),
        _ => None,
    }
}

/// Bytes a `width` by `height` image of `format` takes.
pub fn level_size(format: u32, width: u32, height: u32) -> u32 {
    match block(format) {
        Some((bw, bh, bytes)) => width.div_ceil(bw) * height.div_ceil(bh) * bytes,
        None => 0,
    }
}

/// A texture map's image as its registers describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image {
    pub address: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    /// Mip levels stored, the base included.
    pub levels: u32,
}

impl Image {
    pub fn of(bp: &[u32; 256], map: usize) -> Self {
        let img0 = bp[reg(SETIMAGE0, map)];
        let mode0 = bp[reg(SETMODE0, map)];
        let mode1 = bp[reg(SETMODE1, map)];
        // A mipmapping min filter (any but near and linear) reads levels up to max LOD.
        let min_filter = bits(mode0, 5, 3);
        let mipmapped = !matches!(min_filter, 0 | 4);
        let max_lod = bits(mode1, 8, 8).div_ceil(16);
        Image {
            address: (bp[reg(SETIMAGE3, map)] & 0x00FF_FFFF) << 5,
            width: bits(img0, 0, 10) + 1,
            height: bits(img0, 10, 10) + 1,
            format: bits(img0, 20, 4),
            levels: if mipmapped { max_lod + 1 } else { 1 },
        }
    }

    /// Bytes its levels take, each level half the size of the one before.
    pub fn size(&self) -> u32 {
        let (mut w, mut h, mut total) = (self.width, self.height, 0);
        for _ in 0..self.levels.min(11) {
            total += level_size(self.format, w, h);
            if w == 1 && h == 1 {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        total
    }
}

fn read8(src: &[u8], at: usize) -> u8 {
    src.get(at).copied().unwrap_or(0)
}

/// A big-endian 16-bit value.
fn read16(src: &[u8], at: usize) -> u16 {
    u16::from(read8(src, at)) << 8 | u16::from(read8(src, at + 1))
}

fn c3(v: u16) -> u8 {
    (v << 5 | v << 2 | v >> 1) as u8
}

fn c4(v: u16) -> u8 {
    (v << 4 | v) as u8
}

fn c5(v: u16) -> u8 {
    (v << 3 | v >> 2) as u8
}

fn c6(v: u16) -> u8 {
    (v << 2 | v >> 4) as u8
}

fn ia8(v: u16) -> [u8; 4] {
    let (i, a) = ((v & 0xFF) as u8, (v >> 8) as u8);
    [i, i, i, a]
}

fn rgb565(v: u16) -> [u8; 4] {
    [c5(v >> 11 & 31), c6(v >> 5 & 63), c5(v & 31), 255]
}

fn rgb5a3(v: u16) -> [u8; 4] {
    if v & 0x8000 != 0 {
        [c5(v >> 10 & 31), c5(v >> 5 & 31), c5(v & 31), 255]
    } else {
        [
            c4(v >> 8 & 15),
            c4(v >> 4 & 15),
            c4(v & 15),
            c3(v >> 12 & 7),
        ]
    }
}

/// Palette entry `index` of a palette in format `tlut_format` (IA8, RGB565, RGB5A3).
fn paletted(tlut: &[u8], index: usize, tlut_format: u32) -> [u8; 4] {
    let v = read16(tlut, 2 * index);
    match tlut_format {
        0 => ia8(v),
        1 => rgb565(v),
        2 => rgb5a3(v),
        _ => [0; 4],
    }
}

fn dxt_blend(v1: u32, v2: u32) -> u8 {
    ((v1 * 3 + v2 * 5) >> 3) as u8
}

/// Texel (`s`, `t`) of an image of format `format` whose width less one is `width_m1`, as RGBA
/// (Dolphin's TexDecoder_DecodeTexel). `tlut` is the palette, from its start in texture memory.
pub fn texel(
    src: &[u8],
    s: u32,
    t: u32,
    width_m1: u32,
    format: u32,
    tlut: &[u8],
    tlut_format: u32,
) -> [u8; 4] {
    let (s, t) = (s as usize, t as usize);
    let w = width_m1 as usize;
    // Blocks of 8x8 4-bit texels, 8x4 8-bit ones and 4x4 16-bit ones.
    let nibble = |src: &[u8]| {
        let base = ((t >> 3) * ((w >> 3) + 1) + (s >> 3)) << 5;
        let off = ((t & 7) << 3) + (s & 7);
        let shift = if off & 1 != 0 { 0 } else { 4 };
        (read8(src, base + (off >> 1)) >> shift) & 0xF
    };
    let byte = |src: &[u8]| {
        let base = ((t >> 2) * ((w >> 3) + 1) + (s >> 3)) << 5;
        read8(src, base + ((t & 3) << 3) + (s & 7))
    };
    let half = |src: &[u8]| {
        let base = ((t >> 2) * ((w >> 2) + 1) + (s >> 2)) << 4;
        read16(src, (base + ((t & 3) << 2) + (s & 3)) << 1)
    };
    match format {
        0 => [c4(u16::from(nibble(src))); 4],
        1 => [byte(src); 4],
        2 => {
            let v = u16::from(byte(src));
            let l = c4(v & 0xF);
            [l, l, l, c4(v >> 4)]
        }
        3 => ia8(half(src)),
        4 => rgb565(half(src)),
        5 => rgb5a3(half(src)),
        6 => {
            let base = ((t >> 2) * ((w >> 2) + 1) + (s >> 2)) << 5;
            let off = (base + ((t & 3) << 2) + (s & 3)) << 1;
            [
                read8(src, off + 1),
                read8(src, off + 32),
                read8(src, off + 33),
                read8(src, off),
            ]
        }
        8 => paletted(tlut, usize::from(nibble(src)), tlut_format),
        9 => paletted(tlut, usize::from(byte(src)), tlut_format),
        10 => paletted(tlut, usize::from(half(src) & 0x3FFF), tlut_format),
        14 => {
            let (sd, td) = (s >> 2, t >> 2);
            let base = ((td >> 1) * ((w >> 3) + 1) + (sd >> 1)) << 2;
            let off = (base + ((td & 1) << 1) + (sd & 1)) << 3;
            let c1 = read16(src, off);
            let c2 = read16(src, off + 2);
            let (r1, g1, b1) = (
                u32::from(c5(c1 >> 11 & 31)),
                u32::from(c6(c1 >> 5 & 63)),
                u32::from(c5(c1 & 31)),
            );
            let (r2, g2, b2) = (
                u32::from(c5(c2 >> 11 & 31)),
                u32::from(c6(c2 >> 5 & 63)),
                u32::from(c5(c2 & 31)),
            );
            let line = read8(src, off + 4 + (t & 3));
            let sel = (line >> (6 - 2 * (s & 3))) & 3 | if c1 > c2 { 0 } else { 4 };
            match sel {
                0 | 4 => [r1 as u8, g1 as u8, b1 as u8, 255],
                1 | 5 => [r2 as u8, g2 as u8, b2 as u8, 255],
                2 => [dxt_blend(r2, r1), dxt_blend(g2, g1), dxt_blend(b2, b1), 255],
                3 => [dxt_blend(r1, r2), dxt_blend(g1, g2), dxt_blend(b1, b2), 255],
                6 => [
                    ((r1 + r2) / 2) as u8,
                    ((g1 + g2) / 2) as u8,
                    ((b1 + b2) / 2) as u8,
                    255,
                ],
                _ => [
                    ((r1 + r2) / 2) as u8,
                    ((g1 + g2) / 2) as u8,
                    ((b1 + b2) / 2) as u8,
                    0,
                ],
            }
        }
        _ => [0; 4],
    }
}

/// Texel size in nibbles.
fn nibbles(format: u32) -> u32 {
    match format {
        0 | 8 | 14 => 1,
        1 | 2 | 9 => 2,
        6 => 8,
        _ => 4,
    }
}

/// Decodes `levels` levels of an image as RGBA rows, each level `((width - 1) >> level) + 1`
/// wide, found where Dolphin's software renderer finds them.
pub fn decode(
    src: &[u8],
    width: u32,
    height: u32,
    levels: u32,
    format: u32,
    tlut: &[u8],
    tlut_format: u32,
) -> Vec<Vec<u8>> {
    let (bw, bh, _) = block(format).unwrap_or((8, 8, 32));
    let mut out = Vec::new();
    let mut offset = 0usize;
    let (mut mw, mut mh) = (width, height);
    for level in 0..levels {
        let (wm1, hm1) = ((width - 1) >> level, (height - 1) >> level);
        let data = src.get(offset..).unwrap_or(&[]);
        let mut rgba = Vec::with_capacity(((wm1 + 1) * (hm1 + 1) * 4) as usize);
        for t in 0..=hm1 {
            for s in 0..=wm1 {
                rgba.extend_from_slice(&texel(data, s, t, wm1, format, tlut, tlut_format));
            }
        }
        out.push(rgba);
        mw = mw.max(bw);
        mh = mh.max(bh);
        offset += (mw * mh * nibbles(format) / 2) as usize;
        mw >>= 1;
        mh >>= 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_count_whole_blocks() {
        assert_eq!(level_size(6, 4, 4), 64);
        assert_eq!(level_size(0, 9, 8), 64);
        assert_eq!(level_size(14, 64, 64), 2048);
    }
}
