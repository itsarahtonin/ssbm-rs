// SPDX-License-Identifier: GPL-3.0-or-later

//! Differential fuzz: every float instruction the game uses, run by our interpreter (and so by
//! `gekko-fp`) and by a reference, must give identical register bits and CR. Hardware mode is
//! checked against current Dolphin's interpreter, Slippi mode against what Slippi's Ishiiruka
//! JIT emits. Set `FP_FUZZ_ITERS` to raise the per-instruction case count.

use fp_fuzz::{Fprs, Variant};
use gekko_fp::{self as fp, FpMode, Ps};
use ssbm_rt::Ctx;

const FD: u32 = 0;
const FA: u32 = 1;
const FB: u32 = 2;
const FC: u32 = 3;

fn a_form(op: u32, xo: u32) -> u32 {
    (op << 26) | (FD << 21) | (FA << 16) | (FB << 11) | (FC << 6) | (xo << 1)
}

fn x_form(op: u32, xo: u32) -> u32 {
    (op << 26) | (FD << 21) | (FA << 16) | (FB << 11) | (xo << 1)
}

/// `fcmpu`-style compares write CR field 3 so the result is visible in CR.
fn cmp_form(op: u32, xo: u32) -> u32 {
    (op << 26) | (3 << 23) | (FA << 16) | (FB << 11) | (xo << 1)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Result depends on the fused-multiply-add mode.
    Fused,
    Plain,
}

struct Op {
    name: &'static str,
    word: u32,
    kind: Kind,
}

fn ops() -> Vec<Op> {
    use Kind::*;
    let mut v = Vec::new();
    let mut add = |name, word, kind| v.push(Op { name, word, kind });
    // Single precision.
    add("fdivs", a_form(59, 18), Plain);
    add("fsubs", a_form(59, 20), Plain);
    add("fadds", a_form(59, 21), Plain);
    add("fres", a_form(59, 24), Plain);
    add("fmuls", a_form(59, 25), Plain);
    add("fmsubs", a_form(59, 28), Fused);
    add("fmadds", a_form(59, 29), Fused);
    add("fnmsubs", a_form(59, 30), Fused);
    add("fnmadds", a_form(59, 31), Fused);
    // Double precision.
    add("fdiv", a_form(63, 18), Plain);
    add("fsub", a_form(63, 20), Plain);
    add("fadd", a_form(63, 21), Plain);
    add("fsel", a_form(63, 23), Plain);
    add("fmul", a_form(63, 25), Plain);
    add("frsqrte", a_form(63, 26), Plain);
    add("fmsub", a_form(63, 28), Plain);
    add("fmadd", a_form(63, 29), Plain);
    add("fnmsub", a_form(63, 30), Plain);
    add("fnmadd", a_form(63, 31), Plain);
    add("fcmpu", cmp_form(63, 0), Plain);
    add("fcmpo", cmp_form(63, 32), Plain);
    add("frsp", x_form(63, 12), Plain);
    add("fctiw", x_form(63, 14), Plain);
    add("fctiwz", x_form(63, 15), Plain);
    add("fneg", x_form(63, 40), Plain);
    add("fmr", x_form(63, 72), Plain);
    add("fnabs", x_form(63, 136), Plain);
    add("fabs", x_form(63, 264), Plain);
    // Paired singles.
    add("ps_sum0", a_form(4, 10), Plain);
    add("ps_sum1", a_form(4, 11), Plain);
    add("ps_muls0", a_form(4, 12), Plain);
    add("ps_muls1", a_form(4, 13), Plain);
    add("ps_madds0", a_form(4, 14), Fused);
    add("ps_madds1", a_form(4, 15), Fused);
    add("ps_div", a_form(4, 18), Plain);
    add("ps_sub", a_form(4, 20), Plain);
    add("ps_add", a_form(4, 21), Plain);
    add("ps_sel", a_form(4, 23), Plain);
    add("ps_res", a_form(4, 24), Plain);
    add("ps_mul", a_form(4, 25), Plain);
    add("ps_rsqrte", a_form(4, 26), Plain);
    add("ps_msub", a_form(4, 28), Fused);
    add("ps_madd", a_form(4, 29), Fused);
    add("ps_nmsub", a_form(4, 30), Fused);
    add("ps_nmadd", a_form(4, 31), Fused);
    add("ps_cmpu0", cmp_form(4, 0), Plain);
    add("ps_cmpo0", cmp_form(4, 32), Plain);
    add("ps_cmpu1", cmp_form(4, 64), Plain);
    add("ps_cmpo1", cmp_form(4, 96), Plain);
    add("ps_neg", x_form(4, 40), Plain);
    add("ps_mr", x_form(4, 72), Plain);
    add("ps_nabs", x_form(4, 136), Plain);
    add("ps_abs", x_form(4, 264), Plain);
    add("ps_merge00", x_form(4, 528), Plain);
    add("ps_merge01", x_form(4, 560), Plain);
    add("ps_merge10", x_form(4, 592), Plain);
    add("ps_merge11", x_form(4, 624), Plain);
    v
}

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The double a register holds after `lfs` of these single bits.
fn single(bits: u32) -> u64 {
    fp::lfs(bits).to_bits()
}

