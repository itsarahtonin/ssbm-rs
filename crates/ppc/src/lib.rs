// SPDX-License-Identifier: GPL-3.0-or-later
// Instruction semantics follow Dolphin's interpreter (GPL-2.0-or-later).

//! Gekko interpreter for dev builds. It runs the original code from memory and hands every
//! call to an address with a native implementation over to dispatch, so ports and the
//! original interleave freely. Exceptions and interrupts are not emulated: the SDK layer
//! stands in for the code that would handle them.

use std::cell::{Cell, RefCell};

use gekko_fp::{self as fp, Ps};
use ssbm_rt::cpu::*;
use ssbm_rt::{Backend, Ctx, FLAG_HOOK, FLAG_NATIVE, HEARTBEAT, Mode, RETURN_SENTINEL, spr};

/// Runs original code. Counts instructions for profiling.
#[derive(Default)]
pub struct Interpreter {
    pub executed: Cell<u64>,
    /// The instruction being executed, for crash reports.
    pub pc: Cell<u32>,
    /// The latest jumps, as (from, to), for crash reports.
    jumps: RefCell<[(u32, u32); JUMPS]>,
    next_jump: Cell<usize>,
}

const JUMPS: usize = 32;

impl Interpreter {
    /// The latest jumps, oldest first.
    pub fn recent_jumps(&self) -> Vec<(u32, u32)> {
        let jumps = self.jumps.borrow();
        let at = self.next_jump.get();
        (0..JUMPS)
            .map(|i| jumps[(at + i) % JUMPS])
            .filter(|j| j.1 != 0 || j.0 != 0)
            .collect()
    }
}

impl Backend for Interpreter {
    fn run(&self, ctx: &Ctx, addr: u32) {
        let ret = ctx.regs.lr.get();
        ctx.regs.lr.set(RETURN_SENTINEL);
        ctx.convention_call(addr, RETURN_SENTINEL);
        let end = self.resume_to_sentinel(ctx, addr);
        // A code that returns past its function's caller returns past the sentinel: the port
        // that called continues as far past its own return address.
        if end != RETURN_SENTINEL {
            ctx.resume_at(ret.wrapping_add(end - RETURN_SENTINEL));
        }
    }

    fn resume(&self, ctx: &Ctx, pc: u32) {
        self.resume_to_sentinel(ctx, pc);
    }

    fn executed(&self) -> u64 {
        self.executed.get()
    }
}

impl Interpreter {
    /// Runs from `pc` until a return to the sentinel, or past it; returns where.
    fn resume_to_sentinel(&self, ctx: &Ctx, mut pc: u32) -> u32 {
        while pc < RETURN_SENTINEL {
            if ctx.flags_at(pc) & FLAG_HOOK != 0 {
                ctx.run_hook(pc);
            }
            if ctx.coverage.recording() {
                ctx.coverage.hit(pc);
            }
            self.pc.set(pc);
            let w = ctx.fetch_u32(pc);
            if ctx.lockstep.watching() {
                ctx.lockstep.note_load(ctx, pc, w);
            }
            let from = pc;
            pc = step(ctx, pc, w);
            if pc != from.wrapping_add(4) {
                let i = self.next_jump.get();
                self.jumps.borrow_mut()[i] = (from, pc);
                self.next_jump.set((i + 1) % JUMPS);
            }
            // Heartbeats count every instruction run so far, in this run and the ones nested
            // in it through ported code, so they come at a steady rate.
            let n = self.executed.get() + 1;
            self.executed.set(n);
            if n.is_multiple_of(HEARTBEAT) {
                ctx.beat(pc);
            }
        }
        pc
    }
}


#[inline]
fn fd(w: u32) -> usize {
    ((w >> 21) & 31) as usize
}
#[inline]
fn fa(w: u32) -> usize {
    ((w >> 16) & 31) as usize
}
#[inline]
fn fb(w: u32) -> usize {
    ((w >> 11) & 31) as usize
}
#[inline]
fn fc(w: u32) -> usize {
    ((w >> 6) & 31) as usize
}
#[inline]
fn simm(w: u32) -> u32 {
    w as u16 as i16 as i32 as u32
}
/// The signed 12-bit displacement of `psq_l`/`psq_st`.
#[inline]
fn simm12(w: u32) -> u32 {
    (((w & 0xFFF) << 20) as i32 >> 20) as u32
}
#[inline]
fn uimm(w: u32) -> u32 {
    w & 0xFFFF
}

