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

    // One that returns with its stack pointer above the one it found (addi r1, r1, 16).
    let ctx = machine(&[addi(1, 1, 16), BLR]);
    ctx.check_conventions(true);
    ctx.run_original(CODE);
    assert_eq!(ctx.check_conventions(false), Some(CODE));
}

#[test]
fn mutated_checks_drop_calls_whose_original_fails() {
    // f(p): q = *p; return q != 0 ? *q : p. The call's p points to 0, which mutated checks
    // change: the original then fails reading memory, and the port another way.
    const P: u32 = 0x8000_5000;
    let ctx = machine(&[
        0x8083_0000, // lwz r4, 0(r3)
        0x2C04_0000, // cmpwi r4, 0
        0x4182_0008, // beq +8
        0x8064_0000, // lwz r3, 0(r4)
        BLR,
    ]);
    ctx.write_u32(P, 0);
    ctx.register_port(
        CODE,
        |ctx| {
            let q = ctx.read_u32(ctx.regs.r(3));
            if q != 0 {
                assert!((0x8000_0000..0x8180_0000).contains(&q), "{q:#X} is no object");
                ctx.regs.set_r(3, ctx.read_u32(q));
            }
        },
        ssbm_rt::lockstep::Returns::Int,
    );
    ctx.set_mode(CODE, Mode::Lockstep);
    ctx.lockstep.mutations.set(50);
    ctx.lockstep.rng.set(1);
    ctx.regs.set_r(3, P);
    ctx.invoke(CODE);
    assert_eq!(ctx.regs.r(3), P);
    assert!(ctx.lockstep.mismatches.borrow().is_empty());
}

#[test]
fn checks_end_once_the_blocks_still_needed_are_verified() {
    // f(n): return n != 0 ? 1 : 2, whose `return 2` block alone is still needed.
    let ctx = machine(&[
        0x2C03_0000, // cmpwi r3, 0
        0x4182_000C, // beq +12
        li(3, 1),
        BLR,
        li(3, 2),
        BLR,
    ]);
    ctx.register_port(
        CODE,
        |ctx| ctx.regs.set_r(3, if ctx.regs.r(3) != 0 { 1 } else { 2 }),
        ssbm_rt::lockstep::Returns::Int,
    );
    ctx.coverage
        .set_bounds(Box::new(|a| (a == CODE).then_some((CODE, CODE + 24))));
    ctx.lockstep
        .needed
        .borrow_mut()
        .insert(CODE, vec![(CODE + 16, CODE + 24)]);
    ctx.set_mode(CODE, Mode::Lockstep);
    for n in [1, 0, 0] {
        ctx.regs.set_r(3, n);
        ctx.invoke(CODE);
    }
    assert_eq!(ctx.lockstep.stats.borrow()[&CODE].calls, 2, "the third call ran unchecked");
    assert!(ctx.coverage.is_covered(CODE + 16));
}

#[test]
fn mutated_checks_drop_calls_that_read_a_saved_register() {
    // f(i): saves r31, stores 5 at 8(r1), returns the word at r1 + i. Changed inputs make i 12,
    // the saved r31's slot, which only the original's frame holds.
    let ctx = machine(&[
        0x9421_FFF0, // stwu r1, -16(r1)
        0x93E1_000C, // stw r31, 12(r1)
        li(4, 5),
        0x9081_0008, // stw r4, 8(r1)
        0x7C61_182E, // lwzx r3, r1, r3
        0x83E1_000C, // lwz r31, 12(r1)
        addi(1, 1, 16),
        BLR,
    ]);
    ctx.register_port(
        CODE,
        |ctx| {
            let i = ctx.regs.r(3);
            let _frame = ctx.stack_frame(16);
            let sp = ctx.regs.r(1);
            ctx.write_u32(sp + 8, 5);
            ctx.regs.set_r(3, ctx.read_u32(sp.wrapping_add(i)));
        },
        ssbm_rt::lockstep::Returns::Int,
    );
    ctx.set_mode(CODE, Mode::Lockstep);
    ctx.lockstep.mutations.set(400);
    ctx.lockstep.rng.set(1);
    ctx.regs.set_r(31, 0x1234);
    ctx.regs.set_r(3, 8);
    ctx.invoke(CODE);
    assert_eq!(ctx.regs.r(3), 5);
    assert!(ctx.lockstep.mismatches.borrow().is_empty());
}

#[test]
fn mutated_checks_drop_calls_to_no_function() {
    // f calls through a pointer into the middle of code no port starts at, as a garbage table
    // entry does: its mutated checks end there, and its real call is checked.
    const MID: u32 = CODE + 0x44;
    let ctx = machine(&[li(3, 7), BLR]);
    ctx.write_u32(MID, li(4, 1));
    ctx.write_u32(MID + 4, BLR);
    ctx.register_port(
        CODE,
        |ctx| {
            ctx.invoke(MID);
            ctx.regs.set_r(3, 7);
        },
        ssbm_rt::lockstep::Returns::Int,
    );
    ctx.set_mode(CODE, Mode::Lockstep);
    ctx.lockstep.mutations.set(8);
    ctx.lockstep.rng.set(1);
    ctx.invoke(CODE);
    assert_eq!(ctx.regs.r(3), 7);
    assert_eq!(ctx.lockstep.stats.borrow()[&CODE].calls, 1, "only the real call ran");
}

