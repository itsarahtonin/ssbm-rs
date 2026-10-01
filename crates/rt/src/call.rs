// SPDX-License-Identifier: GPL-3.0-or-later

//! C calling conventions for the GameCube's EABI: integers and pointers in r3..r10 (64-bit
//! values in aligned pairs), floats in f1..f8, results in r3 (r3:r4) or f1. Arguments that do
//! not fit go in 4-byte slots at 8(r1), with `float`s stored as singles.

use crate::{Ctx, Handle};

/// Where the next argument goes, and the stack words for those that overflow.
pub struct ArgRegs {
    gpr: usize,
    fpr: usize,
    stack: Vec<u32>,
    /// For reading arguments: the caller's r1.
    sp: u32,
}

impl ArgRegs {
    fn for_put() -> Self {
        Self {
            gpr: 3,
            fpr: 1,
            stack: Vec::new(),
            sp: 0,
        }
    }

    fn for_take(ctx: &Ctx) -> Self {
        Self {
            gpr: 3,
            fpr: 1,
            stack: Vec::new(),
            sp: ctx.regs.r(1),
        }
    }

    fn gpr(&mut self) -> Option<usize> {
        (self.gpr <= 10).then(|| {
            self.gpr += 1;
            self.gpr - 1
        })
    }

    fn gpr_pair(&mut self) -> Option<usize> {
        if self.gpr.is_multiple_of(2) {
            self.gpr += 1;
        }
        (self.gpr < 10).then(|| {
            self.gpr += 2;
            self.gpr - 2
        })
    }

    fn fpr(&mut self) -> Option<usize> {
        (self.fpr <= 8).then(|| {
            self.fpr += 1;
            self.fpr - 1
        })
    }

    /// Next stack slot index from 8(r1), after aligning to `align_words` words.
    fn slot(&mut self, words: usize, align_words: usize) -> usize {
        while !self.stack.len().is_multiple_of(align_words) {
            self.stack.push(0);
        }
        let at = self.stack.len();
        self.stack.resize(at + words, 0);
        at
    }

    fn read_slot(&mut self, ctx: &Ctx, words: usize, align_words: usize) -> u64 {
        let at = self.slot(words, align_words);
        let addr = self.sp + 8 + 4 * at as u32;
        if words == 2 {
            ctx.read_u64(addr)
        } else {
            u64::from(ctx.read_u32(addr))
        }
    }

    fn write_word(&mut self, v: u32) {
        let at = self.slot(1, 1);
        self.stack[at] = v;
    }

    fn write_dword(&mut self, v: u64) {
        let at = self.slot(2, 2);
        self.stack[at] = (v >> 32) as u32;
        self.stack[at + 1] = v as u32;
    }
}