#[inline]
fn r(ctx: &Ctx, i: usize) -> u32 {
    ctx.regs.gpr[i].get()
}
#[inline]
fn set_r(ctx: &Ctx, i: usize, v: u32) {
    ctx.regs.gpr[i].set(v)
}
/// rA, or 0 when the field is 0 (D-form and indexed addressing).
#[inline]
fn ra0(ctx: &Ctx, w: u32) -> u32 {
    if fa(w) == 0 { 0 } else { r(ctx, fa(w)) }
}
#[inline]
fn ps(ctx: &Ctx, i: usize) -> Ps {
    ctx.regs.fpr[i].get()
}
#[inline]
fn set_ps(ctx: &Ctx, i: usize, v: Ps) {
    ctx.regs.fpr[i].set(v)
}
#[inline]
fn f0(ctx: &Ctx, i: usize) -> f64 {
    ctx.regs.fpr[i].get().ps0
}
#[inline]
fn set_f0(ctx: &Ctx, i: usize, v: f64) {
    ctx.regs.set_f(i, v)
}
#[cold]
fn illegal(ctx: &Ctx, pc: u32, w: u32) -> ! {
    panic!(
        "unimplemented instruction {w:08X} at {pc:#010X} ({})",
        ctx.name_of(pc)
    )
}

/// Before a call or jump through a register: one to the return sentinel's addresses, where no
/// code is, faults on fetching its first instruction, as a port's call through a pointer there
/// does. Only returns go there.
fn fetch_call(ctx: &Ctx, target: u32) {
    if target >= RETURN_SENTINEL {
        ctx.read_u32(target);
    }
}

/// Branches to `target`. Calls into native implementations go through dispatch.
fn branch(ctx: &Ctx, pc: u32, target: u32, link: bool) -> u32 {
    if link {
        ctx.regs.lr.set(pc.wrapping_add(4));
    }
    let native = ctx.flags_at(target) & FLAG_NATIVE != 0
        && ctx.entry(target).is_some_and(|e| e.mode != Mode::Original);
    if link && !native {
        ctx.convention_call(target, pc.wrapping_add(4));
    }
    if native {
        let ret = ctx.regs.lr.get();
        ctx.invoke(target);
        if let Some(to) = ctx.take_resume_at() {
            return to;
        }
        // A tail call returns to our caller.
        return if link { pc.wrapping_add(4) } else { ret };
    }
    target
}

