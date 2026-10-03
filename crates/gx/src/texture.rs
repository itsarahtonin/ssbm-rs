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