#[test]
fn checks_roll_back_the_locked_cache() {
    // f(p): *p += 1, with p in the locked cache. The port runs on what the original found.
    const P: u32 = 0xE000_0000;
    let ctx = machine(&[
        0x8083_0000, // lwz r4, 0(r3)
        addi(4, 4, 1),
        0x9083_0000, // stw r4, 0(r3)
        BLR,
    ]);
    ctx.register_port(
        CODE,
        |ctx| {
            let p = ctx.regs.r(3);
            ctx.write_u32(p, ctx.read_u32(p) + 1);
        },
        ssbm_rt::lockstep::Returns::Nothing,
    );
    ctx.set_mode(CODE, Mode::Lockstep);
    ctx.regs.set_r(3, P);
    ctx.invoke(CODE);
    assert_eq!(ctx.read_u32(P), 1);
    assert!(ctx.lockstep.mismatches.borrow().is_empty());
}

#[test]
fn mutated_checks_drop_calls_whose_port_reads_an_unset_register() {
    // f(n): return n == 1 ? 7 : r5, where r5 holds what code far up left. The port reads it
    // through c::unset_read, as c2rs's ports from machine code do, which ends a mutated check.
    let ctx = machine(&[
        0x2C03_0001, // cmpwi r3, 1
        0x4082_000C, // bne +12
        li(3, 7),
        BLR,
        0x7CA3_2B78, // mr r3, r5
        BLR,
    ]);
    ctx.register_port(
        CODE,
        |ctx| {
            if ctx.regs.r(3) == 1 {
                ctx.regs.set_r(3, 7);
            } else {
                ssbm_rt::cpu::unset_read(ctx);
                ctx.regs.set_r(3, ctx.regs.r(5));
            }
        },
        ssbm_rt::lockstep::Returns::Int,
    );
    ctx.set_mode(CODE, Mode::Lockstep);
    ctx.lockstep.mutations.set(50);
    ctx.lockstep.rng.set(1);
    ctx.regs.set_r(3, 1);
    ctx.invoke(CODE);
    assert_eq!(ctx.regs.r(3), 7);
    assert!(ctx.lockstep.mismatches.borrow().is_empty());
    assert_eq!(ctx.lockstep.stats.borrow()[&CODE].mutated_mismatches, 0);
}

#[test]
fn mutated_checks_leave_code_run_from_ram_as_it_is() {
    // f(p): jumps to p, where `li r3, 5; blr` lies in RAM past the game's code, as playback
    // places injected code; mutated checks change bytes behind p.
    const PLACED: u32 = 0x8060_0000;
    let ctx = machine(&[0x7C89_03A6, 0x4E80_0420]); // mtctr r4; bctr
    ctx.write_u32(PLACED, li(3, 5));
    ctx.write_u32(PLACED + 4, BLR);
    ctx.lockstep.code.set((CODE, CODE + 0x100));
    ctx.lockstep.mutations.set(200);
    ctx.lockstep.rng.set(1);
    ctx.register(CODE, |ctx| ctx.regs.set_r(3, 5));
    ctx.set_mode(CODE, Mode::Lockstep);
    // As ssbm-run does: changed code may loop.
    ctx.set_heartbeat(|ctx, _| {
        if ctx.lockstep.is_mutating() && ctx.lockstep.in_original() {
            std::panic::panic_any(ssbm_rt::lockstep::Runaway);
        }
    });
    ctx.regs.set_r(4, PLACED);
    ctx.invoke(CODE);
    assert!(ctx.lockstep.mismatches.borrow().is_empty());
    assert_eq!(ctx.read_u32(PLACED), li(3, 5));
    assert_eq!(ctx.lockstep.stats.borrow()[&CODE].calls, 201, "every mutated check ran");
}

#[test]
fn mutated_checks_reach_what_a_callee_result_decides() {
    // f: return g() != 0 ? 7 : 9, where g always returns 0.
    const G: u32 = CODE + 0x40;
    let f = [
        0x7C08_02A6, // mflr r0
        0x9001_0004, // stw r0, 4(r1)
        0x9421_FFF0, // stwu r1, -16(r1)
        bl(CODE + 12, G),
        0x2C03_0000, // cmpwi r3, 0
        0x4182_000C, // beq +12
        li(3, 7),
        0x4800_0008, // b +8
        li(3, 9),
        0x8001_0014, // lwz r0, 20(r1)
        addi(1, 1, 16),
        0x7C08_03A6, // mtlr r0
        BLR,
    ];
    let mismatches = |port: fn(&Ctx)| {
        let ctx = machine(&f);
        ctx.write_u32(G, li(3, 0));
        ctx.write_u32(G + 4, BLR);
        ctx.register(CODE, port);
        ctx.register(G, |ctx| ctx.regs.set_r(3, 0));
        ctx.set_mode(CODE, Mode::Lockstep);
        ctx.lockstep.mutations.set(50);
        ctx.lockstep.rng.set(1);
        ctx.lockstep.targets.borrow_mut().insert(CODE, vec![ssbm_rt::lockstep::Target::Call(G)]);
        ctx.invoke(CODE);
        assert_eq!(ctx.regs.r(3), 9, "the call itself goes on unchanged");
        ctx.lockstep.mismatches.borrow().len()
    };
    assert_eq!(mismatches(|ctx| {
        let g: u32 = ctx.call(G, ());
        ctx.regs.set_r(3, if g != 0 { 7 } else { 9 });
    }), 0);
    // A port wrong only where g returns nonzero, which only a stood-in g shows.
    assert!(mismatches(|ctx| {
        let g: u32 = ctx.call(G, ());
        ctx.regs.set_r(3, if g != 0 { 8 } else { 9 });
    }) > 0);
}