/// Executes one instruction and returns the next PC.
pub fn step(ctx: &Ctx, pc: u32, w: u32) -> u32 {
    let next = pc.wrapping_add(4);
    match w >> 26 {
        3 => {
            if trap(fd(w) as u32, r(ctx, fa(w)), simm(w)) {
                panic!("twi trap at {pc:#010X} ({})", ctx.name_of(pc));
            }
        }
        4 => paired(ctx, pc, w),
        7 => set_r(
            ctx,
            fd(w),
            (r(ctx, fa(w)) as i32).wrapping_mul(simm(w) as i32) as u32,
        ),
        8 => {
            let (d, c, _) = add3(!r(ctx, fa(w)), simm(w), 1);
            set_r(ctx, fd(w), d);
            set_ca(ctx, c);
        }
        10 => {
            let (a, b) = (r(ctx, fa(w)), uimm(w));
            compare(ctx, (w >> 23) & 7, a < b, a > b);
        }
        11 => {
            let (a, b) = (r(ctx, fa(w)) as i32, simm(w) as i32);
            compare(ctx, (w >> 23) & 7, a < b, a > b);
        }
        12 | 13 => {
            let (d, c, _) = add3(r(ctx, fa(w)), simm(w), 0);
            set_r(ctx, fd(w), d);
            set_ca(ctx, c);
            if w >> 26 == 13 {
                update_cr0(ctx, d);
            }
        }
        14 => set_r(ctx, fd(w), ra0(ctx, w).wrapping_add(simm(w))),
        15 => set_r(ctx, fd(w), ra0(ctx, w).wrapping_add(w << 16)),
        16 => {
            let (bo, bi) = (fd(w) as u32, fa(w) as u32);
            let ctr = ctr_ok(ctx, bo);
            if ctr && cond_ok(ctx, bo, bi) {
                let disp = (w & 0xFFFC) as u16 as i16 as i32 as u32;
                let target = if w & 2 != 0 {
                    disp
                } else {
                    pc.wrapping_add(disp)
                };
                return branch(ctx, pc, target, w & 1 != 0);
            }
            if w & 1 != 0 {
                ctx.regs.lr.set(next);
            }
        }
        17 => {} // sc: the SDK layer covers what the handler would do.
        18 => {
            let disp = ((w & 0x03FF_FFFC) << 6) as i32 >> 6;
            let target = if w & 2 != 0 {
                disp as u32
            } else {
                pc.wrapping_add(disp as u32)
            };
            return branch(ctx, pc, target, w & 1 != 0);
        }
        19 => return op19(ctx, pc, w),
        20 => {
            let (sh, mb, me) = (fb(w) as u32, fc(w) as u32, (w >> 1) & 31);
            let m = rot_mask(mb, me);
            let v = (r(ctx, fd(w)).rotate_left(sh) & m) | (r(ctx, fa(w)) & !m);
            set_r(ctx, fa(w), v);
            if w & 1 != 0 {
                update_cr0(ctx, v);
            }
        }
        21 | 23 => {
            let sh = if w >> 26 == 21 {
                fb(w) as u32
            } else {
                r(ctx, fb(w)) & 31
            };
            let v = r(ctx, fd(w)).rotate_left(sh) & rot_mask(fc(w) as u32, (w >> 1) & 31);
            set_r(ctx, fa(w), v);
            if w & 1 != 0 {
                update_cr0(ctx, v);
            }
        }
        24 => set_r(ctx, fa(w), r(ctx, fd(w)) | uimm(w)),
        25 => set_r(ctx, fa(w), r(ctx, fd(w)) | (uimm(w) << 16)),
        26 => set_r(ctx, fa(w), r(ctx, fd(w)) ^ uimm(w)),
        27 => set_r(ctx, fa(w), r(ctx, fd(w)) ^ (uimm(w) << 16)),
        28 | 29 => {
            let imm = if w >> 26 == 28 {
                uimm(w)
            } else {
                uimm(w) << 16
            };
            let v = r(ctx, fd(w)) & imm;
            set_r(ctx, fa(w), v);
            update_cr0(ctx, v);
        }
        31 => op31(ctx, pc, w),
        32..=55 => load_store(ctx, w),
        56 | 57 => {
            let ea = if w >> 26 == 57 {
                r(ctx, fa(w))
            } else {
                ra0(ctx, w)
            }
            .wrapping_add(simm12(w));
            psq_load(ctx, ea, fd(w), (w >> 15) & 1 != 0, ((w >> 12) & 7) as usize);
            if w >> 26 == 57 {
                set_r(ctx, fa(w), ea);
            }
        }
        60 | 61 => {
            let ea = if w >> 26 == 61 {
                r(ctx, fa(w))
            } else {
                ra0(ctx, w)
            }
            .wrapping_add(simm12(w));
            psq_store(ctx, ea, fd(w), (w >> 15) & 1 != 0, ((w >> 12) & 7) as usize);
            if w >> 26 == 61 {
                set_r(ctx, fa(w), ea);
            }
        }
        59 => fp_single(ctx, pc, w),
        63 => fp_double(ctx, pc, w),
        _ => illegal(ctx, pc, w),
    }
    next
}

fn op19(ctx: &Ctx, pc: u32, w: u32) -> u32 {
    let next = pc.wrapping_add(4);
    let (d, a, b) = (fd(w) as u32, fa(w) as u32, fb(w) as u32);
    match (w >> 1) & 0x3FF {
        0 => set_cr_field(ctx, (w >> 23) & 7, cr_field(ctx, (w >> 18) & 7)),
        16 => {
            let target = ctx.regs.lr.get() & !3;
            let ctr = ctr_ok(ctx, d);
            if ctr && cond_ok(ctx, d, a) {
                if w & 1 == 0 {
                    ctx.convention_return(target);
                } else {
                    fetch_call(ctx, target);
                }
                return branch(ctx, pc, target, w & 1 != 0);
            }
            if w & 1 != 0 {
                ctx.regs.lr.set(next);
            }
        }
        528 => {
            let target = ctx.regs.ctr.get() & !3;
            if cond_ok(ctx, d, a) {
                fetch_call(ctx, target);
                return branch(ctx, pc, target, w & 1 != 0);
            }
            if w & 1 != 0 {
                ctx.regs.lr.set(next);
            }
        }
        33 => set_cr_bit(ctx, d, !(cr_bit(ctx, a) | cr_bit(ctx, b))),
        129 => set_cr_bit(ctx, d, cr_bit(ctx, a) & !cr_bit(ctx, b)),
        193 => set_cr_bit(ctx, d, cr_bit(ctx, a) ^ cr_bit(ctx, b)),
        225 => set_cr_bit(ctx, d, !(cr_bit(ctx, a) & cr_bit(ctx, b))),
        257 => set_cr_bit(ctx, d, cr_bit(ctx, a) & cr_bit(ctx, b)),
        289 => set_cr_bit(ctx, d, cr_bit(ctx, a) == cr_bit(ctx, b)),
        417 => set_cr_bit(ctx, d, cr_bit(ctx, a) | !cr_bit(ctx, b)),
        449 => set_cr_bit(ctx, d, cr_bit(ctx, a) | cr_bit(ctx, b)),
        150 => {} // isync
        50 => panic!("rfi at {pc:#010X}: exceptions are not emulated"),
        _ => illegal(ctx, pc, w),
    }
    next
}

