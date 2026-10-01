// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

const CODE: u32 = 0x8000_3100;

fn addi(d: u32, a: u32, imm: i16) -> u32 {
    (14 << 26) | (d << 21) | (a << 16) | u32::from(imm as u16)
}
fn li(d: u32, imm: i16) -> u32 {
    addi(d, 0, imm)
}
fn add(d: u32, a: u32, b: u32) -> u32 {
    (31 << 26) | (d << 21) | (a << 16) | (b << 11) | (266 << 1)
}
fn mtctr(s: u32) -> u32 {
    (31 << 26) | (s << 21) | (9 << 16) | (467 << 1)
}
fn bdnz(disp: i16) -> u32 {
    (16 << 26) | (16 << 21) | u32::from(disp as u16 & 0xFFFC)
}
fn bl(from: u32, to: u32) -> u32 {
    (18 << 26) | (to.wrapping_sub(from) & 0x03FF_FFFC) | 1
}
const BLR: u32 = 0x4E80_0020;

fn machine(code: &[u32]) -> Ctx {
    let ctx = Ctx::new();
    ctx.set_backend(Box::new(Interpreter::default()));
    for (i, w) in code.iter().enumerate() {
        ctx.write_u32(CODE + 4 * i as u32, *w);
    }
    ctx.regs.set_r(1, 0x8100_0000);
    ctx
}

#[test]
fn runs_straight_line_code() {
    let ctx = machine(&[li(3, 5), li(4, 7), add(3, 3, 4), BLR]);
    ctx.run_original(CODE);
    assert_eq!(ctx.regs.r(3), 12);
}

#[test]
fn counts_down_ctr_loops() {
    let ctx = machine(&[li(3, 0), li(4, 10), mtctr(4), addi(3, 3, 2), bdnz(-4), BLR]);
    ctx.run_original(CODE);
    assert_eq!(ctx.regs.r(3), 20);
}

const NATIVE: u32 = 0x8000_4000;

#[test]
fn calls_hand_off_to_natives() {
    let body = [
        0x7C08_02A6, // mflr r0
        0x9001_0004, // stw r0, 4(r1)
        0x9421_FFF0, // stwu r1, -16(r1)
        li(3, 20),
        bl(CODE + 16, NATIVE),
        addi(3, 3, 1),
        0x8001_0014, // lwz r0, 20(r1)
        addi(1, 1, 16),
        0x7C08_03A6, // mtlr r0
        BLR,
    ];
    let ctx = machine(&body);
    ctx.register(NATIVE, |ctx| ctx.regs.set_r(3, ctx.regs.r(3) * 2));
    ctx.run_original(CODE);
    assert_eq!(ctx.regs.r(3), 41);
    assert_eq!(ctx.regs.r(1), 0x8100_0000, "stack balanced");
}

#[test]
fn rotates_and_shifts_like_powerpc() {
    // rlwinm r3, r4, 8, 16, 23 ; srawi r5, r6, 4 ; extsb r7, r8
    let rlwinm = (21 << 26) | (4 << 21) | (3 << 16) | (8 << 11) | (16 << 6) | (23 << 1);
    let srawi = (31 << 26) | (6 << 21) | (5 << 16) | (4 << 11) | (824 << 1);
    let extsb = (31 << 26) | (8 << 21) | (7 << 16) | (954 << 1);
    let ctx = machine(&[rlwinm, srawi, extsb, BLR]);
    ctx.regs.set_r(4, 0x1234_5678);
    ctx.regs.set_r(6, 0xFFFF_FFF1);
    ctx.regs.set_r(8, 0x0000_0080);
    ctx.run_original(CODE);
    assert_eq!(ctx.regs.r(3), 0x0000_7800);
    assert_eq!(ctx.regs.r(5), 0xFFFF_FFFF);
    assert_eq!(
        ctx.regs.xer.get() & XER_CA,
        XER_CA,
        "srawi of a negative with lost bits sets CA"
    );
    assert_eq!(ctx.regs.r(7), 0xFFFF_FF80);
}

