// SPDX-License-Identifier: GPL-3.0-or-later

//! Storage-agnostic handles. Generated accessors are built from these, so game code never
//! sees raw offsets. Stage 1 backs every handle with a GameCube address.

use std::fmt;
use std::marker::PhantomData;

use crate::Ctx;

/// An address plus the machine it lives in: the Stage 1 backing of every handle.
#[derive(Clone, Copy)]
pub struct At<'a> {
    pub ctx: &'a Ctx,
    pub addr: u32,
}

impl PartialEq for At<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Eq for At<'_> {}

impl fmt::Debug for At<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#010X}", self.addr)
    }
}

impl<'a> At<'a> {
    #[inline]
    pub fn new(ctx: &'a Ctx, addr: u32) -> Self {
        Self { ctx, addr }
    }

    #[inline]
    pub fn offset(self, off: u32) -> Self {
        Self::new(self.ctx, self.addr.wrapping_add(off))
    }

    #[inline]
    pub fn get<T: Scalar>(self, off: u32) -> T::Value {
        T::read(self.offset(off))
    }

    #[inline]
    pub fn set<T: Scalar>(self, off: u32, v: T::Value) {
        T::write(self.offset(off), v)
    }

    /// The handle of type `H` at `off`.
    #[inline]
    pub fn field<H: Handle<'a>>(self, off: u32) -> H {
        H::from_at(self.offset(off))
    }

    /// Loads the pointer at `off` as a handle.
    #[inline]
    pub fn ptr<H: Handle<'a>>(self, off: u32) -> H {
        H::from_at(Self::new(
            self.ctx,
            self.ctx.read_u32(self.addr.wrapping_add(off)),
        ))
    }

    #[inline]
    pub fn set_ptr<H: Handle<'a>>(self, off: u32, v: H) {
        self.ctx.write_u32(self.addr.wrapping_add(off), v.at().addr)
    }

    /// Reads a bitfield whose first bit is `bit` bits from `off`, counting from the MSB.
    pub fn bits(self, off: u32, bit: u32, width: u32, signed: bool) -> i64 {
        let (addr, n, shift) = bit_window(self.addr.wrapping_add(off), bit, width);
        let v = (self.ctx.read_be(addr, n) >> shift) & mask(width);
        if signed && (v >> (width - 1)) & 1 != 0 {
            (v | !mask(width)) as i64
        } else {
            v as i64
        }
    }

    pub fn set_bits(self, off: u32, bit: u32, width: u32, v: i64) {
        let (addr, n, shift) = bit_window(self.addr.wrapping_add(off), bit, width);
        let raw = self.ctx.read_be(addr, n);
        let m = mask(width) << shift;
        self.ctx
            .write_be(addr, n, (raw & !m) | (((v as u64) << shift) & m));
    }
}

/// The bytes holding a bitfield: first address, byte count, and the right shift within them.
fn bit_window(base: u32, bit: u32, width: u32) -> (u32, u32, u32) {
    let first = bit % 8;
    let n = (first + width).div_ceil(8);
    (base.wrapping_add(bit / 8), n, n * 8 - first - width)
}

fn mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1 << width) - 1
    }
}

/// Anything that names storage: a struct, a scalar slot, a pointer slot, an array.
pub trait Handle<'a>: Copy {
    /// Size in bytes on the GameCube, which is also the stride for pointer arithmetic.
    const SIZE: u32;
    fn from_at(at: At<'a>) -> Self;
    fn at(self) -> At<'a>;

    #[inline]
    fn addr(self) -> u32 {
        self.at().addr
    }

    #[inline]
    fn ctx(self) -> &'a Ctx {
        self.at().ctx
    }

    #[inline]
    fn is_null(self) -> bool {
        self.addr() == 0
    }

    /// C pointer arithmetic: `p + n`.
    #[inline]
    fn add(self, n: i32) -> Self {
        Self::from_at(self.at().offset((n as u32).wrapping_mul(Self::SIZE)))
    }

    /// C pointer cast.
    #[inline]
    fn cast<U: Handle<'a>>(self) -> U {
        U::from_at(self.at())
    }

    /// C struct assignment: copies `SIZE` bytes from `src`.
    fn copy_from(self, src: Self) {
        self.ctx().copy(self.addr(), src.addr(), Self::SIZE);
    }
}

/// A null handle of any type.
pub fn null<'a, H: Handle<'a>>(ctx: &'a Ctx) -> H {
    H::from_at(At::new(ctx, 0))
}