fn op31(ctx: &Ctx, pc: u32, w: u32) {
    let (d, a, b) = (fd(w), fa(w), fb(w));
    let rc = w & 1 != 0;
    let xo = (w >> 1) & 0x3FF;
    let ea_x = || ra0(ctx, w).wrapping_add(r(ctx, b));
    let ea_ux = || r(ctx, a).wrapping_add(r(ctx, b));
    let logical = |v: u32| {
        set_r(ctx, a, v);
        if rc {
            update_cr0(ctx, v);
        }
    };
    match xo {
        0 | 32 => {
            let (x, y) = (r(ctx, a), r(ctx, b));
            if xo == 0 {
                compare(
                    ctx,
                    (w >> 23) & 7,
                    (x as i32) < (y as i32),
                    (x as i32) > (y as i32),
                );
            } else {
                compare(ctx, (w >> 23) & 7, x < y, x > y);
            }
        }
        4 => {
            if trap(d as u32, r(ctx, a), r(ctx, b)) {
                panic!("tw trap at {pc:#010X} ({})", ctx.name_of(pc));
            }
        }
        19 => set_r(ctx, d, ctx.regs.cr.get()),
        20 => set_r(ctx, d, ctx.read_u32(ea_x())),
        23 => set_r(ctx, d, ctx.read_u32(ea_x())),
        55 => {
            let ea = ea_ux();
            set_r(ctx, d, ctx.read_u32(ea));
            set_r(ctx, a, ea);
        }
        87 => set_r(ctx, d, u32::from(ctx.read_u8(ea_x()))),
        119 => {
            let ea = ea_ux();
            set_r(ctx, d, u32::from(ctx.read_u8(ea)));
            set_r(ctx, a, ea);
        }
        279 => set_r(ctx, d, u32::from(ctx.read_u16(ea_x()))),
        311 => {
            let ea = ea_ux();
            set_r(ctx, d, u32::from(ctx.read_u16(ea)));
            set_r(ctx, a, ea);
        }
        343 => set_r(ctx, d, ctx.read_u16(ea_x()) as i16 as u32),
        375 => {
            let ea = ea_ux();
            set_r(ctx, d, ctx.read_u16(ea) as i16 as u32);
            set_r(ctx, a, ea);
        }
        534 => set_r(ctx, d, ctx.read_u32(ea_x()).swap_bytes()),
        790 => set_r(ctx, d, u32::from(ctx.read_u16(ea_x()).swap_bytes())),
        150 => {
            ctx.write_u32(ea_x(), r(ctx, d));
            set_cr_field(ctx, 0, 2 | so(ctx));
        }
        151 => ctx.write_u32(ea_x(), r(ctx, d)),
        183 => {
            let ea = ea_ux();
            if a == 1 && d == 1 {
                ctx.stack_allocated(ea, r(ctx, 1));
            }
            ctx.write_u32(ea, r(ctx, d));
            set_r(ctx, a, ea);
        }
        215 => ctx.write_u8(ea_x(), r(ctx, d) as u8),
        247 => {
            let ea = ea_ux();
            ctx.write_u8(ea, r(ctx, d) as u8);
            set_r(ctx, a, ea);
        }
        407 => ctx.write_u16(ea_x(), r(ctx, d) as u16),
        439 => {
            let ea = ea_ux();
            ctx.write_u16(ea, r(ctx, d) as u16);
            set_r(ctx, a, ea);
        }
        662 => ctx.write_u32(ea_x(), r(ctx, d).swap_bytes()),
        918 => ctx.write_u16(ea_x(), (r(ctx, d) as u16).swap_bytes()),
        535 => fill(ctx, d, fp::lfs(ctx.read_u32(ea_x()))),
        567 => {
            let ea = ea_ux();
            fill(ctx, d, fp::lfs(ctx.read_u32(ea)));
            set_r(ctx, a, ea);
        }
        599 => set_f0(ctx, d, f64::from_bits(ctx.read_u64(ea_x()))),
        631 => {
            let ea = ea_ux();
            set_f0(ctx, d, f64::from_bits(ctx.read_u64(ea)));
            set_r(ctx, a, ea);
        }
        663 => ctx.write_u32(ea_x(), fp::stfs(f0(ctx, d))),
        695 => {
            let ea = ea_ux();
            ctx.write_u32(ea, fp::stfs(f0(ctx, d)));
            set_r(ctx, a, ea);
        }
        727 => ctx.write_u64(ea_x(), f0(ctx, d).to_bits()),
        759 => {
            let ea = ea_ux();
            ctx.write_u64(ea, f0(ctx, d).to_bits());
            set_r(ctx, a, ea);
        }
        983 => ctx.write_u32(ea_x(), f0(ctx, d).to_bits() as u32),
        24 => {
            let n = r(ctx, b) & 0x3F;
            logical(if n >= 32 { 0 } else { r(ctx, d) << n });
        }
        536 => {
            let n = r(ctx, b) & 0x3F;
            logical(if n >= 32 { 0 } else { r(ctx, d) >> n });
        }
        792 => {
            let n = r(ctx, b) & 0x3F;
            let s = r(ctx, d) as i32;
            if n >= 32 {
                set_ca(ctx, s < 0);
                logical(if s < 0 { u32::MAX } else { 0 });
            } else {
                set_ca(ctx, s < 0 && (s as u32) & ((1u32 << n) - 1) != 0);
                logical((s >> n) as u32);
            }
        }
        824 => {
            let n = b as u32;
            let s = r(ctx, d) as i32;
            set_ca(ctx, s < 0 && n > 0 && (s as u32) & ((1u32 << n) - 1) != 0);
            logical((s >> n) as u32);
        }
        26 => logical(r(ctx, d).leading_zeros()),
        28 => logical(r(ctx, d) & r(ctx, b)),
        60 => logical(r(ctx, d) & !r(ctx, b)),
        124 => logical(!(r(ctx, d) | r(ctx, b))),
        284 => logical(!(r(ctx, d) ^ r(ctx, b))),
        316 => logical(r(ctx, d) ^ r(ctx, b)),
        412 => logical(r(ctx, d) | !r(ctx, b)),
        444 => logical(r(ctx, d) | r(ctx, b)),
        476 => logical(!(r(ctx, d) & r(ctx, b))),
        922 => logical(r(ctx, d) as i16 as u32),
        954 => logical(r(ctx, d) as i8 as u32),
        83 => set_r(ctx, d, ctx.regs.msr.get()),
        146 => ctx.set_msr(r(ctx, d)),
        144 => {
            let crm = (w >> 12) & 0xFF;
            let v = r(ctx, d);
            for f in 0..8 {
                if crm & (0x80 >> f) != 0 {
                    set_cr_field(ctx, f, v >> (28 - 4 * f));
                }
            }
        }
        512 => {
            let x = ctx.regs.xer.get();
            set_cr_field(ctx, (w >> 23) & 7, x >> 28);
            ctx.regs.xer.set(x & 0x0FFF_FFFF);
        }
        339 | 371 => {
            let n = ((w >> 16) & 31) | (((w >> 11) & 31) << 5);
            let v = if n == spr::TBL_R { ctx.read_tbl() } else { ctx.regs.get_spr(n) };
            set_r(ctx, d, v);
        }
        467 => {
            let n = ((w >> 16) & 31) | (((w >> 11) & 31) << 5);
            ctx.regs.set_spr(n, r(ctx, d));
        }
        595 => set_r(ctx, d, ctx.regs.get_spr(0x1_0000 + ((w >> 16) & 15))),
        210 => ctx.regs.set_spr(0x1_0000 + ((w >> 16) & 15), r(ctx, d)),
        659 => set_r(ctx, d, ctx.regs.get_spr(0x1_0000 + (r(ctx, b) >> 28))),
        242 => ctx.regs.set_spr(0x1_0000 + (r(ctx, b) >> 28), r(ctx, d)),
        1014 => {
            let ea = ea_x() & !31;
            ctx.fill(ea, 0, 32);
        }
        54 | 86 | 246 | 278 | 470 | 982 | 598 | 854 | 306 | 566 => {} // cache and sync
        _ => arith(ctx, pc, w),
    }
}