const SPECIALS: &[u64] = &[
    0x0000_0000_0000_0000, // +0
    0x8000_0000_0000_0000, // -0
    0x7FF0_0000_0000_0000, // +inf
    0xFFF0_0000_0000_0000, // -inf
    0x7FF8_0000_0000_0000, // default quiet NaN
    0xFFF8_0000_0000_0000,
    0x7FF4_0000_0000_0001, // signaling NaN with payload
    0xFFF0_0000_0000_0001,
    0x7FFC_0000_2000_0000, // quiet NaN with payload
    0x3FF0_0000_0000_0000, // 1
    0xBFF0_0000_0000_0000, // -1
    0x3FE0_0000_0000_0000, // 0.5
    0x4000_0000_0000_0000, // 2
    0x47EF_FFFF_E000_0000, // FLT_MAX
    0x47EF_FFFF_F000_0000, // FLT_MAX + half ulp (rounds to inf as single)
    0x47F0_0000_0000_0000, // 2^128
    0x3810_0000_0000_0000, // FLT_MIN
    0x380F_FFFF_F000_0000, // just below FLT_MIN
    0x36A0_0000_0000_0000, // smallest single denormal
    0x3690_0000_0000_0000, // half the smallest single denormal
    0x3698_0000_0000_0000,
    0x0000_0000_0000_0001, // smallest double denormal
    0x000F_FFFF_FFFF_FFFF, // largest double denormal
    0x7FEF_FFFF_FFFF_FFFF, // DBL_MAX
    0x41E0_0000_0000_0000, // 2^31
    0xC1E0_0000_0000_0000, // -2^31
    0x41DF_FFFF_FFC0_0000, // 2^31 - 1
    0xC1E0_0000_0020_0000, // -2^31 - 1
    0x3FE0_0000_0000_0001, // just above 0.5
    0x4004_0000_0000_0000, // 2.5
    0xC004_0000_0000_0000, // -2.5
];

fn value(rng: &mut Rng) -> u64 {
    match rng.below(10) {
        0..=2 => single(rng.next() as u32),
        3 => rng.next(),
        4 => {
            // Typical game magnitudes.
            let x = (rng.next() as i64 as f64) / (1u64 << (40 + rng.below(20))) as f64;
            single(fp::stfs(x))
        }
        5 => SPECIALS[rng.below(SPECIALS.len() as u64) as usize],
        6 => {
            // A double near a single: exercises frsp and the frC rounding of fused ops.
            single(rng.next() as u32) ^ (rng.next() & 0x1FFF_FFFF)
        }
        7 => ((rng.below(64) as f64) - 32.0).to_bits(),
        8 => {
            // Single denormals and values around FLT_MIN.
            let exp = rng.below(3) as u32;
            single((rng.next() as u32 & 0x807F_FFFF) | (exp << 23))
        }
        _ => {
            let x = single(rng.next() as u32);
            if rng.below(2) == 0 { x } else { x ^ (1 << 63) }
        }
    }
}