/// A value stored in memory: an integer, or a float read as a register value.
pub trait Scalar: 'static {
    type Value: Copy + fmt::Debug;
    const SIZE: u32;
    fn read(at: At<'_>) -> Self::Value;
    fn write(at: At<'_>, v: Self::Value);
}

macro_rules! int_scalar {
    ($t:ty, $read:ident, $write:ident, $size:expr) => {
        impl Scalar for $t {
            type Value = $t;
            const SIZE: u32 = $size;
            #[inline]
            fn read(at: At<'_>) -> $t {
                at.ctx.$read(at.addr) as $t
            }
            #[inline]
            fn write(at: At<'_>, v: $t) {
                at.ctx.$write(at.addr, v as _)
            }
        }
    };
}

int_scalar!(u8, read_u8, write_u8, 1);
int_scalar!(i8, read_u8, write_u8, 1);
int_scalar!(u16, read_u16, write_u16, 2);
int_scalar!(i16, read_u16, write_u16, 2);
int_scalar!(u32, read_u32, write_u32, 4);
int_scalar!(i32, read_u32, write_u32, 4);
int_scalar!(u64, read_u64, write_u64, 8);
int_scalar!(i64, read_u64, write_u64, 8);

/// A C `float`. Reads widen like `lfs`; writes round like C assignment, then store like `stfs`.
pub struct F32;

impl Scalar for F32 {
    type Value = f64;
    const SIZE: u32 = 4;
    #[inline]
    fn read(at: At<'_>) -> f64 {
        gekko_fp::lfs(at.ctx.read_u32(at.addr))
    }
    #[inline]
    fn write(at: At<'_>, v: f64) {
        at.ctx.write_u32(at.addr, gekko_fp::stfs(gekko_fp::frsp(v)))
    }
}

/// A C `double`.
pub struct F64;

impl Scalar for F64 {
    type Value = f64;
    const SIZE: u32 = 8;
    #[inline]
    fn read(at: At<'_>) -> f64 {
        f64::from_bits(at.ctx.read_u64(at.addr))
    }
    #[inline]
    fn write(at: At<'_>, v: f64) {
        at.ctx.write_u64(at.addr, v.to_bits())
    }
}

/// A scalar slot, such as `&fp->percent` or a `u8*`.
pub struct Val<'a, T: Scalar> {
    at: At<'a>,
    _t: PhantomData<fn() -> T>,
}

impl<T: Scalar> Clone for Val<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Scalar> Copy for Val<'_, T> {}
impl<T: Scalar> PartialEq for Val<'_, T> {
    fn eq(&self, o: &Self) -> bool {
        self.at == o.at
    }
}
impl<T: Scalar> fmt::Debug for Val<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.at.fmt(f)
    }
}

impl<'a, T: Scalar> Handle<'a> for Val<'a, T> {
    const SIZE: u32 = T::SIZE;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self {
            at,
            _t: PhantomData,
        }
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.at
    }
}

impl<T: Scalar> Val<'_, T> {
    #[inline]
    pub fn get(self) -> T::Value {
        T::read(self.at)
    }
    #[inline]
    pub fn set(self, v: T::Value) {
        T::write(self.at, v)
    }
}

impl<'a> Val<'a, u8> {
    /// Reads a NUL-terminated string starting here.
    pub fn read_cstr(self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut p = self.at.addr;
        loop {
            let b = self.at.ctx.read_u8(p);
            if b == 0 {
                return out;
            }
            out.push(b);
            p = p.wrapping_add(1);
        }
    }
}

/// A pointer slot holding a pointer to `H`, such as a `HSD_GObj**`.
pub struct Ptr<'a, H> {
    at: At<'a>,
    _h: PhantomData<fn() -> H>,
}

impl<H> Clone for Ptr<'_, H> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<H> Copy for Ptr<'_, H> {}
impl<H> PartialEq for Ptr<'_, H> {
    fn eq(&self, o: &Self) -> bool {
        self.at == o.at
    }
}
impl<H> fmt::Debug for Ptr<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.at.fmt(f)
    }
}

impl<'a, H: Handle<'a>> Handle<'a> for Ptr<'a, H> {
    const SIZE: u32 = 4;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self {
            at,
            _h: PhantomData,
        }
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.at
    }
}

impl<'a, H: Handle<'a>> Ptr<'a, H> {
    #[inline]
    pub fn get(self) -> H {
        self.at.ptr(0)
    }
    #[inline]
    pub fn set(self, v: H) {
        self.at.set_ptr(0, v)
    }
}

