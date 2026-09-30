// SPDX-License-Identifier: GPL-3.0-or-later
// Paired-single semantics ported from Dolphin's Interpreter_Paired.cpp (GPL-2.0-or-later).

use crate::{add, div, fabs, fnabs, fneg, frsp, madd_single, mul, round_c, sub};

/// A paired-single register: two lanes of register values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ps {
    pub ps0: f64,
    pub ps1: f64,
}

impl Ps {
    pub const fn new(ps0: f64, ps1: f64) -> Self {
        Self { ps0, ps1 }
    }

    /// Both lanes set to one value, as scalar single-precision ops leave a register.
    pub const fn splat(x: f64) -> Self {
        Self { ps0: x, ps1: x }
    }

    /// The raw bits of both lanes, for exact comparisons.
    pub fn to_bits(self) -> (u64, u64) {
        (self.ps0.to_bits(), self.ps1.to_bits())
    }
}

fn negate_unless_nan(x: f64) -> f64 {
    if x.is_nan() { x } else { -x }
}

/// `ps_add`
pub fn ps_add(a: Ps, b: Ps) -> Ps {
    Ps::new(frsp(add(a.ps0, b.ps0)), frsp(add(a.ps1, b.ps1)))
}

/// `ps_sub`
pub fn ps_sub(a: Ps, b: Ps) -> Ps {
    Ps::new(frsp(sub(a.ps0, b.ps0)), frsp(sub(a.ps1, b.ps1)))
}

/// `ps_mul`
pub fn ps_mul(a: Ps, c: Ps) -> Ps {
    Ps::new(
        frsp(mul(a.ps0, round_c(c.ps0))),
        frsp(mul(a.ps1, round_c(c.ps1))),
    )
}

/// `ps_div`
pub fn ps_div(a: Ps, b: Ps) -> Ps {
    Ps::new(frsp(div(a.ps0, b.ps0)), frsp(div(a.ps1, b.ps1)))
}

/// `ps_madd`: `a * c + b` per lane.
pub fn ps_madd(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(
        frsp(madd_single(a.ps0, c.ps0, b.ps0, false)),
        frsp(madd_single(a.ps1, c.ps1, b.ps1, false)),
    )
}

/// `ps_msub`: `a * c - b` per lane.
pub fn ps_msub(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(
        frsp(madd_single(a.ps0, c.ps0, b.ps0, true)),
        frsp(madd_single(a.ps1, c.ps1, b.ps1, true)),
    )
}

/// `ps_nmadd`: `-(a * c + b)` per lane.
pub fn ps_nmadd(a: Ps, c: Ps, b: Ps) -> Ps {
    let r = ps_madd(a, c, b);
    Ps::new(negate_unless_nan(r.ps0), negate_unless_nan(r.ps1))
}

/// `ps_nmsub`: `-(a * c - b)` per lane.
pub fn ps_nmsub(a: Ps, c: Ps, b: Ps) -> Ps {
    let r = ps_msub(a, c, b);
    Ps::new(negate_unless_nan(r.ps0), negate_unless_nan(r.ps1))
}

/// `ps_sum0`: `(a.ps0 + b.ps1, c.ps1)`.
pub fn ps_sum0(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(frsp(add(a.ps0, b.ps1)), frsp(c.ps1))
}

/// `ps_sum1`: `(c.ps0, a.ps0 + b.ps1)`.
pub fn ps_sum1(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(frsp(c.ps0), frsp(add(a.ps0, b.ps1)))
}

/// `ps_muls0`: both lanes of `a` times `c.ps0`.
pub fn ps_muls0(a: Ps, c: Ps) -> Ps {
    let c0 = round_c(c.ps0);
    Ps::new(frsp(mul(a.ps0, c0)), frsp(mul(a.ps1, c0)))
}

/// `ps_muls1`: both lanes of `a` times `c.ps1`.
pub fn ps_muls1(a: Ps, c: Ps) -> Ps {
    let c1 = round_c(c.ps1);
    Ps::new(frsp(mul(a.ps0, c1)), frsp(mul(a.ps1, c1)))
}

/// `ps_madds0`: `a * c.ps0 + b` per lane.
pub fn ps_madds0(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(
        frsp(madd_single(a.ps0, c.ps0, b.ps0, false)),
        frsp(madd_single(a.ps1, c.ps0, b.ps1, false)),
    )
}

/// `ps_madds1`: `a * c.ps1 + b` per lane.
pub fn ps_madds1(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(
        frsp(madd_single(a.ps0, c.ps1, b.ps0, false)),
        frsp(madd_single(a.ps1, c.ps1, b.ps1, false)),
    )
}

/// `ps_res`: reciprocal estimate per lane.
pub fn ps_res(b: Ps) -> Ps {
    Ps::new(crate::fres(b.ps0), crate::fres(b.ps1))
}

/// `ps_rsqrte`: reciprocal square root estimate per lane, rounded to single.
pub fn ps_rsqrte(b: Ps) -> Ps {
    Ps::new(frsp(crate::frsqrte(b.ps0)), frsp(crate::frsqrte(b.ps1)))
}

/// `ps_sel`: per lane, `c` if `a >= -0.0`, else `b`.
pub fn ps_sel(a: Ps, c: Ps, b: Ps) -> Ps {
    Ps::new(
        crate::fsel(a.ps0, c.ps0, b.ps0),
        crate::fsel(a.ps1, c.ps1, b.ps1),
    )
}

/// `ps_neg`
pub fn ps_neg(b: Ps) -> Ps {
    Ps::new(fneg(b.ps0), fneg(b.ps1))
}

/// `ps_abs`
pub fn ps_abs(b: Ps) -> Ps {
    Ps::new(fabs(b.ps0), fabs(b.ps1))
}

/// `ps_nabs`
pub fn ps_nabs(b: Ps) -> Ps {
    Ps::new(fnabs(b.ps0), fnabs(b.ps1))
}

/// `ps_merge00`
pub fn ps_merge00(a: Ps, b: Ps) -> Ps {
    Ps::new(a.ps0, b.ps0)
}

/// `ps_merge01`
pub fn ps_merge01(a: Ps, b: Ps) -> Ps {
    Ps::new(a.ps0, b.ps1)
}

/// `ps_merge10`
pub fn ps_merge10(a: Ps, b: Ps) -> Ps {
    Ps::new(a.ps1, b.ps0)
}

/// `ps_merge11`
pub fn ps_merge11(a: Ps, b: Ps) -> Ps {
    Ps::new(a.ps1, b.ps1)
}
