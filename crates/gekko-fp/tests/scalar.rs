// SPDX-License-Identifier: GPL-3.0-or-later

use gekko_fp::*;

const SINGLE_TEST_VALUES: [u32; 33] = [
    0x0000_0000,
    0x0000_0001,
    0x0000_1000,
    0x007F_FFFF,
    0x0080_0000,
    0x0080_0002,
    0x3F80_0000,
    0x7F7F_FFFF,
    0x7F80_0000,
    0x7F80_0001,
    0x7FBF_FFFF,
    0x7FC0_0000,
    0x7FFF_FFFF,
    0x8000_0000,
    0x8000_0001,
    0x8000_1000,
    0x807F_FFFF,
    0x8080_0000,
    0x8080_0002,
    0xBFF0_0000,
    0xFF7F_FFFF,
    0xFF80_0000,
    0xFF80_0001,
    0xFFBF_FFFF,
    0xFFC0_0000,
    0xFFFF_FFFF,
    0x7E00_0000,
    0x7E80_0000,
    0xC7C0_0000,
    0xC7D0_0000,
    0x3FC0_0000,
    0x447A_0000,
    0xC040_0000,
];

#[test]
fn invalid_operations_return_positive_nan() {
    let nan = PPC_NAN.to_bits();
    assert_eq!(nan, 0x7FF8_0000_0000_0000);
    assert_eq!(fsub(f64::INFINITY, f64::INFINITY).to_bits(), nan);
    assert_eq!(fadd(f64::INFINITY, f64::NEG_INFINITY).to_bits(), nan);
    assert_eq!(fmul(0.0, f64::INFINITY).to_bits(), nan);
    assert_eq!(fdiv(0.0, 0.0).to_bits(), nan);
    assert_eq!(fdiv(f64::INFINITY, f64::INFINITY).to_bits(), nan);
    assert_eq!(fadds(f64::INFINITY, f64::NEG_INFINITY).to_bits(), nan);
}

#[test]
fn nan_operands_propagate_quieted_first_operand_first() {
    let snan = f64::from_bits(0x7FF0_0000_0000_0001);
    let qnan_a = f64::from_bits(0xFFF8_0000_0000_0AAA);
    let qnan_b = f64::from_bits(0x7FF8_0000_0000_0BBB);
    assert_eq!(fadd(snan, 1.0).to_bits(), 0x7FF8_0000_0000_0001);
    assert_eq!(fadd(1.0, snan).to_bits(), 0x7FF8_0000_0000_0001);
    assert_eq!(fmul(qnan_a, qnan_b).to_bits(), qnan_a.to_bits());
    assert_eq!(fdiv(qnan_b, qnan_a).to_bits(), qnan_b.to_bits());
}

#[test]
fn frsp_rounds_to_nearest_even_and_truncates_nan_payloads() {
    // 1 + 2^-24 is a tie between 1.0 and 1 + 2^-23; ties go to even.
    assert_eq!(frsp(1.0 + 2f64.powi(-24)), 1.0);
    assert_eq!(frsp(1.0 + 3.0 * 2f64.powi(-24)), 1.0 + 2f64.powi(-22));
    assert_eq!(frsp(1e39), f64::INFINITY);
    let snan = f64::from_bits(0xFFF0_0000_2000_0001);
    assert_eq!(frsp(snan).to_bits(), 0xFFF8_0000_2000_0000);
}

#[test]
fn fmuls_rounds_frc_to_25_bits_first() {
    let a = 1.0 + 2f64.powi(-23);
    let c = 1.0 + 2f64.powi(-25);
    // frC becomes 1 + 2^-24, which pushes the product over the halfway point.
    assert_eq!(fmuls(a, c), 1.0 + 2f64.powi(-22));
    assert_eq!(f64::from((a as f32) * (c as f32)), 1.0 + 2f64.powi(-23));
    assert_eq!(fmul(a, c), a * c);
}