/// One argument.
pub trait Arg<'a>: Sized {
    fn put(self, ctx: &'a Ctx, r: &mut ArgRegs);
    fn take(ctx: &'a Ctx, r: &mut ArgRegs) -> Self;
}

/// A function result.
pub trait Ret<'a>: Sized {
    fn get(ctx: &'a Ctx) -> Self;
    fn put(self, ctx: &'a Ctx);
}

macro_rules! int_arg {
    ($($t:ty),*) => {$(
        impl<'a> Arg<'a> for $t {
            #[inline]
            fn put(self, ctx: &'a Ctx, r: &mut ArgRegs) {
                match r.gpr() {
                    Some(i) => ctx.regs.set_r(i, self as u32),
                    None => r.write_word(self as u32),
                }
            }
            #[inline]
            fn take(ctx: &'a Ctx, r: &mut ArgRegs) -> Self {
                match r.gpr() {
                    Some(i) => ctx.regs.r(i) as $t,
                    None => r.read_slot(ctx, 1, 1) as $t,
                }
            }
        }
        impl<'a> Ret<'a> for $t {
            #[inline]
            fn get(ctx: &'a Ctx) -> Self {
                ctx.regs.r(3) as $t
            }
            #[inline]
            fn put(self, ctx: &'a Ctx) {
                ctx.regs.set_r(3, self as u32)
            }
        }
    )*};
}

int_arg!(u8, i8, u16, i16, u32, i32);

macro_rules! wide_arg {
    ($($t:ty),*) => {$(
        impl<'a> Arg<'a> for $t {
            fn put(self, ctx: &'a Ctx, r: &mut ArgRegs) {
                match r.gpr_pair() {
                    Some(i) => {
                        ctx.regs.set_r(i, (self as u64 >> 32) as u32);
                        ctx.regs.set_r(i + 1, self as u32);
                    }
                    None => r.write_dword(self as u64),
                }
            }
            fn take(ctx: &'a Ctx, r: &mut ArgRegs) -> Self {
                match r.gpr_pair() {
                    Some(i) => ((u64::from(ctx.regs.r(i)) << 32) | u64::from(ctx.regs.r(i + 1))) as $t,
                    None => r.read_slot(ctx, 2, 2) as $t,
                }
            }
        }
        impl<'a> Ret<'a> for $t {
            fn get(ctx: &'a Ctx) -> Self {
                ((u64::from(ctx.regs.r(3)) << 32) | u64::from(ctx.regs.r(4))) as $t
            }
            fn put(self, ctx: &'a Ctx) {
                ctx.regs.set_r(3, (self as u64 >> 32) as u32);
                ctx.regs.set_r(4, self as u32);
            }
        }
    )*};
}

wide_arg!(u64, i64);

/// A C `double` argument.
impl<'a> Arg<'a> for f64 {
    #[inline]
    fn put(self, ctx: &'a Ctx, r: &mut ArgRegs) {
        match r.fpr() {
            Some(i) => ctx.regs.set_f(i, self),
            None => r.write_dword(self.to_bits()),
        }
    }
    #[inline]
    fn take(ctx: &'a Ctx, r: &mut ArgRegs) -> Self {
        match r.fpr() {
            Some(i) => ctx.regs.f(i),
            None => f64::from_bits(r.read_slot(ctx, 2, 2)),
        }
    }
}

impl<'a> Ret<'a> for f64 {
    #[inline]
    fn get(ctx: &'a Ctx) -> Self {
        ctx.regs.f(1)
    }
    #[inline]
    fn put(self, ctx: &'a Ctx) {
        ctx.regs.set_f(1, self)
    }
}

/// A C `float` argument: a double in registers, a single on the stack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Single(pub f64);

impl<'a> Arg<'a> for Single {
    #[inline]
    fn put(self, ctx: &'a Ctx, r: &mut ArgRegs) {
        match r.fpr() {
            Some(i) => ctx.regs.set_f(i, self.0),
            None => r.write_word(gekko_fp::stfs(self.0)),
        }
    }
    #[inline]
    fn take(ctx: &'a Ctx, r: &mut ArgRegs) -> Self {
        match r.fpr() {
            Some(i) => Single(ctx.regs.f(i)),
            None => Single(gekko_fp::lfs(r.read_slot(ctx, 1, 1) as u32)),
        }
    }
}

impl<'a, H: Handle<'a>> Arg<'a> for H {
    #[inline]
    fn put(self, ctx: &'a Ctx, r: &mut ArgRegs) {
        Arg::put(self.addr(), ctx, r)
    }
    #[inline]
    fn take(ctx: &'a Ctx, r: &mut ArgRegs) -> Self {
        crate::At::new(ctx, u32::take(ctx, r)).field(0)
    }
}

impl<'a, H: Handle<'a>> Ret<'a> for H {
    #[inline]
    fn get(ctx: &'a Ctx) -> Self {
        crate::At::new(ctx, ctx.regs.r(3)).field(0)
    }
    #[inline]
    fn put(self, ctx: &'a Ctx) {
        ctx.regs.set_r(3, self.addr())
    }
}

