// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::Rng;
use gekko_fp::*;

fn random_ps(rng: &mut Rng) -> Ps {
    Ps::new(
        f64::from(rng.f32_in(100, 150)),
        f64::from(rng.f32_in(100, 150)),
    )
}

fn same(a: Ps, b: Ps) -> bool {
    a.to_bits() == b.to_bits()
}

#[test]
fn lanes_match_scalar_ops() {
    let mut rng = Rng(0x9A1E_D000_0000_0001);
    for _ in 0..20_000 {
        let (a, b, c) = (
            random_ps(&mut rng),
            random_ps(&mut rng),
            random_ps(&mut rng),
        );
        let lanes = |op: fn(f64, f64, f64) -> f64| {
            Ps::new(op(a.ps0, c.ps0, b.ps0), op(a.ps1, c.ps1, b.ps1))
        };
        assert!(same(
            ps_add(a, b),
            Ps::new(fadds(a.ps0, b.ps0), fadds(a.ps1, b.ps1))
        ));
        assert!(same(
            ps_sub(a, b),
            Ps::new(fsubs(a.ps0, b.ps0), fsubs(a.ps1, b.ps1))
        ));
        assert!(same(
            ps_mul(a, c),
            Ps::new(fmuls(a.ps0, c.ps0), fmuls(a.ps1, c.ps1))
        ));
        assert!(same(
            ps_div(a, b),
            Ps::new(fdivs(a.ps0, b.ps0), fdivs(a.ps1, b.ps1))
        ));
        assert!(same(ps_madd(a, c, b), lanes(fmadds)));
        assert!(same(ps_msub(a, c, b), lanes(fmsubs)));
        assert!(same(ps_nmadd(a, c, b), lanes(fnmadds)));
        assert!(same(ps_nmsub(a, c, b), lanes(fnmsubs)));
    }
}

#[test]
fn scalar_lane_variants_pick_the_right_lanes() {
    let a = Ps::new(1.5, 2.5);
    let b = Ps::new(10.0, 20.0);
    let c = Ps::new(3.0, 5.0);
    assert!(same(ps_sum0(a, c, b), Ps::new(21.5, 5.0)));
    assert!(same(ps_sum1(a, c, b), Ps::new(3.0, 21.5)));
    assert!(same(ps_muls0(a, c), Ps::new(4.5, 7.5)));
    assert!(same(ps_muls1(a, c), Ps::new(7.5, 12.5)));
    assert!(same(ps_madds0(a, c, b), Ps::new(14.5, 27.5)));
    assert!(same(ps_madds1(a, c, b), Ps::new(17.5, 32.5)));
}

#[test]
fn merges_move_lanes_without_rounding() {
    let a = Ps::new(1.0 + 2f64.powi(-40), 2.0);
    let b = Ps::new(3.0, 4.0);
    assert!(same(ps_merge00(a, b), Ps::new(a.ps0, 3.0)));
    assert!(same(ps_merge01(a, b), Ps::new(a.ps0, 4.0)));
    assert!(same(ps_merge10(a, b), Ps::new(2.0, 3.0)));
    assert!(same(ps_merge11(a, b), Ps::new(2.0, 4.0)));
}

#[test]
fn estimates_follow_their_scalar_forms() {
    let b = Ps::new(2.0, 0.3);
    assert!(same(ps_res(b), Ps::new(fres(2.0), fres(0.3))));
    assert!(same(
        ps_rsqrte(b),
        Ps::new(frsp(frsqrte(2.0)), frsp(frsqrte(0.3)))
    ));
    assert!(same(
        ps_sel(Ps::new(-0.0, -1.0), Ps::splat(1.0), Ps::splat(2.0)),
        Ps::new(1.0, 2.0)
    ));
}