/// An untyped pointer (`void*`). Its arithmetic steps one byte, like GNU C.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Addr<'a>(At<'a>);

impl<'a> Handle<'a> for Addr<'a> {
    const SIZE: u32 = 1;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self(at)
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.0
    }
}

/// A function pointer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FnPtr<'a>(At<'a>);

impl<'a> Handle<'a> for FnPtr<'a> {
    const SIZE: u32 = 4;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self(at)
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.0
    }
}

impl<'a> FnPtr<'a> {
    /// Calls through the pointer with C calling conventions.
    #[inline]
    pub fn call<A: crate::Args<'a>, R: crate::Ret<'a>>(self, args: A) -> R {
        self.0.ctx.call(self.0.addr, args)
    }
}

/// An array of handles (structs, unions, nested arrays).
pub struct Arr<'a, H, const N: u32> {
    at: At<'a>,
    _h: PhantomData<fn() -> H>,
}

/// An array of scalars.
pub struct ArrV<'a, T: Scalar, const N: u32> {
    at: At<'a>,
    _t: PhantomData<fn() -> T>,
}

/// An array of pointers.
pub struct ArrP<'a, H, const N: u32> {
    at: At<'a>,
    _h: PhantomData<fn() -> H>,
}

macro_rules! array_common {
    ($name:ident, $bound:path) => {
        impl<H: $bound, const N: u32> Clone for $name<'_, H, N> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<H: $bound, const N: u32> Copy for $name<'_, H, N> {}
        impl<H: $bound, const N: u32> PartialEq for $name<'_, H, N> {
            fn eq(&self, o: &Self) -> bool {
                self.at == o.at
            }
        }
        impl<H: $bound, const N: u32> fmt::Debug for $name<'_, H, N> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.at.fmt(f)
            }
        }
    };
}

pub trait AnyHandle {}
impl<T> AnyHandle for T {}

array_common!(Arr, AnyHandle);
array_common!(ArrP, AnyHandle);

impl<T: Scalar, const N: u32> Clone for ArrV<'_, T, N> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Scalar, const N: u32> Copy for ArrV<'_, T, N> {}
impl<T: Scalar, const N: u32> PartialEq for ArrV<'_, T, N> {
    fn eq(&self, o: &Self) -> bool {
        self.at == o.at
    }
}
impl<T: Scalar, const N: u32> fmt::Debug for ArrV<'_, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.at.fmt(f)
    }
}

impl<'a, H: Handle<'a>, const N: u32> Handle<'a> for Arr<'a, H, N> {
    const SIZE: u32 = H::SIZE * N;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self {
            at,
            _h: PhantomData,
        }
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.at
    }
}

impl<'a, T: Scalar, const N: u32> Handle<'a> for ArrV<'a, T, N> {
    const SIZE: u32 = T::SIZE * N;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self {
            at,
            _t: PhantomData,
        }
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.at
    }
}

impl<'a, H: Handle<'a>, const N: u32> Handle<'a> for ArrP<'a, H, N> {
    const SIZE: u32 = 4 * N;
    #[inline]
    fn from_at(at: At<'a>) -> Self {
        Self {
            at,
            _h: PhantomData,
        }
    }
    #[inline]
    fn at(self) -> At<'a> {
        self.at
    }
}

// Indexing is unchecked, as in C: out-of-range reads see whatever the original would.
impl<'a, H: Handle<'a>, const N: u32> Arr<'a, H, N> {
    pub const LEN: u32 = N;
    #[inline]
    pub fn get(self, i: i32) -> H {
        H::from_at(self.at).add(i)
    }
}

impl<'a, T: Scalar, const N: u32> ArrV<'a, T, N> {
    pub const LEN: u32 = N;
    #[inline]
    pub fn at(self, i: i32) -> Val<'a, T> {
        Val::from_at(self.at).add(i)
    }
    #[inline]
    pub fn get(self, i: i32) -> T::Value {
        self.at(i).get()
    }
    #[inline]
    pub fn set(self, i: i32, v: T::Value) {
        self.at(i).set(v)
    }
}

impl<'a, H: Handle<'a>, const N: u32> ArrP<'a, H, N> {
    pub const LEN: u32 = N;
    #[inline]
    pub fn at(self, i: i32) -> Ptr<'a, H> {
        Ptr::from_at(self.at).add(i)
    }
    #[inline]
    pub fn get(self, i: i32) -> H {
        self.at(i).get()
    }
    #[inline]
    pub fn set(self, i: i32, v: H) {
        self.at(i).set(v)
    }
}