fn arith(ctx: &Ctx, pc: u32, w: u32) {
    let (d, a, b) = (fd(w), fa(w), fb(w));
    let oe = (w >> 10) & 1 != 0;
    let (x, y) = (r(ctx, a), r(ctx, b));
    let (v, carry, overflow): (u32, Option<bool>, bool) = match (w >> 1) & 0x1FF {
        266 => {
            let (v, _, o) = add3(x, y, 0);
            (v, None, o)
        }
        10 => {
            let (v, c, o) = add3(x, y, 0);
            (v, Some(c), o)
        }
        138 => {
            let (v, c, o) = add3(x, y, ca(ctx));
            (v, Some(c), o)
        }
        234 => {
            let (v, c, o) = add3(x, u32::MAX, ca(ctx));
            (v, Some(c), o)
        }
        202 => {
            let (v, c, o) = add3(x, 0, ca(ctx));
            (v, Some(c), o)
        }
        40 => {
            let (v, _, o) = add3(!x, y, 1);
            (v, None, o)
        }
        8 => {
            let (v, c, o) = add3(!x, y, 1);
            (v, Some(c), o)
        }
        136 => {
            let (v, c, o) = add3(!x, y, ca(ctx));
            (v, Some(c), o)
        }
        232 => {
            let (v, c, o) = add3(!x, u32::MAX, ca(ctx));
            (v, Some(c), o)
        }
        200 => {
            let (v, c, o) = add3(!x, 0, ca(ctx));
            (v, Some(c), o)
        }
        104 => {
            let (v, _, o) = add3(!x, 0, 1);
            (v, None, o)
        }
        235 => {
            let p = i64::from(x as i32) * i64::from(y as i32);
            (p as u32, None, p != i64::from(p as i32))
        }
        75 => (
            ((i64::from(x as i32) * i64::from(y as i32)) >> 32) as u32,
            None,
            false,
        ),
        11 => (((u64::from(x) * u64::from(y)) >> 32) as u32, None, false),
        491 => {
            let (sx, sy) = (x as i32, y as i32);
            let o = sy == 0 || (x == 0x8000_0000 && sy == -1);
            let v = if o {
                if sx < 0 { u32::MAX } else { 0 }
            } else {
                (sx / sy) as u32
            };
            (v, None, o)
        }
        459 => {
            let o = y == 0;
            (if o { 0 } else { x / y }, None, o)
        }
        _ => illegal(ctx, pc, w),
    };
    set_r(ctx, d, v);
    if let Some(c) = carry {
        set_ca(ctx, c);
    }
    if oe {
        set_ov(ctx, overflow);
    }
    if w & 1 != 0 {
        update_cr0(ctx, v);
    }
}