#[test]
fn fctiwz_saturates_like_hardware() {
    assert_eq!(fctiwz(f64::NAN), i32::MIN);
    assert_eq!(
        std::hint::black_box(f64::NAN) as i32,
        0,
        "Rust's cast differs, which is why fctiwz exists"
    );
    assert_eq!(fctiwz(f64::INFINITY), i32::MAX);
    assert_eq!(fctiwz(f64::NEG_INFINITY), i32::MIN);
    assert_eq!(fctiwz(2_147_483_647.9), i32::MAX);
    assert_eq!(fctiwz(2_147_483_648.0), i32::MAX);
    assert_eq!(fctiwz(-2_147_483_648.9), i32::MIN);
    assert_eq!(fctiwz(-2_147_483_649.0), i32::MIN);
    assert_eq!(fctiwz(1.9), 1);
    assert_eq!(fctiwz(-1.9), -1);
}

#[test]
fn fctiw_rounds_half_to_even() {
    for (input, expected) in [
        (0.5, 0),
        (1.5, 2),
        (2.5, 2),
        (3.5, 4),
        (-2.5, -2),
        (-3.5, -4),
    ] {
        assert_eq!(fctiw(input), expected, "fctiw({input})");
    }
}

#[test]
fn fcti_bits_matches_the_register_image() {
    assert_eq!(fcti_bits(1.2, 1), 0xFFF8_0000_0000_0001);
    assert_eq!(fcti_bits(-1.0, -1), 0xFFF8_0000_FFFF_FFFF);
    assert_eq!(fcti_bits(-0.0, 0), 0xFFF8_0001_0000_0000);
    assert_eq!(fcti_bits(-0.4, fctiwz(-0.4)), 0xFFF8_0001_0000_0000);
}

#[test]
fn fsel_treats_negative_zero_as_non_negative() {
    assert_eq!(fsel(-0.0, 1.0, 2.0), 1.0);
    assert_eq!(fsel(-1e-30, 1.0, 2.0), 2.0);
    assert_eq!(fsel(f64::NAN, 1.0, 2.0), 2.0);
}

#[test]
fn sign_ops_only_touch_the_sign_bit() {
    let nan = f64::from_bits(0x7FF8_0000_0000_1234);
    assert_eq!(fneg(nan).to_bits(), 0xFFF8_0000_0000_1234);
    assert_eq!(fabs(-2.0), 2.0);
    assert_eq!(fnabs(2.0), -2.0);
    assert_eq!(fcmp(nan, 1.0), FpCompare::Unordered);
    assert_eq!(fcmp(-0.0, 0.0), FpCompare::Equal);
}

#[test]
fn lfs_then_stfs_round_trips_every_single() {
    let strided = (0..=u32::MAX).step_by(0x1_0001);
    for bits in SINGLE_TEST_VALUES.into_iter().chain(strided) {
        assert_eq!(stfs(lfs(bits)), bits, "{bits:08X}");
    }
}

#[test]
fn lfs_keeps_signaling_nans_signaling() {
    assert_eq!(lfs(0x7F80_0001).to_bits(), 0x7FF0_0000_2000_0000);
    assert_eq!(lfs(0x3F80_0000), 1.0);
    assert_eq!(lfs(0x0000_0001), 2f64.powi(-149));
}

#[test]
fn stfs_truncates_where_frsp_rounds() {
    let x = 1.0 + 2f64.powi(-23) - 2f64.powi(-40);
    assert_eq!(stfs(x), 0x3F80_0000);
    assert_eq!(stfs(frsp(x)), 0x3F80_0001);
}

#[test]
fn stfs_ftz_flushes_single_subnormals() {
    let tiny = 2f64.powi(-140);
    assert_eq!(stfs(tiny), 0x0000_0200);
    assert_eq!(stfs_ftz(tiny), 0);
    assert_eq!(stfs_ftz(-tiny), 0x8000_0000);
    assert_eq!(stfs_ftz(1.0), 0x3F80_0000);
}
