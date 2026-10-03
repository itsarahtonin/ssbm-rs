// SPDX-License-Identifier: GPL-3.0-or-later

//! Checks that a copy kept on the GPU, read back and re-encoded (copy::encode_texels), writes
//! the bytes the CPU encoder (encode.rs) writes for the same copy, and that the memory a copy
//! is recorded as covering (copy::footprint) is the memory the encoder writes.
//!
//! The GPU copy's texture is modelled as the texture decoder's reading of the encoder's bytes,
//! which is what copy.wgsl claims to compute; the shader itself is not run here.
//!
//! Partial blocks: when the copied rectangle isn't a whole number of blocks, the encoder still
//! writes its last blocks whole, with the EFB pixels past the rectangle in them (or zero past
//! the EFB's end). The GPU copy keeps only the rectangle, so encode_texels writes zero for those
//! texels. The test requires texels inside the rectangle to match exactly, requires those past
//! it to be zero from encode_texels, and counts (and prints) how many of them differ from what
//! the encoder writes, rather than hiding the difference.

use ssbm_gx::State;
use ssbm_gx::texture::decode;

use crate::copy::{self, Copied};
use crate::encode;
use crate::{EFB_HEIGHT, EFB_WIDTH};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 32) as u32 % n.max(1)
    }

    fn bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let v = self.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

/// BP 0x52's value for a copy to a texture of `format`, halved or not, with the intensity
/// (YUV) flags or not.
fn copy_value(format: u32, half: u32, yuv: bool) -> u32 {
    let tp = if format < 8 { format * 2 } else { (format - 8) * 2 + 1 };
    tp << 3 | half << 9 | if yuv { 1 << 15 | 1 << 16 } else { 0 } | 0x52 << 24
}

#[allow(clippy::too_many_arguments)]
fn state(pixel_format: u32, left: u32, top: u32, wm1: u32, hm1: u32, addr: u32, stride: u32) -> State {
    let mut s = State::new();
    s.bp[0x43] = pixel_format;
    s.bp[0x49] = left | top << 10;
    s.bp[0x4A] = wm1 | hm1 << 10;
    s.bp[0x4B] = addr >> 5;
    s.bp[0x4D] = stride >> 5;
    s
}

/// Texel `i` of a block (in the encoder's order) of `format`: its nibble (I4) or halfword
/// (RGB5A3).
fn texel_bits(format: u32, block: &[u8], i: usize) -> u32 {
    if format == 0 {
        let b = u32::from(block[i / 2]);
        if i % 2 == 0 { b >> 4 } else { b & 0xF }
    } else {
        u32::from(u16::from_be_bytes([block[2 * i], block[2 * i + 1]]))
    }
}

#[derive(Default, Debug)]
struct Stats {
    cases: u32,
    texels_inside: u64,
    mismatches_inside: u64,
    texels_past: u64,
    past_differing: u64,
    past_nonzero_from_texels: u64,
    whole_rows_equal: u64,
    rows: u64,
    opaque: u64,
    translucent: u64,
}