impl<'a> Ret<'a> for () {
    #[inline]
    fn get(_: &'a Ctx) -> Self {}
    #[inline]
    fn put(self, _: &'a Ctx) {}
}

/// An argument list.
pub trait Args<'a>: Sized {
    fn put_into(self, ctx: &'a Ctx, r: &mut ArgRegs);
    fn take_from(ctx: &'a Ctx, r: &mut ArgRegs) -> Self;

    #[inline]
    fn take_all(ctx: &'a Ctx) -> Self {
        Self::take_from(ctx, &mut ArgRegs::for_take(ctx))
    }

    /// Puts the arguments that go in registers there, as a function being entered finds them.
    /// Arguments past the registers stay where the caller put them.
    #[inline]
    fn put_regs(self, ctx: &'a Ctx) {
        self.put_into(ctx, &mut ArgRegs::for_put());
    }
}

macro_rules! tuple_args {
    ($($a:ident),*) => {
        impl<'a, $($a: Arg<'a>),*> Args<'a> for ($($a,)*) {
            #[inline]
            #[allow(non_snake_case, unused_variables)]
            fn put_into(self, ctx: &'a Ctx, r: &mut ArgRegs) {
                let ($($a,)*) = self;
                $($a.put(ctx, r);)*
            }
            #[inline]
            #[allow(unused_variables, clippy::unused_unit)]
            fn take_from(ctx: &'a Ctx, r: &mut ArgRegs) -> Self {
                ($($a::take(ctx, r),)*)
            }
        }
    };
}

macro_rules! all_tuples {
    ($($a:ident),*) => {
        all_tuples!(@ [] $($a),*);
    };
    (@ [$($done:ident),*]) => {
        tuple_args!($($done),*);
    };
    (@ [$($done:ident),*] $next:ident $(, $rest:ident)*) => {
        tuple_args!($($done),*);
        all_tuples!(@ [$($done,)* $next] $($rest),*);
    };
}

all_tuples!(
    A0, A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11, A12, A13, A14, A15, A16, A17, A18, A19, A20,
    A21, A22, A23
);

impl Ctx {
    fn call_with(&self, addr: u32, r: ArgRegs) {
        if self.lockstep.trace_calls.get() {
            self.lockstep.note_arity(addr, r.gpr - 3);
        }
        // Arguments past the registers go at 8(r1), which the caller's frame holds for them as
        // the original's does, so the callee runs at the original's stack addresses.
        let sp = self.regs.r(1);
        for (i, w) in r.stack.iter().enumerate() {
            self.write_u32(sp + 8 + 4 * i as u32, *w);
        }
        self.invoke(addr);
        if let Some(to) = self.take_resume_at() {
            // A jump to no code faults where the original fetches its first instruction.
            if to.wrapping_sub(0x8000_0000) >= crate::MEM1_SIZE {
                self.fault(to, 4, false);
            }
            panic!(
                "{} jumped to {} past a port that called it",
                self.name_of(addr),
                self.name_of(to)
            );
        }
    }

    /// Calls the function at `addr` with C calling conventions.
    #[inline]
    pub fn call<'a, A: Args<'a>, R: Ret<'a>>(&'a self, addr: u32, args: A) -> R {
        let mut r = ArgRegs::for_put();
        args.put_into(self, &mut r);
        self.call_with(addr, r);
        R::get(self)
    }

    /// Calls a variadic function. CR bit 6 tells the callee whether floats are in registers.
    pub fn call_variadic<'a, A: Args<'a>, R: Ret<'a>>(
        &'a self,
        addr: u32,
        args: A,
        varargs: &[VarArg],
    ) -> R {
        let mut r = ArgRegs::for_put();
        args.put_into(self, &mut r);
        let mut floats = false;
        for v in varargs {
            match *v {
                VarArg::Int(x) => Arg::put(x, self, &mut r),
                VarArg::Wide(x) => Arg::put(x, self, &mut r),
                VarArg::Float(f) => {
                    floats = true;
                    Arg::put(f, self, &mut r)
                }
            }
        }
        let bit = 1 << (31 - 6);
        let cr = self.regs.cr.get();
        self.regs.cr.set(if floats { cr | bit } else { cr & !bit });
        self.call_with(addr, r);
        R::get(self)
    }
}

/// One argument after the `...` of a variadic call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VarArg {
    Int(u32),
    /// A `long long`, in an aligned register pair or stack slot.
    Wide(u64),
    Float(f64),
}

/// Reads a native variadic function's arguments after its fixed ones.
pub struct VarArgs<'a> {
    ctx: &'a Ctx,
    gpr: usize,
    fpr: usize,
    stack: u32,
}

impl<'a> VarArgs<'a> {
    /// `fixed_gprs` and `fixed_fprs` count the registers the fixed parameters used.
    pub fn new(ctx: &'a Ctx, fixed_gprs: usize, fixed_fprs: usize) -> Self {
        Self {
            ctx,
            gpr: 3 + fixed_gprs,
            fpr: 1 + fixed_fprs,
            stack: ctx.regs.r(1) + 8,
        }
    }

    pub fn int(&mut self) -> u32 {
        if self.gpr <= 10 {
            self.gpr += 1;
            self.ctx.regs.r(self.gpr - 1)
        } else {
            self.stack += 4;
            self.ctx.read_u32(self.stack - 4)
        }
    }

    pub fn float(&mut self) -> f64 {
        if self.fpr <= 8 {
            self.fpr += 1;
            self.ctx.regs.f(self.fpr - 1)
        } else {
            self.stack = (self.stack + 7) & !7;
            self.stack += 8;
            f64::from_bits(self.ctx.read_u64(self.stack - 8))
        }
    }
}
