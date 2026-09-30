// SPDX-License-Identifier: GPL-3.0-or-later
// Tables and algorithms ported from Dolphin's Common/FloatUtils.cpp (GPL-2.0-or-later).

use crate::make_quiet;

struct BaseAndDec {
    base: i64,
    dec: i64,
}

const fn e(base: i64, dec: i64) -> BaseAndDec {
    BaseAndDec { base, dec }
}

#[rustfmt::skip]
const FRSQRTE_EXPECTED: [BaseAndDec; 32] = [
    e(0x1a7e800, -0x568), e(0x17cb800, -0x4f3), e(0x1552800, -0x48d), e(0x130c000, -0x435),
    e(0x10f2000, -0x3e7), e(0x0eff000, -0x3a2), e(0x0d2e000, -0x365), e(0x0b7c000, -0x32e),
    e(0x09e5000, -0x2fc), e(0x0867000, -0x2d0), e(0x06ff000, -0x2a8), e(0x05ab800, -0x283),
    e(0x046a000, -0x261), e(0x0339800, -0x243), e(0x0218800, -0x226), e(0x0105800, -0x20b),
    e(0x3ffa000, -0x7a4), e(0x3c29000, -0x700), e(0x38aa000, -0x670), e(0x3572000, -0x5f2),
    e(0x3279000, -0x584), e(0x2fb7000, -0x524), e(0x2d26000, -0x4cc), e(0x2ac0000, -0x47e),
    e(0x2881000, -0x43a), e(0x2665000, -0x3fa), e(0x2468000, -0x3c2), e(0x2287000, -0x38e),
    e(0x20c1000, -0x35e), e(0x1f12000, -0x332), e(0x1d79000, -0x30a), e(0x1bf4000, -0x2e6),
];

#[rustfmt::skip]
const FRES_EXPECTED: [BaseAndDec; 32] = [
    e(0x7ff800, 0x3e1), e(0x783800, 0x3a7), e(0x70ea00, 0x371), e(0x6a0800, 0x340),
    e(0x638800, 0x313), e(0x5d6200, 0x2ea), e(0x579000, 0x2c4), e(0x520800, 0x2a0),
    e(0x4cc800, 0x27f), e(0x47ca00, 0x261), e(0x430800, 0x245), e(0x3e8000, 0x22a),
    e(0x3a2c00, 0x212), e(0x360800, 0x1fb), e(0x321400, 0x1e5), e(0x2e4a00, 0x1d1),
    e(0x2aa800, 0x1be), e(0x272c00, 0x1ac), e(0x23d600, 0x19b), e(0x209e00, 0x18b),
    e(0x1d8800, 0x17c), e(0x1a9000, 0x16e), e(0x17ae00, 0x15b), e(0x14f800, 0x15b),
    e(0x124400, 0x143), e(0x0fbe00, 0x143), e(0x0d3800, 0x12d), e(0x0ade00, 0x12d),
    e(0x088400, 0x11a), e(0x065000, 0x11a), e(0x041c00, 0x108), e(0x020c00, 0x106),
];

const EXP_MASK: i64 = 0x7FF << 52;
const MANTISSA_MASK: i64 = (1 << 52) - 1;

/// `frsqrte`: the hardware's reciprocal square root estimate.
pub fn frsqrte(val: f64) -> f64 {
    let integral = val.to_bits() as i64;
    let mut mantissa = integral & MANTISSA_MASK;
    let sign = integral & i64::MIN;
    let mut exponent = integral & EXP_MASK;

    if mantissa == 0 && exponent == 0 {
        return if sign != 0 {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if exponent == EXP_MASK {
        if mantissa == 0 {
            return if sign != 0 { crate::PPC_NAN } else { 0.0 };
        }
        return make_quiet(val);
    }
    if sign != 0 {
        return crate::PPC_NAN;
    }

    if exponent == 0 {
        // Normalize subnormals.
        loop {
            exponent -= 1 << 52;
            mantissa <<= 1;
            if mantissa & (1 << 52) != 0 {
                break;
            }
        }
        mantissa &= MANTISSA_MASK;
        exponent += 1 << 52;
    }

    let exponent_lsb = exponent & (1 << 52);
    let exponent = ((0x3FF << 52) - ((exponent - (0x3FE << 52)) / 2)) & EXP_MASK;
    let i = (exponent_lsb | mantissa) >> 37;
    let entry = &FRSQRTE_EXPECTED[(i / 2048) as usize];
    let result = sign | exponent | ((entry.base + entry.dec * (i % 2048)) << 26);
    f64::from_bits(result as u64)
}

/// `fres`: the hardware's reciprocal estimate.
pub fn fres(val: f64) -> f64 {
    let integral = val.to_bits() as i64;
    let mantissa = integral & MANTISSA_MASK;
    let sign = integral & i64::MIN;
    let exponent = integral & EXP_MASK;

    if mantissa == 0 && exponent == 0 {
        return f64::INFINITY.copysign(val);
    }
    if exponent == EXP_MASK {
        if mantissa == 0 {
            return 0.0_f64.copysign(val);
        }
        return make_quiet(val);
    }
    if exponent < (895 << 52) {
        return f64::from(f32::MAX).copysign(val);
    }
    if exponent >= (1149 << 52) {
        return 0.0_f64.copysign(val);
    }

    let exponent = (0x7FD << 52) - exponent;
    let i = mantissa >> 37;
    let entry = &FRES_EXPECTED[(i / 1024) as usize];
    let result = sign | exponent | ((entry.base - (entry.dec * (i % 1024) + 1) / 2) << 29);
    f64::from_bits(result as u64)
}
