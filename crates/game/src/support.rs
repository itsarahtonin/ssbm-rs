// SPDX-License-Identifier: GPL-3.0-or-later

//! What translated code needs beyond the runtime: C integer semantics as the Gekko computes
//! them, and handles made from original addresses.

use ssbm_rt::{At, Ctx, FnPtr, Handle, StackFrame, Val};

/// A handle for the original address `addr`, as a C cast from an integer makes one.
#[inline]
pub fn ptr<'a, H: Handle<'a>>(ctx: &'a Ctx, addr: u32) -> H {
    H::from_at(At::new(ctx, addr))
}

/// A string literal, which lives at its original address in the game's data.
#[inline]
pub fn cstr(ctx: &Ctx, addr: u32) -> Val<'_, i8> {
    ptr(ctx, addr)
}

/// A pointer to the function at `addr`.
#[inline]
pub fn fnptr(ctx: &Ctx, addr: u32) -> FnPtr<'_> {
    ptr(ctx, addr)
}

/// The local at `off` in a function's emulated stack frame.
#[inline]
pub fn frame_at<'a, H: Handle<'a>>(ctx: &'a Ctx, frame: &StackFrame<'a>, off: u32) -> H {
    ptr(ctx, frame.base() + off)
}

// Division as `divw` and `divwu` compute it: dividing by zero, or the most negative value by
// -1, gives a defined result instead of a trap. MWCC divides and shifts 64-bit values with the
// runtime's helpers, which translated code calls.

#[inline]
pub fn div_i32(a: i32, b: i32) -> i32 {
    if b == 0 || (a == i32::MIN && b == -1) {
        if a < 0 { -1 } else { 0 }
    } else {
        a / b
    }
}

#[inline]
pub fn div_u32(a: u32, b: u32) -> u32 {
    a.checked_div(b).unwrap_or(0)
}

/// `%` is computed from the quotient, as MWCC does.
#[inline]
pub fn rem_i32(a: i32, b: i32) -> i32 {
    a.wrapping_sub(div_i32(a, b).wrapping_mul(b))
}

#[inline]
pub fn rem_u32(a: u32, b: u32) -> u32 {
    a.wrapping_sub(div_u32(a, b).wrapping_mul(b))
}

// Shifts as `slw`, `srw` and `sraw` compute them: the amount is taken mod 64, and 32 or more
// shifts everything out.

#[inline]
pub fn shl_i32(a: i32, n: u32) -> i32 {
    shl_u32(a as u32, n) as i32
}

#[inline]
pub fn shl_u32(a: u32, n: u32) -> u32 {
    let n = n & 63;
    if n >= 32 { 0 } else { a << n }
}

#[inline]
pub fn shr_u32(a: u32, n: u32) -> u32 {
    let n = n & 63;
    if n >= 32 { 0 } else { a >> n }
}

#[inline]
pub fn shr_i32(a: i32, n: u32) -> i32 {
    shr_u32(a as u32, n) as i32
}

#[inline]
pub fn sar_i32(a: i32, n: u32) -> i32 {
    let n = n & 63;
    if n >= 32 { a >> 31 } else { a >> n }
}

#[inline]
pub fn sar_u32(a: u32, n: u32) -> u32 {
    sar_i32(a as i32, n) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn division_matches_the_gekko() {
        assert_eq!(div_i32(7, 0), 0);
        assert_eq!(div_i32(-7, 0), -1);
        assert_eq!(div_i32(i32::MIN, -1), -1);
        assert_eq!(div_i32(-7, 2), -3);
        assert_eq!(rem_i32(-7, 2), -1);
        assert_eq!(div_u32(7, 0), 0);
        assert_eq!(rem_u32(7, 0), 7);
    }

    #[test]
    fn shifts_match_the_gekko() {
        assert_eq!(shl_u32(1, 32), 0);
        assert_eq!(shl_u32(1, 64 + 3), 8);
        assert_eq!(sar_i32(-8, 40), -1);
        assert_eq!(shr_u32(0x8000_0000, 31), 1);
    }
}
