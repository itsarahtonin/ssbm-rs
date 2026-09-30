// SPDX-License-Identifier: GPL-3.0-or-later

/// Deterministic xorshift64* generator, so failures reproduce.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A random single with a biased exponent in `lo..=hi` and a random sign and mantissa.
    pub fn f32_in(&mut self, lo: u32, hi: u32) -> f32 {
        let r = self.next();
        let sign = (r >> 63) as u32;
        let exp = lo + ((r >> 32) as u32 % (hi - lo + 1));
        f32::from_bits((sign << 31) | (exp << 23) | (r as u32 & 0x7F_FFFF))
    }
}
