// SPDX-License-Identifier: GPL-3.0-or-later

//! Runtime core: the machine context every function runs against.
//!
//! Game code takes `&Ctx` and reaches memory only through handles. Every call goes through
//! dispatch by original address, so any function can be a Rust port, an SDK stand-in, or (in
//! dev builds) the original code run by the interpreter.

use std::any::{Any, TypeId};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

pub use ssbm_mem::{MEM1_SIZE, Mem, PAGE_SIZE, Pages};

mod call;
mod handle;
pub mod lockstep;
mod regs;

pub use call::{Arg, ArgRegs, Args, Ret, Single, VarArg, VarArgs};
pub use handle::{Addr, Arr, ArrP, ArrV, At, F32, F64, FnPtr, Handle, Ptr, Scalar, Val, null};
pub use lockstep::Returns;
pub use regs::{Regs, RegsSnapshot, spr};

/// Code run when execution reaches an address.
pub type Hook = Rc<dyn Fn(&Ctx)>;

/// Called with the PC every `HEARTBEAT` interpreted instructions.
pub type Heartbeat = Rc<dyn Fn(&Ctx, u32)>;

/// A function at the register level: arguments and results are in `ctx.regs`.
pub type Native = fn(&Ctx);

/// How a call to an address with a native implementation runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Run the native implementation.
    Native,
    /// Run the original and the native implementation from the same state and compare.
    Lockstep,
    /// Run the original even though a native implementation exists.
    Original,
}

#[derive(Clone, Copy)]
pub struct Entry {
    pub native: Native,
    pub mode: Mode,
    /// A stand-in for something outside game memory, such as an SDK device or service.
    /// Lockstep runs it for the original only and replays its effects for the port.
    pub external: bool,
    /// Where the result is, for lockstep to compare.
    pub returns: Returns,
}

/// Runs original PowerPC code. Only dev builds provide one.
pub trait Backend {
    /// Runs the function at `addr` until it returns.
    fn run(&self, ctx: &Ctx, addr: u32);
}

/// Hardware registers at `0xCC00_0000`, including the GX write-gather pipe.
pub trait Mmio {
    fn read(&self, ctx: &Ctx, addr: u32, size: u32) -> u32;
    fn write(&self, ctx: &Ctx, addr: u32, size: u32, value: u32);
}

/// A bad memory access, raised as a panic payload (the console would take a DSI exception).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub addr: u32,
    pub len: u32,
    pub write: bool,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.write { "write" } else { "read" };
        write!(
            f,
            "unmapped {kind} of {} bytes at {:#010X}",
            self.len, self.addr
        )
    }
}

const MMIO_BASE: u32 = 0xCC00_0000;
const MMIO_END: u32 = 0xCC01_0000;
const LOCKED_CACHE: u32 = 0xE000_0000;
const LOCKED_CACHE_SIZE: u32 = 0x4000;
/// Return address that ends a call into original code.
pub const RETURN_SENTINEL: u32 = 0xFFFF_FFF0;

/// The machine: memory, registers, and what implements each function.
pub struct Ctx {
    pub mem: Mem,
    pub regs: Regs,
    locked_cache: Box<[Cell<u8>]>,
    dispatch: RefCell<HashMap<u32, Entry>>,
    backend: OnceCell<Box<dyn Backend>>,
    mmio: OnceCell<Box<dyn Mmio>>,
    names: OnceCell<Box<dyn Fn(u32) -> Option<String>>>,
    hooks: RefCell<HashMap<u32, Hook>>,
    /// Per MEM1 word: `FLAG_NATIVE` and `FLAG_HOOK`, so the interpreter checks cheaply.
    flags: Box<[Cell<u8>]>,
    pub lockstep: lockstep::State,
    /// Per-machine state of layers above the runtime, such as the SDK layer, by type.
    ext: RefCell<HashMap<TypeId, Rc<dyn Any>>>,
    /// Called by the interpreter every `HEARTBEAT` instructions with the current PC.
    heartbeat: RefCell<Option<Heartbeat>>,
    /// Called when interrupts are enabled or time passes, so pending ones can be taken.
    interrupt_check: RefCell<Option<Hook>>,
    /// A debugging watchpoint: writes to `[start, start + len)` call `watch_hook`.
    watch: Cell<(u32, u32)>,
    watch_hook: RefCell<Option<WatchHook>>,
}

