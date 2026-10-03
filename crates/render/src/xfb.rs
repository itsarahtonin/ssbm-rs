// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer's EncodeXFB and its XFB decoder (GPL-2.0-or-later).

//! Copies to the XFB: the EFB through the copy filter (anti-aliasing and deflicker), gamma, and
//! YUV 4:2:2, then back to RGB as the video interface shows it.

use ssbm_gx::State;

use crate::{EFB_WIDTH, Frame, bits};

#[derive(Clone, Copy, Default)]
struct Yuv {
    y: u8,
    u: i8,
    v: i8,
}

fn to_yuv(r: u8, g: u8, b: u8) -> Yuv {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = (66 * r + 129 * g + 25 * b) as u16;
    let u = (-38 * r - 74 * g + 112 * b) as i16;
    let v = (112 * r - 94 * g - 18 * b) as i16;
    Yuv {
        y: ((y >> 8) + ((y >> 7) & 1)) as u8,
        u: ((u >> 8) + ((u >> 7) & 1)) as i8,
        v: ((v >> 8) + ((v >> 7) & 1)) as i8,
    }
}

/// Copies `rect` (left, top, right, bottom) of the EFB, read back as RGBA rows of 640, to the
/// XFB as BP 0x52 value `value` asks, and shows it.
pub(crate) fn copy(state: &State, value: u32, efb: &[u8], rect: [u32; 4]) -> Frame {
    let [left, top, right, bottom] = rect;
    let format = bits(state.bp[0x43], 0, 3);
    let clamp_top = bits(value, 0, 1) != 0;
    let clamp_bottom = bits(value, 1, 1) != 0;
    let gamma = [1.0f32, 1.7, 2.2, 1.0][bits(value, 7, 2) as usize];
    let gamma_rcp = 1.0 / gamma;
    let lo = state.bp[0x53];
    let hi = state.bp[0x54];
    let w = [
        bits(lo, 0, 6),
        bits(lo, 6, 6),
        bits(lo, 12, 6),
        bits(lo, 18, 6),
        bits(hi, 0, 6),
        bits(hi, 6, 6),
        bits(hi, 12, 6),
    ];
    let (w_prev, w_mid, w_next) = (w[0] + w[1], w[2] + w[3] + w[4], w[5] + w[6]);
    let lut: Vec<u8> = (0..256)
        .map(|c| ((c as f32 / 255.0).powf(gamma_rcp) * 255.0).clamp(0.0, 255.0) as u8)
        .collect();
    let color = |x: u32, y: u32| -> [u32; 3] {
        let at = ((y * EFB_WIDTH + x) * 4) as usize;
        let p = &efb[at..at + 3];
        // Formats without alpha read their color as is; RGBA6's is already six bits a channel.
        let _ = format;
        [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])]
    };
    let width = right - left;
    let height = bottom - top;
    // YUYV pairs: (Y, U or V) per pixel.
    let mut yuyv = vec![(0u8, 0u8); (width * height) as usize];
    let mut scanline = vec![Yuv::default(); (EFB_WIDTH + 2) as usize];
    for y in top..bottom {
        let y_prev = (y as i32 - 1).max(if clamp_top { top as i32 } else { 0 }) as u32;
        let y_next = (y + 1).min(
            if clamp_bottom {
                bottom
            } else {
                crate::EFB_HEIGHT
            } - 1,
        );
        for (i, x) in (left..right).enumerate() {
            let (p, c, n) = (color(x, y_prev), color(x, y), color(x, y_next));
            let mut out = [0u8; 3];
            for k in 0..3 {
                let sum = p[k] * w_prev + c[k] * w_mid + n[k] * w_next;
                out[k] = lut[(sum >> 6).min(255) as usize];
            }
            scanline[i + 1] = to_yuv(out[0], out[1], out[2]);
        }
        scanline[0] = scanline[1];
        scanline[(right + 1) as usize] = scanline[right as usize];
        let row = ((y - top) * width) as usize;
        let mut i = 1usize;
        let mut x = 0usize;
        while x + 1 < width as usize + 1 && i + 1 < scanline.len() {
            let (a, b, c) = (scanline[i - 1], scanline[i], scanline[i + 1]);
            let u = 128i32 + ((i32::from(a.u) + (i32::from(b.u) << 1) + i32::from(c.u)) >> 2);
            let v = 128i32 + ((i32::from(a.v) + (i32::from(b.v) << 1) + i32::from(c.v)) >> 2);
            if x < width as usize {
                yuyv[row + x] = (b.y.wrapping_add(16), u as u8);
            }
            if x + 1 < width as usize {
                yuyv[row + x + 1] = (c.y.wrapping_add(16), v as u8);
            }
            i += 2;
            x += 2;
        }
    }
    let y_scale = if bits(value, 10, 1) != 0 {
        256.0 / bits(state.bp[0x4E], 0, 9) as f32
    } else {
        bits(state.bp[0x4E], 0, 9) as f32 / 256.0
    };
    let out_height = ((height as f32) * y_scale) as u32;
    let mut rgba = Vec::with_capacity((width * out_height * 4) as usize);
    for oy in 0..out_height {
        let sy = ((oy as f32 / y_scale) as u32).min(height - 1);
        let row = (sy * width) as usize;
        let mut x = 0;
        while x < width as usize {
            let (y1, u) = yuyv[row + x];
            let (y2, v) = if x + 1 < width as usize {
                yuyv[row + x + 1]
            } else {
                (y1, 128)
            };
            let (y1, y2) = (i32::from(y1) - 16, i32::from(y2) - 16);
            let (u, v) = (i32::from(u) - 128, i32::from(v) - 128);
            for yy in [y1, y2].iter().take((width as usize - x).min(2)) {
                let y = *yy as f32;
                let r = ((1.164 * y + 1.596 * v as f32) as i32).clamp(0, 255);
                let g = ((1.164 * y - 0.392 * u as f32 - 0.813 * v as f32) as i32).clamp(0, 255);
                let b = ((1.164 * y + 2.017 * u as f32) as i32).clamp(0, 255);
                rgba.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
            }
            x += 2;
        }
    }
    Frame {
        width,
        height: out_height,
        rgba,
    }
}