fn load_store(ctx: &Ctx, w: u32) {
    let op = w >> 26;
    let (d, a) = (fd(w), fa(w));
    let update = matches!(op, 33 | 35 | 37 | 39 | 41 | 43 | 45 | 49 | 51 | 53 | 55);
    let ea = if update { r(ctx, a) } else { ra0(ctx, w) }.wrapping_add(simm(w));
    if op == 37 && a == 1 && d == 1 {
        // `stwu r1, -n(r1)` allocates a frame.
        ctx.stack_allocated(ea, r(ctx, 1));
    }
    // Saves and restores of the registers a call keeps, and of the link register, through the
    // stack pointer, for lockstep to tell from other uses of those words.
    let saves = ctx.tracks_saves();
    let restore = saves
        && match op {
            46 => true,
            32 | 33 => a == 1 && (d >= 14 || d == 0),
            50 | 51 => a == 1 && d >= 14,
            _ => false,
        };
    if restore {
        ctx.set_restoring(true);
    }
    match op {
        32 | 33 => set_r(ctx, d, ctx.read_u32(ea)),
        34 | 35 => set_r(ctx, d, u32::from(ctx.read_u8(ea))),
        40 | 41 => set_r(ctx, d, u32::from(ctx.read_u16(ea))),
        42 | 43 => set_r(ctx, d, ctx.read_u16(ea) as i16 as u32),
        36 | 37 => ctx.write_u32(ea, r(ctx, d)),
        38 | 39 => ctx.write_u8(ea, r(ctx, d) as u8),
        44 | 45 => ctx.write_u16(ea, r(ctx, d) as u16),
        46 => {
            for (i, reg) in (d..32).enumerate() {
                set_r(ctx, reg, ctx.read_u32(ea.wrapping_add(4 * i as u32)));
            }
        }
        47 => {
            for (i, reg) in (d..32).enumerate() {
                ctx.write_u32(ea.wrapping_add(4 * i as u32), r(ctx, reg));
            }
        }
        48 | 49 => fill(ctx, d, fp::lfs(ctx.read_u32(ea))),
        50 | 51 => set_f0(ctx, d, f64::from_bits(ctx.read_u64(ea))),
        52 | 53 => ctx.write_u32(ea, fp::stfs(f0(ctx, d))),
        54 | 55 => ctx.write_u64(ea, f0(ctx, d).to_bits()),
        _ => unreachable!(),
    }
    if restore {
        ctx.set_restoring(false);
    }
    if saves && a == 1 {
        match op {
            47 => ctx.note_gpr_store(ea, d as u32, 4 * (32 - d as u32)),
            36 | 37 if d >= 14 => ctx.note_gpr_store(ea, d as u32, 4),
            36 | 37 if d == 0 && r(ctx, 0) == ctx.regs.lr.get() => ctx.note_save(ea, 4),
            54 | 55 if d >= 14 => ctx.note_fpr_store(ea, d as u32),
            _ => {}
        }
    }
    if update {
        set_r(ctx, a, ea);
    }
}

