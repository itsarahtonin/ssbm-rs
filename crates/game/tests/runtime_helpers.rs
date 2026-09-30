// SPDX-License-Identifier: GPL-3.0-or-later

//! The hand ports of MWCC's runtime helpers against the original code, run by the interpreter
//! on the same inputs. Runs only when `SSBM_DISC` points at a disc image.

use ssbm_disc::Disc;
use ssbm_game::manual::Runtime__runtime as rt;
use ssbm_ppc::Interpreter;
use ssbm_rt::Ctx;
use ssbm_types::fns::addr;

fn machine() -> Option<Ctx> {
    let path = std::env::var_os("SSBM_DISC")?;
    let disc = Disc::open(path).expect("SSBM_DISC should be Melee NTSC 1.02");
    let ctx = Ctx::new();
    ctx.set_backend(Box::new(Interpreter::default()));
    disc.main_dol().unwrap().load_into(&ctx.mem).unwrap();
    ctx.regs.set_r(1, 0x8100_0000);
    Some(ctx)
}

/// xorshift64*, for inputs that cover every magnitude.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value with a random number of significant bits, so both halves and small values
    /// come up often.
    fn wide(&mut self) -> u64 {
        let bits = self.next() % 65;
        let v = if bits == 64 { self.next() } else { self.next() & ((1u64 << bits) - 1) };
        if self.next() % 2 == 0 { v } else { v.wrapping_neg() }
    }

    fn double(&mut self) -> f64 {
        match self.next() % 8 {
            0 => f64::from_bits(self.next()),
            1 => [0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.5, -0.5, 1.0][(self.next() % 8) as usize],
            _ => {
                let exp = (self.next() % 140) as i32 - 30;
                let m = (self.next() >> 11) as f64 / (1u64 << 53) as f64 + 1.0;
                let v = m * 2f64.powi(exp);
                if self.next() % 2 == 0 { v } else { -v }
            }
        }
    }
}

const ROUNDS: usize = 4000;

#[test]
fn division_helpers_match_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    // The original really runs: it leaves the quotient where the dividend was.
    assert_eq!(ctx.call::<_, u64>(addr::__div2u, (100u64, 7u64)), 14);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..ROUNDS {
        let (a, b) = (rng.wide(), if i % 50 == 0 { 0 } else { rng.wide() });
        let want: u64 = ctx.call(addr::__div2u, (a, b));
        assert_eq!(rt::__div2u(&ctx, a, b), want, "__div2u({a:#x}, {b:#x})");
        let want: u64 = ctx.call(addr::__mod2u, (a, b));
        assert_eq!(rt::__mod2u(&ctx, a, b), want, "__mod2u({a:#x}, {b:#x})");
        let (a, b) = (a as i64, b as i64);
        let want: i64 = ctx.call(addr::__div2i, (a, b));
        assert_eq!(rt::__div2i(&ctx, a, b), want, "__div2i({a:#x}, {b:#x})");
        let want: i64 = ctx.call(addr::__mod2i, (a, b));
        assert_eq!(rt::__mod2i(&ctx, a, b), want, "__mod2i({a:#x}, {b:#x})");
    }
}

#[test]
fn shift_helpers_match_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for _ in 0..ROUNDS {
        let a = rng.wide();
        let n = (rng.next() % 140) as i32 - 6;
        let want: i64 = ctx.call(addr::__shl2i, (a as i64, n));
        assert_eq!(rt::__shl2i(&ctx, a as i64, n), want, "__shl2i({a:#x}, {n})");
        let want: i64 = ctx.call(addr::__shr2i, (a as i64, n));
        assert_eq!(rt::__shr2i(&ctx, a as i64, n), want, "__shr2i({a:#x}, {n})");
        let want: u64 = ctx.call(addr::__shr2u, (a, n));
        assert_eq!(rt::__shr2u(&ctx, a, n), want, "__shr2u({a:#x}, {n})");
    }
}

#[test]
fn conversion_helpers_match_the_original() {
    let Some(ctx) = machine() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let mut rng = Rng(0x94D0_49BB_1331_11EB);
    for _ in 0..ROUNDS {
        let x = rng.wide() as i64;
        let want: f64 = ctx.call(addr::__cvt_sll_flt, (x,));
        let got = rt::__cvt_sll_flt(&ctx, x);
        assert_eq!(got.to_bits(), want.to_bits(), "__cvt_sll_flt({x:#x})");
        let d = rng.double();
        let want: u64 = ctx.call(addr::__cvt_dbl_usll, (d,));
        assert_eq!(rt::__cvt_dbl_usll(&ctx, d), want, "__cvt_dbl_usll({d:e})");
        let want: u32 = ctx.call(addr::__cvt_fp2unsigned, (d,));
        assert_eq!(rt::__cvt_fp2unsigned(&ctx, d), want, "__cvt_fp2unsigned({d:e})");
    }
}
