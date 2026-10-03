// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's free DSP ROM's generate_coefs.py (GPL-2.0-or-later), and NumPy's window
// and Bessel functions it calls (BSD-3-Clause).

//! The polyphase resampler's coefficients: the table Dolphin's free DSP ROM ships as
//! dsp_coef.bin (Nintendo's DSP ROM has its own), computed as its generator computes it. Three
//! 4-tap filters of 128 phases (windowed sincs), a fourth of zeros, and 23 values the GBA
//! microcode needs set by hand.

use std::f64::consts::PI;

const N: usize = 512;

/// NumPy's `linspace(-2, 2, 512)`.
fn linspace(i: usize) -> f64 {
    if i == N - 1 {
        2.0
    } else {
        i as f64 * (4.0 / (N - 1) as f64) + -2.0
    }
}

/// NumPy's `hamming(512)`.
fn hamming(i: usize) -> f64 {
    let n = (1.0 - N as f64) + 2.0 * i as f64;
    0.54 + 0.46 * (PI * n / (N as f64 - 1.0)).cos()
}

/// NumPy's Chebyshev series evaluation.
fn chbevl(x: f64, vals: &[f64]) -> f64 {
    let (mut b0, mut b1, mut b2) = (vals[0], 0.0, 0.0);
    for v in &vals[1..] {
        b2 = b1;
        b1 = b0;
        b0 = x * b1 - b2 + v;
    }
    0.5 * (b0 - b2)
}

const I0_A: [f64; 30] = [
    -4.4153416464793395e-18,
    3.3307945188222384e-17,
    -2.431279846547955e-16,
    1.715391285555133e-15,
    -1.1685332877993451e-14,
    7.676185498604936e-14,
    -4.856446783111929e-13,
    2.95505266312964e-12,
    -1.726826291441556e-11,
    9.675809035373237e-11,
    -5.189795601635263e-10,
    2.6598237246823866e-09,
    -1.300025009986248e-08,
    6.046995022541919e-08,
    -2.670793853940612e-07,
    1.1173875391201037e-06,
    -4.4167383584587505e-06,
    1.6448448070728896e-05,
    -5.754195010082104e-05,
    0.00018850288509584165,
    -0.0005763755745385824,
    0.0016394756169413357,
    -0.004324309995050576,
    0.010546460394594998,
    -0.02373741480589947,
    0.04930528423967071,
    -0.09490109704804764,
    0.17162090152220877,
    -0.3046826723431984,
    0.6767952744094761,
];

const I0_B: [f64; 25] = [
    -7.233180487874754e-18,
    -4.830504485944182e-18,
    4.46562142029676e-17,
    3.461222867697461e-17,
    -2.8276239805165836e-16,
    -3.425485619677219e-16,
    1.7725601330565263e-15,
    3.8116806693526224e-15,
    -9.554846698828307e-15,
    -4.150569347287222e-14,
    1.54008621752141e-14,
    3.8527783827421426e-13,
    7.180124451383666e-13,
    -1.7941785315068062e-12,
    -1.3215811840447713e-11,
    -3.1499165279632416e-11,
    1.1889147107846439e-11,
    4.94060238822497e-10,
    3.3962320257083865e-09,
    2.266668990498178e-08,
    2.0489185894690638e-07,
    2.8913705208347567e-06,
    6.889758346916825e-05,
    0.0033691164782556943,
    0.8044904110141088,
];

/// NumPy's modified Bessel function of the first kind, order 0.
fn i0(x: f64) -> f64 {
    let x = x.abs();
    if x <= 8.0 {
        x.exp() * chbevl(x / 2.0 - 2.0, &I0_A)
    } else {
        x.exp() * chbevl(32.0 / x - 2.0, &I0_B) / x.sqrt()
    }
}

/// NumPy's `kaiser(512, beta)`.
fn kaiser(i: usize, beta: f64) -> f64 {
    let alpha = (N as f64 - 1.0) / 2.0;
    let r = (i as f64 - alpha) / alpha;
    i0(beta * (1.0 - r * r).sqrt()) / i0(beta)
}

/// NumPy's `sinc`.
fn sinc(x: f64) -> f64 {
    let x = PI * x;
    let y = if x != 0.0 { x } else { f64::EPSILON };
    y.sin() / y
}

/// Python 3's round(), to even at halves.
fn round_even(v: f64) -> i64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 {
        (2.0 * (v / 2.0).round()) as i64
    } else {
        r as i64
    }
}

/// One filter's 512 coefficients as 128 phases of 4 taps, normalized so the largest phase sums
/// to 32767.
fn convert(c: &[f64]) -> Vec<u16> {
    let phase = |p: usize| [c[127 - p], c[255 - p], c[383 - p], c[511 - p]];
    let m = (0..128)
        .map(|p| phase(p).iter().fold(0.0, |s, v| s + v))
        .fold(f64::NEG_INFINITY, f64::max);
    (0..128)
        .flat_map(|p| phase(p).map(|n| (round_even(n / m * 32767.0) & 0xFFFF) as u16))
        .collect()
}

const GBA: [(usize, u16); 23] = [
    (0x03b, 0x0065),
    (0x043, 0x0076),
    (0x0ca, 0x3461),
    (0x0e2, 0x376f),
    (0x1b8, 0x007f),
    (0x1b8, 0x007f),
    (0x1f8, 0x0009),
    (0x1fc, 0x0003),
    (0x229, 0x657c),
    (0x231, 0x64fc),
    (0x259, 0x6143),
    (0x285, 0x5aff),
    (0x456, 0x102f),
    (0x468, 0xf808),
    (0x491, 0x6a0f),
    (0x5f1, 0x0200),
    (0x5f6, 0x7f65),
    (0x65b, 0x0000),
    (0x66b, 0x0000),
    (0x66c, 0x06f2),
    (0x6fe, 0x0008),
    (0x723, 0xffe0),
    (0x766, 0x0273),
];

/// The 0x800 coefficients, as dsp_coef.bin holds them.
pub fn table() -> Vec<i16> {
    let filter = |f: &dyn Fn(usize) -> f64| (0..N).map(f).collect::<Vec<f64>>();
    let beta = PI * 9.0 / 4.0;
    let c1 = filter(&|i| sinc(linspace(i) * 0.5) * hamming(i));
    let c2 = filter(&|i| sinc(linspace(i) * 0.75) * kaiser(i, beta));
    let c3 = filter(&|i| sinc(linspace(i)) * hamming(i));
    let mut out: Vec<u16> = [convert(&c1), convert(&c2), convert(&c3)].concat();
    out.resize(0x800, 0);
    for (at, v) in GBA {
        out[at] = v;
    }
    out.into_iter().map(|v| v as i16).collect()
}

#[cfg(test)]
mod tests {
    /// DSP_COEF names Dolphin's Sys/GC/dsp_coef.bin to check the table against.
    #[test]
    fn matches_dolphins_table() {
        let Ok(path) = std::env::var("DSP_COEF") else {
            return;
        };
        let file = std::fs::read(path).expect("reading DSP_COEF");
        let ours: Vec<u8> = super::table()
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        let first = ours.iter().zip(&file).position(|(a, b)| a != b);
        assert_eq!(first, None, "first difference at byte {first:?}");
        assert_eq!(ours.len(), file.len());
    }
}