/// Called with the address and size of a write to the watched range.
pub type WatchHook = Rc<dyn Fn(&Ctx, u32, u32)>;

/// `MSR[EE]`: external interrupts enabled.
pub const MSR_EE: u32 = 1 << 15;

/// Instructions between heartbeats.
pub const HEARTBEAT: u64 = 1 << 24;

/// The word starts a function with a native implementation.
pub const FLAG_NATIVE: u8 = 1;
/// The word has a hook.
pub const FLAG_HOOK: u8 = 2;

impl Default for Ctx {
    fn default() -> Self {
        Self::new()
    }
}

impl Ctx {
    pub fn new() -> Self {
        Self {
            mem: Mem::new(),
            regs: Regs::default(),
            locked_cache: vec![Cell::new(0); LOCKED_CACHE_SIZE as usize].into_boxed_slice(),
            dispatch: RefCell::default(),
            backend: OnceCell::new(),
            mmio: OnceCell::new(),
            names: OnceCell::new(),
            hooks: RefCell::default(),
            flags: vec![Cell::new(0); (MEM1_SIZE / 4) as usize].into_boxed_slice(),
            lockstep: lockstep::State::default(),
            ext: RefCell::default(),
            heartbeat: RefCell::default(),
            interrupt_check: RefCell::default(),
            watch: Cell::new((0, 0)),
            watch_hook: RefCell::default(),
        }
    }

    /// Calls `hook` after every write that touches `[addr, addr + len)`. For debugging.
    pub fn set_watch(&self, addr: u32, len: u32, hook: WatchHook) {
        self.watch.set((addr, len));
        *self.watch_hook.borrow_mut() = Some(hook);
    }

    /// Moves the watched range, keeping the hook.
    pub fn move_watch(&self, addr: u32, len: u32) {
        self.watch.set((addr, len));
    }

    #[inline]
    fn watched(&self, addr: u32, len: u32) {
        let (start, n) = self.watch.get();
        let past = addr.wrapping_add(len).wrapping_sub(start);
        if n != 0 && past.wrapping_sub(1) < n + len - 1 {
            let hook = self.watch_hook.borrow().clone();
            if let Some(hook) = hook {
                hook(self, addr, len);
            }
        }
    }

    /// Sets what runs when interrupts become enabled or time passes: the device layer takes
    /// pending interrupts and due events there, as the CPU would take an exception.
    pub fn set_interrupt_check(&self, f: Hook) {
        *self.interrupt_check.borrow_mut() = Some(f);
    }

    fn check_interrupts(&self) {
        let f = self.interrupt_check.borrow().clone();
        if let Some(f) = f {
            if self.lockstep.is_active() {
                lockstep::interrupts(self, &f);
            } else {
                f(self);
            }
        }
    }

    /// Writes MSR, as `mtmsr` does: enabling interrupts lets pending ones in.
    pub fn set_msr(&self, v: u32) {
        let old = self.regs.msr.replace(v);
        if old & MSR_EE == 0 && v & MSR_EE != 0 {
            self.check_interrupts();
        }
    }

    /// Lets `ticks` of time base pass, as code that reads the clock or polls hardware does.
    pub fn tick(&self, ticks: u64) {
        self.regs.tb.set(self.regs.tb.get() + ticks);
        if self.regs.msr.get() & MSR_EE != 0 {
            self.check_interrupts();
        }
    }

    /// Sets the function the interpreter calls every `HEARTBEAT` instructions, for progress
    /// reports and stall detection.
    pub fn set_heartbeat(&self, f: impl Fn(&Ctx, u32) + 'static) {
        *self.heartbeat.borrow_mut() = Some(Rc::new(f));
    }