#[test]
fn mutated_checks_reach_what_a_loaded_value_decides() {
    // f: return *(u8*) 0x80600010 == 7 ? 7 : 9, where the byte is 0.
    let f = [
        0x3C80_8060, // lis r4, 0x8060
        0x8804_0010, // lbz r0, 0x10(r4)
        0x2C00_0007, // cmpwi r0, 7
        0x4182_000C, // beq +12
        li(3, 9),
        BLR,
        li(3, 7),
        BLR,
    ];
    let mismatches = |port: fn(&Ctx)| {
        let ctx = machine(&f);
        // Its result is r3 alone: r4 is left as lis set it.
        ctx.register_port(CODE, port, ssbm_rt::lockstep::Returns::Int);
        ctx.set_mode(CODE, Mode::Lockstep);
        ctx.lockstep.mutations.set(50);
        ctx.lockstep.rng.set(1);
        let load = ssbm_rt::lockstep::Target::Load { pc: CODE + 4, size: 1, value: 7, bits: false };
        ctx.lockstep.targets.borrow_mut().insert(CODE, vec![load]);
        ctx.invoke(CODE);
        assert_eq!(ctx.regs.r(3), 9, "the call itself goes on unchanged");
        ctx.lockstep.mismatches.borrow().len()
    };
    assert_eq!(mismatches(|ctx| {
        let seven = ctx.read_u8(0x8060_0010) == 7;
        ctx.regs.set_r(3, if seven { 7 } else { 9 });
    }), 0);
    assert!(mismatches(|ctx| {
        let seven = ctx.read_u8(0x8060_0010) == 7;
        ctx.regs.set_r(3, if seven { 8 } else { 9 });
    }) > 0);
}

#[test]
fn mutated_checks_tell_which_nan_an_add_passes_on() {
    // f(a, b): return b + a, which MWCC emits as fadds f1, f2, f1.
    let mismatches = |port: fn(&Ctx)| {
        let ctx = machine(&[0xEC22_082A, BLR]);
        ctx.register_port(CODE, port, ssbm_rt::lockstep::Returns::Float);
        ctx.set_mode(CODE, Mode::Lockstep);
        ctx.lockstep.mutations.set(2000);
        ctx.lockstep.rng.set(1);
        ctx.regs.set_f(1, 1.0);
        ctx.regs.set_f(2, 2.0);
        ctx.invoke(CODE);
        ctx.lockstep.mismatches.borrow().len()
    };
    assert_eq!(mismatches(|ctx| ctx.regs.set_f(1, fp::fadds(ctx.regs.f(2), ctx.regs.f(1)))), 0);
    // The same sum, but of two NaNs the other one's.
    assert!(mismatches(|ctx| ctx.regs.set_f(1, fp::fadds(ctx.regs.f(1), ctx.regs.f(2)))) > 0);
}

#[test]
fn mutated_checks_meet_hardware_that_answers_nothing() {
    // f: return *(u32*) 0xCC006000, a register of a device that counts its reads.
    static READS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    struct Device;
    impl ssbm_rt::Mmio for Device {
        fn read(&self, _: &Ctx, _: u32, _: u32) -> u32 {
            READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            5
        }
        fn write(&self, _: &Ctx, _: u32, _: u32, _: u32) {}
    }
    let ctx = machine(&[0x3C80_CC00, 0x8064_6000, BLR]); // lis r4, 0xCC00; lwz r3, 0x6000(r4)
    ctx.set_mmio(Box::new(Device));
    ctx.register_port(
        CODE,
        |ctx| ctx.regs.set_r(3, ctx.read_u32(0xCC00_6000)),
        ssbm_rt::lockstep::Returns::Int,
    );
    ctx.set_mode(CODE, Mode::Lockstep);
    ctx.lockstep.mutations.set(20);
    ctx.lockstep.rng.set(1);
    ctx.invoke(CODE);
    assert_eq!(ctx.regs.r(3), 5);
    assert!(ctx.lockstep.mismatches.borrow().is_empty());
    // Every mutated check ran to the end, and only the real call reached the device.
    assert_eq!(ctx.lockstep.stats.borrow()[&CODE].calls, 21);
    assert_eq!(READS.load(std::sync::atomic::Ordering::Relaxed), 1);
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