fn fp_single(ctx: &Ctx, pc: u32, w: u32) {
    let (d, a, b, c) = (fd(w), fa(w), fb(w), fc(w));
    let (fa_, fb_, fc_) = (f0(ctx, a), f0(ctx, b), f0(ctx, c));
    let v = match (w >> 1) & 0x1F {
        18 => fp::fdivs(fa_, fb_),
        20 => fp::fsubs(fa_, fb_),
        21 => fp::fadds(fa_, fb_),
        24 => fp::fres(fb_),
        25 => fp::fmuls(fa_, fc_),
        28 => fp::fmsubs(fa_, fc_, fb_),
        29 => fp::fmadds(fa_, fc_, fb_),
        30 => fp::fnmsubs(fa_, fc_, fb_),
        31 => fp::fnmadds(fa_, fc_, fb_),
        _ => illegal(ctx, pc, w),
    };
    fill(ctx, d, v);
    if w & 1 != 0 {
        update_cr1(ctx);
    }
}

fn fp_double(ctx: &Ctx, pc: u32, w: u32) {
    let (d, a, b, c) = (fd(w), fa(w), fb(w), fc(w));
    let (fa_, fb_, fc_) = (f0(ctx, a), f0(ctx, b), f0(ctx, c));
    let a_form = match (w >> 1) & 0x1F {
        18 => Some(fp::fdiv(fa_, fb_)),
        20 => Some(fp::fsub(fa_, fb_)),
        21 => Some(fp::fadd(fa_, fb_)),
        23 => Some(fp::fsel(fa_, fc_, fb_)),
        25 => Some(fp::fmul(fa_, fc_)),
        26 => Some(fp::frsqrte(fb_)),
        28 => Some(fp::fmsub(fa_, fc_, fb_)),
        29 => Some(fp::fmadd(fa_, fc_, fb_)),
        30 => Some(fp::fnmsub(fa_, fc_, fb_)),
        31 => Some(fp::fnmadd(fa_, fc_, fb_)),
        _ => None,
    };
    if let Some(v) = a_form {
        set_f0(ctx, d, v);
    } else {
        match (w >> 1) & 0x3FF {
            0 | 32 => fp_compare(ctx, (w >> 23) & 7, fa_, fb_),
            12 => fill(ctx, d, fp::frsp(fb_)),
            14 => set_f0(ctx, d, f64::from_bits(fp::fcti_bits(fb_, fp::fctiw(fb_)))),
            15 => set_f0(ctx, d, f64::from_bits(fp::fcti_bits(fb_, fp::fctiwz(fb_)))),
            40 => set_f0(ctx, d, fp::fneg(fb_)),
            72 => set_f0(ctx, d, fb_),
            136 => set_f0(ctx, d, fp::fnabs(fb_)),
            264 => set_f0(ctx, d, fp::fabs(fb_)),
            583 => set_f0(
                ctx,
                d,
                f64::from_bits(0xFFF8_0000_0000_0000 | u64::from(ctx.regs.fpscr.get())),
            ),
            711 => {
                let fm = (w >> 17) & 0xFF;
                let v = fb_.to_bits() as u32;
                let mut fpscr = ctx.regs.fpscr.get();
                for f in 0..8 {
                    if fm & (0x80 >> f) != 0 {
                        let m = 0xF000_0000 >> (4 * f);
                        fpscr = (fpscr & !m) | (v & m);
                    }
                }
                ctx.regs.fpscr.set(fpscr);
            }
            38 | 70 => {
                let m = 1 << (31 - (w >> 21 & 31));
                let fpscr = ctx.regs.fpscr.get();
                ctx.regs.fpscr.set(if (w >> 1) & 0x3FF == 38 {
                    fpscr | m
                } else {
                    fpscr & !m
                });
            }
            134 => {
                let sh = 28 - 4 * ((w >> 23) & 7);
                let fpscr = ctx.regs.fpscr.get();
                ctx.regs
                    .fpscr
                    .set((fpscr & !(0xF << sh)) | (((w >> 12) & 0xF) << sh));
            }
            64 => {
                let src = (w >> 18) & 7;
                set_cr_field(ctx, (w >> 23) & 7, ctx.regs.fpscr.get() >> (28 - 4 * src));
            }
            _ => illegal(ctx, pc, w),
        }
    }
    if w & 1 != 0 {
        update_cr1(ctx);
    }
}

