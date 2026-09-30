// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::Rng;
use gekko_fp::{FmaMode, fma_mode, fmadd, fmadds, fmsubs, fnmadds, fnmsubs, set_fma_mode};
use num_bigint::{BigInt, BigUint, Sign};

fn decompose(x: f32) -> (bool, u64, i64) {
    let bits = x.to_bits();
    let exp = i64::from((bits >> 23) & 0xFF);
    let frac = u64::from(bits & 0x7F_FFFF);
    if exp == 0 {
        (bits >> 31 != 0, frac, -149)
    } else {
        (bits >> 31 != 0, frac | 0x80_0000, exp - 150)
    }
}

/// Exactly rounded `a * c + b` as single bits, or `None` if zero, subnormal or overflowing.
fn exact_madd(a: f32, c: f32, b: f32) -> Option<u32> {
    let (na, ma, ea) = decompose(a);
    let (nc, mc, ec) = decompose(c);
    let (nb, mb, eb) = decompose(b);
    let product = BigInt::from(ma) * BigInt::from(mc);
    let product = if na != nc { -product } else { product };
    let addend = if nb {
        -BigInt::from(mb)
    } else {
        BigInt::from(mb)
    };
    let (ep, emin) = (ea + ec, (ea + ec).min(eb));
    let sum = (product << (ep - emin) as usize) + (addend << (eb - emin) as usize);
    if sum.sign() == Sign::NoSign {
        return None;
    }

    let negative = sum.sign() == Sign::Minus;
    let magnitude = sum.magnitude().clone();
    let shift = magnitude.bits() as i64 - 24;
    let (mut q, mut e) = if shift > 0 {
        let q = &magnitude >> shift as usize;
        let rem = &magnitude - (&q << shift as usize);
        let half = BigUint::from(1u8) << (shift - 1) as usize;
        let round_up = rem > half || (rem == half && q.bit(0));
        (if round_up { q + 1u8 } else { q }, emin + shift)
    } else {
        (magnitude << (-shift) as usize, emin + shift)
    };
    if q.bits() > 24 {
        q >>= 1;
        e += 1;
    }

    let biased = e + 23 + 127;
    if !(1..=254).contains(&biased) {
        return None;
    }
    let q = u64::try_from(&q).unwrap() as u32;
    Some((u32::from(negative) << 31) | ((biased as u32) << 23) | (q & 0x7F_FFFF))
}

fn bits(x: f64) -> u32 {
    (x as f32).to_bits()
}

fn f(bits: u32) -> f64 {
    f64::from(f32::from_bits(bits))
}

#[test]
fn hardware_fused_ops_round_exactly_once() {
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    let mut checked = 0;
    for i in 0..200_000 {
        let a = rng.f32_in(110, 145);
        let c = rng.f32_in(110, 145);
        // Every fourth case nearly cancels the product, which exercises long carries.
        let b = if i % 4 == 0 {
            let p = -(a * c);
            f32::from_bits(p.to_bits() ^ (rng.next() as u32 & 0xFF))
        } else {
            rng.f32_in(80, 175)
        };
        let (fa, fc, fb) = (f64::from(a), f64::from(c), f64::from(b));
        if let Some(expected) = exact_madd(a, c, b) {
            assert_eq!(
                bits(fmadds(fa, fc, fb)),
                expected,
                "fmadds {a:e} {c:e} {b:e}"
            );
            assert_eq!(
                bits(fnmadds(fa, fc, fb)),
                expected ^ 0x8000_0000,
                "fnmadds {a:e} {c:e} {b:e}"
            );
            checked += 1;
        }
        if let Some(expected) = exact_madd(a, c, -b) {
            assert_eq!(
                bits(fmsubs(fa, fc, fb)),
                expected,
                "fmsubs {a:e} {c:e} {b:e}"
            );
            assert_eq!(
                bits(fnmsubs(fa, fc, fb)),
                expected ^ 0x8000_0000,
                "fnmsubs {a:e} {c:e} {b:e}"
            );
        }
    }
    assert!(checked > 150_000, "too few normal results: {checked}");
}

// (a, c, b, hardware result, Slippi Dolphin result): the product is exactly halfway
// between two singles, and a tiny `b` decides the direction.
const TIES: [(u32, u32, u32, u32, u32); 3] = [
    // 4097 * 4097 + 2^-40: just above a tie whose lower neighbour is even.
    (
        0x4580_0800,
        0x4580_0800,
        0x2B80_0000,
        0x4B80_1001,
        0x4B80_1000,
    ),
    // 4099 * 4097 - 2^-40: just below a tie whose lower neighbour is odd.
    (
        0x4580_1800,
        0x4580_0800,
        0xAB80_0000,
        0x4B80_2001,
        0x4B80_2002,
    ),
    // Mario Strikers Charged, from Dolphin's regression report.
    (
        0x4248_0000,
        0xBC88_CC38,
        0x1B1C_72A0,
        0xBF55_BF17,
        0xBF55_BF18,
    ),
];

#[test]
fn tie_cases_follow_the_selected_mode() {
    for (a, c, b, hardware, slippi) in TIES {
        let (fa, fc, fb) = (f(a), f(c), f(b));
        assert_eq!(
            exact_madd(f32::from_bits(a), f32::from_bits(c), f32::from_bits(b)),
            Some(hardware)
        );

        set_fma_mode(FmaMode::Hardware);
        assert_eq!(bits(fmadds(fa, fc, fb)), hardware, "hardware {a:08X}");

        set_fma_mode(FmaMode::SlippiDolphin);
        assert_eq!(bits(fmadds(fa, fc, fb)), slippi, "slippi {a:08X}");
        assert_eq!(bits(fa.mul_add(fc, fb)), slippi, "double rounding {a:08X}");
    }
    set_fma_mode(FmaMode::Hardware);
}

#[test]
fn fma_mode_is_per_thread() {
    set_fma_mode(FmaMode::SlippiDolphin);
    let other = std::thread::spawn(fma_mode).join().unwrap();
    assert_eq!(other, FmaMode::Hardware);
    assert_eq!(fma_mode(), FmaMode::SlippiDolphin);
    set_fma_mode(FmaMode::Hardware);
}

#[test]
fn fused_nan_operands_are_checked_a_then_b_then_c() {
    let nan_a = f64::from_bits(0x7FF8_0000_0000_00A0);
    let nan_b = f64::from_bits(0x7FF8_0000_0000_00B0);
    let nan_c = f64::from_bits(0x7FF0_0000_0000_00C0); // signaling
    assert_eq!(fmadd(nan_a, nan_c, nan_b).to_bits(), nan_a.to_bits());
    assert_eq!(fmadd(1.0, nan_c, nan_b).to_bits(), nan_b.to_bits());
    assert_eq!(fmadd(1.0, nan_c, 1.0).to_bits(), 0x7FF8_0000_0000_00C0);
    assert_eq!(
        fmadd(0.0, f64::INFINITY, 1.0).to_bits(),
        gekko_fp::PPC_NAN.to_bits()
    );
}

#[test]
fn double_fused_ops_round_once() {
    let mut rng = Rng(0xD0B1_E000_0000_0001);
    for _ in 0..10_000 {
        let [a, c, b] = [(); 3].map(|_| f64::from_bits(rng.next() & 0xBFFF_FFFF_FFFF_FFFF));
        let expected = a.mul_add(c, b);
        if !expected.is_nan() {
            assert_eq!(fmadd(a, c, b).to_bits(), expected.to_bits());
        }
    }
}