/// Inputs for fused ops whose exact product sits on a single-precision tie, plus an addend far
/// below the product, so rounding to double first and then to single rounds the wrong way.
fn fused_tie(rng: &mut Rng) -> (u64, u64, u64) {
    loop {
        let a_odd = (rng.below(1 << 12) as u32) | (1 << 12) | 1;
        let c_odd = (rng.below(1 << 12) as u32) | (1 << 12) | 1;
        let p = u64::from(a_odd) * u64::from(c_odd);
        if p >> 24 != 1 {
            continue;
        }
        let ea = rng.below(40) as i32 - 20;
        let ec = rng.below(40) as i32 - 20;
        let a = f64::from(a_odd) * 2f64.powi(ea);
        let c = f64::from(c_odd) * 2f64.powi(ec);
        let prod = a * c;
        let tiny_exp = (prod.abs().log2().floor() as i32) - 60 - rng.below(20) as i32;
        let mut b = 2f64.powi(tiny_exp);
        if rng.below(2) == 0 {
            b = -b;
        }
        let a = if rng.below(2) == 0 { a } else { -a };
        let c = if rng.below(2) == 0 { c } else { -c };
        return (a.to_bits(), b.to_bits(), c.to_bits());
    }
}

fn inputs(rng: &mut Rng, fused: bool) -> Fprs {
    let mut fpr = [(0u64, 0u64); 32];
    fpr[FD as usize] = (rng.next(), rng.next());
    for r in [FA, FB, FC] {
        fpr[r as usize] = (value(rng), value(rng));
    }
    if fused && rng.below(4) == 0 {
        let (a, b, c) = fused_tie(rng);
        fpr[FA as usize].0 = a;
        fpr[FB as usize].0 = b;
        fpr[FC as usize].0 = c;
        let (a, b, c) = fused_tie(rng);
        fpr[FA as usize].1 = a;
        fpr[FB as usize].1 = b;
        fpr[FC as usize].1 = c;
    }
    fpr
}

fn is_nan(bits: u64) -> bool {
    f64::from_bits(bits).is_nan()
}

/// Whether our frD matches the reference's. Against Ishiiruka, a NaN matches any NaN, since
/// which operand an x86 NaN comes from depends on the JIT's register allocation. So does the
/// high lane of scalar double arithmetic and of the sign ops: the JIT leaves whatever its
/// scratch register held there, and game code never reads it.
fn matches(variant: Variant, word: u32, theirs: (u64, u64), ours: (u64, u64)) -> bool {
    if variant == Variant::Master {
        return theirs == ours;
    }
    let same = |a: u64, b: u64| a == b || (is_nan(a) && is_nan(b));
    let (op, sub5, sub10) = (word >> 26, (word >> 1) & 31, (word >> 1) & 1023);
    let high_undefined =
        op == 63 && (matches!(sub5, 18 | 20 | 21 | 25) || matches!(sub10, 40 | 136 | 264));
    same(theirs.0, ours.0) && (high_undefined || same(theirs.1, ours.1))
}

fn run_ours(ctx: &Ctx, word: u32, fpr: &Fprs) -> (Fprs, u32) {
    for (i, (ps0, ps1)) in fpr.iter().enumerate() {
        ctx.regs.fpr[i].set(Ps::new(f64::from_bits(*ps0), f64::from_bits(*ps1)));
    }
    ctx.regs.cr.set(0);
    ctx.regs.fpscr.set(0);
    ssbm_ppc::step(ctx, 0x8000_0000, word);
    let mut out = [(0, 0); 32];
    for (i, reg) in out.iter_mut().enumerate() {
        let (ps0, ps1) = ctx.regs.fpr[i].get().to_bits();
        *reg = (ps0, ps1);
    }
    (out, ctx.regs.cr.get())
}