fn paired(ctx: &Ctx, pc: u32, w: u32) {
    let (d, a, b, c) = (fd(w), fa(w), fb(w), fc(w));
    match (w >> 1) & 0x3F {
        6 | 7 | 38 | 39 => {
            let update = (w >> 1) & 0x3F >= 38;
            let ea = if update { r(ctx, a) } else { ra0(ctx, w) }.wrapping_add(r(ctx, b));
            let (wbit, i) = ((w >> 10) & 1 != 0, ((w >> 7) & 7) as usize);
            if (w >> 1) & 1 == 0 {
                psq_load(ctx, ea, d, wbit, i);
            } else {
                psq_store(ctx, ea, d, wbit, i);
            }
            if update {
                set_r(ctx, a, ea);
            }
            return;
        }
        _ => {}
    }
    let (pa, pb, pc_) = (ps(ctx, a), ps(ctx, b), ps(ctx, c));
    let v = match (w >> 1) & 0x1F {
        10 => Some(fp::ps_sum0(pa, pc_, pb)),
        11 => Some(fp::ps_sum1(pa, pc_, pb)),
        12 => Some(fp::ps_muls0(pa, pc_)),
        13 => Some(fp::ps_muls1(pa, pc_)),
        14 => Some(fp::ps_madds0(pa, pc_, pb)),
        15 => Some(fp::ps_madds1(pa, pc_, pb)),
        18 => Some(fp::ps_div(pa, pb)),
        20 => Some(fp::ps_sub(pa, pb)),
        21 => Some(fp::ps_add(pa, pb)),
        23 => Some(fp::ps_sel(pa, pc_, pb)),
        24 => Some(fp::ps_res(pb)),
        25 => Some(fp::ps_mul(pa, pc_)),
        26 => Some(fp::ps_rsqrte(pb)),
        28 => Some(fp::ps_msub(pa, pc_, pb)),
        29 => Some(fp::ps_madd(pa, pc_, pb)),
        30 => Some(fp::ps_nmsub(pa, pc_, pb)),
        31 => Some(fp::ps_nmadd(pa, pc_, pb)),
        _ => None,
    };
    let v = match v {
        Some(v) => v,
        None => match (w >> 1) & 0x3FF {
            0 | 32 => return fp_compare(ctx, (w >> 23) & 7, pa.ps0, pb.ps0),
            64 | 96 => return fp_compare(ctx, (w >> 23) & 7, pa.ps1, pb.ps1),
            40 => fp::ps_neg(pb),
            72 => pb,
            136 => fp::ps_nabs(pb),
            264 => fp::ps_abs(pb),
            528 => fp::ps_merge00(pa, pb),
            560 => fp::ps_merge01(pa, pb),
            592 => fp::ps_merge10(pa, pb),
            624 => fp::ps_merge11(pa, pb),
            1014 => {
                let ea = ra0(ctx, w).wrapping_add(r(ctx, b)) & !31;
                ctx.fill(ea, 0, 32);
                return;
            }
            _ => illegal(ctx, pc, w),
        },
    };
    set_ps(ctx, d, v);
    if w & 1 != 0 {
        update_cr1(ctx);
    }
}

#[cfg(test)]
mod tests;