    pub fn beat(&self, pc: u32) {
        let f = self.heartbeat.borrow().clone();
        if let Some(f) = f {
            f(self, pc);
        }
    }

    /// Stores state for a layer above the runtime, replacing any of the same type.
    pub fn set_ext<T: Any>(&self, value: T) -> Rc<T> {
        let rc = Rc::new(value);
        self.ext
            .borrow_mut()
            .insert(TypeId::of::<T>(), rc.clone() as Rc<dyn Any>);
        rc
    }

    /// State stored with `set_ext`, if any.
    pub fn try_ext<T: Any>(&self) -> Option<Rc<T>> {
        let any = self.ext.borrow().get(&TypeId::of::<T>())?.clone();
        any.downcast().ok()
    }

    /// State stored with `set_ext`. Panics if there is none.
    pub fn ext<T: Any>(&self) -> Rc<T> {
        self.try_ext()
            .unwrap_or_else(|| panic!("no {} in this context", std::any::type_name::<T>()))
    }

    /// `FLAG_*` bits for the word at `addr`.
    #[inline]
    pub fn flags_at(&self, addr: u32) -> u8 {
        match ssbm_mem::phys(addr, 4) {
            Some(p) => self.flags[(p / 4) as usize].get(),
            None => 0,
        }
    }

    fn set_flag(&self, addr: u32, flag: u8, on: bool) {
        if let Some(p) = ssbm_mem::phys(addr, 4) {
            let cell = &self.flags[(p / 4) as usize];
            cell.set(if on {
                cell.get() | flag
            } else {
                cell.get() & !flag
            });
        }
    }

    pub fn set_backend(&self, backend: Box<dyn Backend>) {
        assert!(self.backend.set(backend).is_ok(), "backend already set");
    }

    pub fn has_backend(&self) -> bool {
        self.backend.get().is_some()
    }

    pub fn set_mmio(&self, mmio: Box<dyn Mmio>) {
        assert!(self.mmio.set(mmio).is_ok(), "MMIO already set");
    }

    /// Names addresses in diagnostics, usually from the decomp's symbols.
    pub fn set_names(&self, names: Box<dyn Fn(u32) -> Option<String>>) {
        let _ = self.names.set(names);
    }

    pub fn name_of(&self, addr: u32) -> String {
        self.names
            .get()
            .and_then(|f| f(addr))
            .unwrap_or_else(|| format!("{addr:#010X}"))
    }

    // Memory. Faults panic with a `Fault` payload.

    #[cold]
    fn fault(&self, addr: u32, len: u32, write: bool) -> ! {
        std::panic::panic_any(Fault { addr, len, write })
    }

    #[inline]
    fn locked(&self, addr: u32, len: u32) -> Option<&[Cell<u8>]> {
        let off = addr.wrapping_sub(LOCKED_CACHE);
        (off < LOCKED_CACHE_SIZE && off + len <= LOCKED_CACHE_SIZE)
            .then(|| &self.locked_cache[off as usize..(off + len) as usize])
    }

    fn read_slow(&self, addr: u32, len: u32) -> u64 {
        if (MMIO_BASE..MMIO_END).contains(&addr)
            && let Some(mmio) = self.mmio.get()
        {
            return u64::from(lockstep::mmio_read(self, mmio.as_ref(), addr, len));
        }
        match self.locked(addr, len) {
            Some(cells) => cells
                .iter()
                .fold(0, |acc, c| (acc << 8) | u64::from(c.get())),
            None => self.fault(addr, len, false),
        }
    }

    fn write_slow(&self, addr: u32, len: u32, value: u64) {
        if (MMIO_BASE..MMIO_END).contains(&addr)
            && let Some(mmio) = self.mmio.get()
        {
            return lockstep::mmio_write(self, mmio.as_ref(), addr, len, value as u32);
        }
        match self.locked(addr, len) {
            Some(cells) => {
                for (i, c) in cells.iter().enumerate() {
                    c.set((value >> (8 * (len as usize - 1 - i))) as u8);
                }
            }
            None => self.fault(addr, len, true),
        }
    }