fn iters() -> u64 {
    std::env::var("FP_FUZZ_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000)
}

fn fuzz(variant: Variant, mode: FpMode, seed: u64) {
    fp::set_fp_mode(mode);
    let ctx = Ctx::new();
    let mut rng = Rng(seed);
    let n = iters();
    let mut failures = Vec::new();
    for op in ops() {
        let mut bad = 0u64;
        for _ in 0..n {
            let fpr = inputs(&mut rng, op.kind == Kind::Fused);
            let mut theirs = fpr;
            let their_cr = variant.exec(op.word, &mut theirs);
            let (ours, our_cr) = run_ours(&ctx, op.word, &fpr);
            if !matches(variant, op.word, theirs[FD as usize], ours[FD as usize])
                || their_cr != our_cr
            {
                bad += 1;
                if bad <= 3 {
                    let r = |i: u32| fpr[i as usize];
                    failures.push(format!(
                        "{}: a={:016X?} b={:016X?} c={:016X?} d0={:016X?} -> reference {:016X?} cr {their_cr:08X}, ours {:016X?} cr {our_cr:08X}",
                        op.name,
                        r(FA),
                        r(FB),
                        r(FC),
                        r(FD),
                        theirs[FD as usize],
                        ours[FD as usize],
                    ));
                }
            }
        }
        if bad > 0 {
            failures.push(format!("{}: {bad} of {n} cases differ", op.name));
        }
    }
    assert!(
        failures.is_empty(),
        "{variant:?} / {mode:?} mismatches:\n{}",
        failures.join("\n")
    );
}

/// Every combination of special values in frA, frB and frC, in both halves.
fn specials_exhaustive(variant: Variant, mode: FpMode) {
    fp::set_fp_mode(mode);
    let ctx = Ctx::new();
    let mut failures = Vec::new();
    for op in ops() {
        let mut bad = 0u64;
        for &a in SPECIALS {
            for &b in SPECIALS {
                for &c in SPECIALS {
                    let mut fpr = [(0u64, 0u64); 32];
                    fpr[FD as usize] = (0x1234_5678_9ABC_DEF0, 0x0FED_CBA9_8765_4321);
                    fpr[FA as usize] = (a, c);
                    fpr[FB as usize] = (b, a);
                    fpr[FC as usize] = (c, b);
                    let mut theirs = fpr;
                    let their_cr = variant.exec(op.word, &mut theirs);
                    let (ours, our_cr) = run_ours(&ctx, op.word, &fpr);
                    if !matches(variant, op.word, theirs[FD as usize], ours[FD as usize])
                        || their_cr != our_cr
                    {
                        bad += 1;
                        if bad <= 3 {
                            failures.push(format!(
                                "{}: a={a:016X} b={b:016X} c={c:016X} -> reference {:016X?}, ours {:016X?}",
                                op.name, theirs[FD as usize], ours[FD as usize]
                            ));
                        }
                    }
                }
            }
        }
        if bad > 0 {
            failures.push(format!("{}: {bad} special cases differ", op.name));
        }
    }
    assert!(
        failures.is_empty(),
        "{variant:?} / {mode:?} special-value mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn hardware_mode_specials_match_dolphin_master() {
    specials_exhaustive(Variant::Master, FpMode::Hardware);
}

#[test]
fn slippi_mode_specials_match_ishiiruka() {
    specials_exhaustive(Variant::Ishiiruka, FpMode::Slippi);
}

#[test]
fn hardware_mode_matches_dolphin_master() {
    fuzz(Variant::Master, FpMode::Hardware, 0x9E37_79B9_7F4A_7C15);
}

#[test]
fn slippi_mode_matches_ishiiruka() {
    fuzz(Variant::Ishiiruka, FpMode::Slippi, 0xD1B5_4A32_D192_ED03);
}

#[test]
fn load_store_conversions_match() {
    let mut rng = Rng(0x94D0_49BB_1331_11EB);
    for (variant, mode) in [
        (Variant::Master, FpMode::Hardware),
        (Variant::Ishiiruka, FpMode::Slippi),
    ] {
        fp::set_fp_mode(mode);
        for i in 0..iters() * 20 {
            let s = if i < 1 << 16 {
                // Every exponent and sign with random mantissas, then anything.
                ((i as u32) << 16) | (rng.next() as u32 & 0xFFFF)
            } else {
                rng.next() as u32
            };
            assert_eq!(
                fp::lfs(s).to_bits(),
                variant.convert_to_double(s),
                "{variant:?} lfs {s:08X}"
            );
            let d = value(&mut rng);
            assert_eq!(
                fp::stfs(f64::from_bits(d)),
                variant.convert_to_single(d),
                "{variant:?} stfs {d:016X}"
            );
            assert_eq!(
                fp::stfs_ftz(f64::from_bits(d)),
                variant.convert_to_single_ftz(d),
                "{variant:?} stfs_ftz {d:016X}"
            );
        }
    }
}