#[allow(clippy::too_many_arguments)]
fn check_case(
    stats: &mut Stats,
    efb: &[u8],
    format: u32,
    yuv: bool,
    pixel_format: u32,
    half: u32,
    left: u32,
    top: u32,
    wm1: u32,
    hm1: u32,
    addr: u32,
    stride_pad: u32,
) {
    let value = copy_value(format, half, yuv);
    let (lw, lh, block) = if format == 0 { (3, 3, 32) } else { (2, 2, 32) };
    let row_bytes = (((wm1 >> half) >> lw) + 1) * block;
    let stride = row_bytes + stride_pad;
    let st = state(pixel_format, left, top, wm1, hm1, addr, stride);
    let label = format!(
        "format {format} yuv {yuv} pixel format {pixel_format} half {half} at ({left}, {top}) \
         size field {}x{} addr {addr:#x} stride {stride}",
        wm1 + 1,
        hm1 + 1
    );
    let e = encode::encode(&st, value, efb).unwrap_or_else(|| panic!("encoded: {label}"));
    let r = copy::request(&st, value).unwrap_or_else(|| panic!("requested: {label}"));
    let f = r.footprint;
    assert_eq!(f.address, e.address, "{label}");
    assert_eq!(f.stride, e.stride, "{label}");
    assert_eq!(f.rows as usize, e.rows.len(), "{label}");
    assert!(e.rows.iter().all(|row| row.len() == f.row_bytes as usize), "{label}");
    assert_eq!((r.width, r.height), ((wm1 >> half) + 1, (hm1 >> half) + 1), "{label}");
    let c = Copied {
        id: 0,
        format: r.format,
        width: r.width,
        height: r.height,
        footprint: f,
    };
    // The texture the GPU copy holds: the decoder's reading of the encoder's bytes, its rows
    // of blocks laid end to end as the decoder expects.
    let src: Vec<u8> = e.rows.concat();
    let texels = decode(&src, c.width, c.height, 1, c.format, &[], 0).remove(0);
    assert_eq!(texels.len(), (c.width * c.height * 4) as usize, "{label}");
    if format == 5 {
        for p in texels.chunks(4) {
            if p[3] == 255 {
                stats.opaque += 1;
            } else {
                stats.translucent += 1;
            }
        }
    }
    let got = copy::encode_texels(&c, &texels);
    assert_eq!(got.len(), e.rows.len(), "{label}");
    let (bw, bh) = (1u32 << lw, 1u32 << lh);
    let texels_per_block = (bw * bh) as usize;
    for (tb, ((at, row), want)) in got.iter().zip(&e.rows).enumerate() {
        let tb = tb as u32;
        assert_eq!(*at, (e.address + tb * e.stride) & 0x01FF_FFFF, "{label} row {tb}");
        assert_eq!(row.len(), want.len(), "{label} row {tb}");
        stats.rows += 1;
        if row == want {
            stats.whole_rows_equal += 1;
        }
        for (sb, (gb, wb)) in row
            .chunks(block as usize)
            .zip(want.chunks(block as usize))
            .enumerate()
        {
            for i in 0..texels_per_block {
                let (s, t) = (sb as u32 * bw + i as u32 % bw, tb * bh + i as u32 / bw);
                let (g, w) = (texel_bits(format, gb, i), texel_bits(format, wb, i));
                if s < c.width && t < c.height {
                    stats.texels_inside += 1;
                    if g != w {
                        stats.mismatches_inside += 1;
                        if stats.mismatches_inside <= 10 {
                            eprintln!(
                                "MISMATCH {label}: texel ({s}, {t}) re-encoded {g:#x}, \
                                 encoder {w:#x}"
                            );
                        }
                    }
                } else {
                    stats.texels_past += 1;
                    if g != 0 {
                        stats.past_nonzero_from_texels += 1;
                    }
                    if g != w {
                        stats.past_differing += 1;
                    }
                }
            }
        }
    }
    stats.cases += 1;
}

fn random_efb(rng: &mut Rng) -> Vec<u8> {
    let mut efb = vec![0u8; (EFB_WIDTH * EFB_HEIGHT * 4) as usize];
    rng.bytes(&mut efb);
    efb
}

#[test]
fn encode_texels_rewrites_the_encoders_bytes() {
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    // EFB rectangles (width, height) as the copy's size field gives them (less one). At half
    // scale each is also tried doubled, so the texture comes out at the size named.
    let sizes: &[(u32, u32)] = &[
        (52, 74),
        (70, 100),
        (256, 256),
        (1, 1),
        (5, 3),
        (8, 8),
        (4, 4),
        (9, 7),
        (33, 17),
        (640, 528),
        (639, 527),
    ];
    let formats: &[(u32, bool)] = &[(0, false), (0, true), (5, false)];
    let mut by_format: Vec<((u32, bool), Stats)> = Vec::new();
    for &(format, yuv) in formats {
        let mut stats = Stats::default();
        for pixel_format in 0..2 {
            for half in 0..2 {
                for &(w, h) in sizes {
                    let mut fields = vec![(w - 1, h - 1)];
                    if half == 1 && 2 * w <= EFB_WIDTH && 2 * h <= EFB_HEIGHT {
                        fields.push((2 * w - 1, 2 * h - 1));
                    }
                    for (wm1, hm1) in fields {
                        for trial in 0..3 {
                            let efb = random_efb(&mut rng);
                            let (ew, eh) = (wm1 + 1, hm1 + 1);
                            // Anywhere it fits, and once against the EFB's bottom right.
                            let (left, top) = if trial == 2 {
                                (EFB_WIDTH - ew, EFB_HEIGHT - eh)
                            } else {
                                (rng.below(EFB_WIDTH - ew + 1), rng.below(EFB_HEIGHT - eh + 1))
                            };
                            let addr = (0x0040_0000 + rng.below(0x0100_0000)) & !31;
                            let stride_pad = if trial == 1 { 32 * rng.below(4) } else { 0 };
                            check_case(
                                &mut stats, &efb, format, yuv, pixel_format, half, left, top,
                                wm1, hm1, addr, stride_pad,
                            );
                        }
                    }
                }
            }
        }
        // Fully random rectangles, sizes anywhere up to the EFB's.
        for _ in 0..200 {
            let efb = random_efb(&mut rng);
            let (pixel_format, half) = (rng.below(2), rng.below(2));
            let (wm1, hm1) = (rng.below(EFB_WIDTH), rng.below(EFB_HEIGHT));
            let left = rng.below(EFB_WIDTH - wm1);
            let top = rng.below(EFB_HEIGHT - hm1);
            let addr = rng.below(0x0200_0000) & !31;
            check_case(
                &mut stats, &efb, format, yuv, pixel_format, half, left, top, wm1, hm1, addr,
                32 * rng.below(3),
            );
        }
        eprintln!("format {format} yuv {yuv}: {stats:?}");
        by_format.push(((format, yuv), stats));
    }
    for ((format, yuv), s) in &by_format {
        assert_eq!(s.mismatches_inside, 0, "format {format} yuv {yuv}: texels inside differ");
        assert_eq!(
            s.past_nonzero_from_texels, 0,
            "format {format} yuv {yuv}: encode_texels wrote past the rectangle"
        );
        if *format == 5 {
            assert!(s.opaque > 0 && s.translucent > 0, "both RGB5A3 forms exercised");
        }
    }
}