    #[inline]
    pub fn read_u8(&self, addr: u32) -> u8 {
        match self.mem.read_u8(addr) {
            Ok(v) => v,
            Err(_) => self.read_slow(addr, 1) as u8,
        }
    }

    #[inline]
    pub fn read_u16(&self, addr: u32) -> u16 {
        match self.mem.read_u16(addr) {
            Ok(v) => v,
            Err(_) => self.read_slow(addr, 2) as u16,
        }
    }

    #[inline]
    pub fn read_u32(&self, addr: u32) -> u32 {
        match self.mem.read_u32(addr) {
            Ok(v) => v,
            Err(_) => self.read_slow(addr, 4) as u32,
        }
    }

    #[inline]
    pub fn read_u64(&self, addr: u32) -> u64 {
        match self.mem.read_u64(addr) {
            Ok(v) => v,
            Err(_) => (self.read_slow(addr, 4) << 32) | self.read_slow(addr.wrapping_add(4), 4),
        }
    }

    #[inline]
    pub fn write_u8(&self, addr: u32, v: u8) {
        if self.mem.write_u8(addr, v).is_err() {
            self.write_slow(addr, 1, u64::from(v));
        }
        self.watched(addr, 1);
    }

    #[inline]
    pub fn write_u16(&self, addr: u32, v: u16) {
        if self.mem.write_u16(addr, v).is_err() {
            self.write_slow(addr, 2, u64::from(v));
        }
        self.watched(addr, 2);
    }

    #[inline]
    pub fn write_u32(&self, addr: u32, v: u32) {
        if self.mem.write_u32(addr, v).is_err() {
            self.write_slow(addr, 4, u64::from(v));
        }
        self.watched(addr, 4);
    }

    #[inline]
    pub fn write_u64(&self, addr: u32, v: u64) {
        if self.mem.write_u64(addr, v).is_err() {
            self.write_slow(addr, 4, v >> 32);
            self.write_slow(addr.wrapping_add(4), 4, v & 0xFFFF_FFFF);
        }
        self.watched(addr, 8);
    }

    /// Reads `n` (1..=8) bytes as a big-endian integer.
    pub fn read_be(&self, addr: u32, n: u32) -> u64 {
        (0..n).fold(0, |acc, i| {
            (acc << 8) | u64::from(self.read_u8(addr.wrapping_add(i)))
        })
    }

    /// Writes the low `n` (1..=8) bytes of `v` big-endian.
    pub fn write_be(&self, addr: u32, n: u32, v: u64) {
        for i in 0..n {
            self.write_u8(addr.wrapping_add(i), (v >> (8 * (n - 1 - i))) as u8);
        }
    }