#[test]
fn float_ops_use_gekko_semantics() {
    // lfs f2,0(r3); lfs f3,4(r3); lfs f4,8(r3); fmadds f1,f2,f3,f4; stfs f1,12(r3)
    let lfs = |f: u32, d: u32| (48 << 26) | (f << 21) | (3 << 16) | d;
    let fmadds = (59 << 26) | (1 << 21) | (2 << 16) | (4 << 11) | (3 << 6) | (29 << 1);
    let stfs = (52 << 26) | (1 << 21) | (3 << 16) | 12;
    let ctx = machine(&[lfs(2, 0), lfs(3, 4), lfs(4, 8), fmadds, stfs, BLR]);
    let data = 0x8000_5000;
    ctx.regs.set_r(3, data);
    for (i, v) in [0x4248_0000u32, 0xBC88_CC38, 0x1B1C_72A0]
        .iter()
        .enumerate()
    {
        ctx.write_u32(data + 4 * i as u32, *v);
    }
    ctx.run_original(CODE);
    assert_eq!(
        ctx.read_u32(data + 12),
        0xBF55_BF17,
        "hardware fmadds rounding"
    );
}

#[test]
fn quantized_loads_scale() {
    // psq_l f1, 0(r3), 0, qr2 with GQR2 = u8 scaled by 2^-4
    let psq_l = (56 << 26) | (1 << 21) | (3 << 16) | (2 << 12);
    let ctx = machine(&[psq_l, BLR]);
    ctx.regs.gqr[2].set((4 << 24) | (4 << 16));
    ctx.regs.set_r(3, 0x8000_5000);
    ctx.write_u16(0x8000_5000, 0x10FF);
    ctx.run_original(CODE);
    let ps = ctx.regs.fpr[1].get();
    assert_eq!((ps.ps0, ps.ps1), (1.0, 255.0 / 16.0));
}

#[test]
fn notes_calls_that_break_the_calling_convention() {
    // stwu r1, -16(r1); stw r31, 12(r1); ...; lwz r31, 12(r1); addi r1, r1, 16; blr
    let keeps = [0x9421_FFF0, 0x93E1_000C, li(31, 7), 0x83E1_000C, addi(1, 1, 16), BLR];
    let ctx = machine(&keeps);
    ctx.regs.set_r(31, 42);
    ctx.check_conventions(true);
    ctx.run_original(CODE);
    assert_eq!(ctx.check_conventions(false), None);
    assert_eq!(ctx.regs.r(31), 42);

    // The same with r31's saved copy written over (stw r0, 12(r1)) before it comes back.
    let smashes = [
        0x9421_FFF0,
        0x93E1_000C,
        li(0, 99),
        0x9001_000C,
        0x83E1_000C,
        addi(1, 1, 16),
        BLR,
    ];
    let ctx = machine(&smashes);
    ctx.regs.set_r(31, 42);
    ctx.check_conventions(true);
    ctx.run_original(CODE);
    assert_eq!(ctx.check_conventions(false), Some(CODE));
}

#[test]
fn lockstep_flags_a_wrong_port() {
    let body = [li(3, 5), 0x9061_0000 | 0x2000, BLR]; // stw r3, 0x2000(r1)
    let good = machine(&body);
    good.register(CODE, |ctx| {
        ctx.regs.set_r(3, 5);
        ctx.write_u32(ctx.regs.r(1) + 0x2000, 5);
    });
    good.set_mode(CODE, Mode::Lockstep);
    good.invoke(CODE);
    assert!(good.lockstep.mismatches.borrow().is_empty());

    let bad = machine(&body);
    bad.register(CODE, |ctx| {
        ctx.regs.set_r(3, 6);
        ctx.write_u32(ctx.regs.r(1) + 0x2000, 6);
    });
    bad.set_mode(CODE, Mode::Lockstep);
    bad.invoke(CODE);
    let mismatches = bad.lockstep.mismatches.borrow();
    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].diffs.len(), 2, "r3 and the stored word");
    assert_eq!(
        bad.regs.r(3),
        5,
        "execution continues with the original's results"
    );
    assert_eq!(bad.read_u32(0x8100_2000), 5);
}