/// Copies that are whole blocks match the encoder byte for byte, rows and all.
#[test]
fn whole_block_copies_match_byte_for_byte() {
    let mut rng = Rng(0x0BAD_CAFE_F00D_0002);
    for &(format, yuv) in &[(0u32, false), (0, true), (5, false)] {
        let (bw, bh) = if format == 0 { (8, 8) } else { (4, 4) };
        for pixel_format in 0..2 {
            for half in 0..2 {
                for &(w, h) in &[(64u32, 64u32), (8, 8), (4, 4), (72, 104), (256, 256)] {
                    if w % bw != 0 || h % bh != 0 {
                        continue;
                    }
                    let (wm1, hm1) = ((w << half) - 1, (h << half) - 1);
                    if wm1 >= EFB_WIDTH || hm1 >= EFB_HEIGHT {
                        continue;
                    }
                    let efb = random_efb(&mut rng);
                    let mut stats = Stats::default();
                    check_case(
                        &mut stats, &efb, format, yuv, pixel_format, half,
                        rng.below(EFB_WIDTH - wm1), rng.below(EFB_HEIGHT - hm1), wm1, hm1,
                        0x0080_0000, 0,
                    );
                    assert_eq!(stats.texels_past, 0);
                    assert_eq!(stats.whole_rows_equal, stats.rows, "format {format} {w}x{h}");
                }
            }
        }
    }
}

/// copy::footprint describes the memory encode::encode writes, for every copy format and pixel
/// format, and is None exactly where the encoder writes nothing.
#[test]
fn footprint_is_what_the_encoder_writes() {
    let mut rng = Rng(0xF007_9417_0000_0003);
    let efb = random_efb(&mut rng);
    let (mut some, mut none) = (0u32, 0u32);
    for tp in 0..16 {
        for half in 0..2 {
            for yuv in [false, true] {
                for pixel_format in 0..8 {
                    for _ in 0..40 {
                        let (wm1, hm1) = (rng.below(1024), rng.below(1024));
                        let st = state(
                            pixel_format,
                            rng.below(EFB_WIDTH),
                            rng.below(EFB_HEIGHT),
                            wm1,
                            hm1,
                            rng.below(1 << 24) << 5,
                            rng.below(1024) << 5,
                        );
                        let value = tp << 3 | half << 9 | if yuv { 3 << 15 } else { 0 } | 0x52 << 24;
                        let f = copy::footprint(&st, value);
                        let e = encode::encode(&st, value, &efb);
                        match (f, e) {
                            (None, None) => none += 1,
                            (Some(f), Some(e)) => {
                                some += 1;
                                let label = format!("tp {tp} half {half} pf {pixel_format}");
                                assert_eq!(f.address, e.address, "{label}");
                                assert_eq!(f.stride, e.stride, "{label}");
                                assert_eq!(f.rows as usize, e.rows.len(), "{label}");
                                for row in &e.rows {
                                    assert_eq!(row.len(), f.row_bytes as usize, "{label}");
                                }
                                let (start, end) = f.span();
                                assert_eq!(start, e.address);
                                assert_eq!(
                                    end,
                                    e.address
                                        + (e.rows.len() as u32 - 1) * e.stride
                                        + e.rows[0].len() as u32
                                );
                            }
                            (f, e) => panic!(
                                "tp {tp} pf {pixel_format}: footprint {f:?}, encoded {}",
                                e.is_some()
                            ),
                        }
                    }
                }
            }
        }
    }
    eprintln!("footprint: {some} copies compared, {none} neither encodes");
    assert!(some > 0 && none > 0);
}