    /// `memmove` within emulated memory.
    pub fn copy(&self, dst: u32, src: u32, n: u32) {
        let mut buf = vec![0; n as usize];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.read_u8(src.wrapping_add(i as u32));
        }
        for (i, b) in buf.iter().enumerate() {
            self.write_u8(dst.wrapping_add(i as u32), *b);
        }
    }

    pub fn fill(&self, dst: u32, byte: u8, n: u32) {
        for i in 0..n {
            self.write_u8(dst.wrapping_add(i), byte);
        }
    }

    // Dispatch.

    /// Registers a native implementation for the function at `addr`.
    pub fn register(&self, addr: u32, native: Native) {
        self.set_flag(addr, FLAG_NATIVE, true);
        self.dispatch.borrow_mut().insert(
            addr,
            Entry {
                native,
                mode: Mode::Native,
                external: false,
                returns: Returns::Unknown,
            },
        );
    }

    /// Registers a port of the game function at `addr`, whose result is where `returns` says.
    pub fn register_port(&self, addr: u32, native: Native, returns: Returns) {
        self.register(addr, native);
        if let Some(e) = self.dispatch.borrow_mut().get_mut(&addr) {
            e.returns = returns;
        }
    }

    /// Marks the native implementation at `addr` as external (see [`Entry::external`]).
    pub fn mark_external(&self, addr: u32) {
        if let Some(e) = self.dispatch.borrow_mut().get_mut(&addr) {
            e.external = true;
        }
    }

    /// Removes the native implementation at `addr`.
    pub fn unregister(&self, addr: u32) {
        self.set_flag(addr, FLAG_NATIVE, false);
        self.dispatch.borrow_mut().remove(&addr);
    }

    pub fn set_mode(&self, addr: u32, mode: Mode) {
        if let Some(e) = self.dispatch.borrow_mut().get_mut(&addr) {
            e.mode = mode;
        }
    }

    /// Sets every registered function's mode.
    pub fn set_all_modes(&self, mode: Mode) {
        for e in self.dispatch.borrow_mut().values_mut() {
            e.mode = mode;
        }
    }

    pub fn entry(&self, addr: u32) -> Option<Entry> {
        self.dispatch.borrow().get(&addr).copied()
    }

    pub fn registered(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.dispatch.borrow().keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Runs the function at `addr` with arguments already in registers.
    pub fn invoke(&self, addr: u32) {
        let Some(e) = self.entry(addr) else {
            return self.run_original(addr);
        };
        match e.mode {
            Mode::Native if e.external && self.lockstep.is_active() => {
                lockstep::external(self, addr, e.native)
            }
            Mode::Native => (e.native)(self),
            Mode::Original if self.has_backend() => self.run_original(addr),
            Mode::Lockstep if self.has_backend() && !self.lockstep.is_active() => {
                lockstep::run(self, addr, e.native, e.returns)
            }
            // Nested inside a lockstep run: the original side runs originals throughout.
            Mode::Lockstep if self.has_backend() && self.lockstep.in_original() => {
                self.run_original(addr)
            }
            _ => (e.native)(self),
        }
    }

    /// Runs the original code at `addr`, which needs a backend.
    pub fn run_original(&self, addr: u32) {
        match self.backend.get() {
            Some(b) => b.run(self, addr),
            None => panic!("no implementation of {}", self.name_of(addr)),
        }
    }

    // Hooks run when original code reaches an address; ported code calls `run_hook`.

    pub fn set_hook(&self, addr: u32, hook: Hook) {
        self.set_flag(addr, FLAG_HOOK, true);
        self.hooks.borrow_mut().insert(addr, hook);
    }

    pub fn remove_hook(&self, addr: u32) {
        self.set_flag(addr, FLAG_HOOK, false);
        self.hooks.borrow_mut().remove(&addr);
    }

    pub fn has_hook(&self, addr: u32) -> bool {
        self.hooks.borrow().contains_key(&addr)
    }

    pub fn run_hook(&self, addr: u32) {
        let hook = self.hooks.borrow().get(&addr).cloned();
        if let Some(hook) = hook {
            hook(self);
        }
    }

    // Stack.

    /// Reserves `size` bytes on the emulated stack for locals whose address escapes. The
    /// block starts 8 bytes above r1, leaving room for a callee's back chain and saved LR.
    pub fn stack_alloc(&self, size: u32) -> StackFrame<'_> {
        let old = self.regs.r(1);
        let new = (old - (size + 8)) & !0xF;
        self.write_u32(new, old);
        self.regs.set_r(1, new);
        StackFrame {
            ctx: self,
            old,
            base: new + 8,
        }
    }
}

/// Space on the emulated stack; restores r1 when dropped.
pub struct StackFrame<'a> {
    ctx: &'a Ctx,
    old: u32,
    base: u32,
}

impl<'a> StackFrame<'a> {
    /// Address of the reserved block.
    pub fn base(&self) -> u32 {
        self.base
    }

    /// The reserved block as a handle.
    pub fn get<H: Handle<'a>>(&self) -> H {
        At::new(self.ctx, self.base).field(0)
    }
}

impl Drop for StackFrame<'_> {
    fn drop(&mut self) {
        self.ctx.regs.set_r(1, self.old);
    }
}
