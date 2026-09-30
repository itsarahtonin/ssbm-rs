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
pub mod cpu;
mod jump;
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
    /// Goes on running original code at `pc`, as after a `longjmp`, until the function the run
    /// started with returns.
    fn resume(&self, ctx: &Ctx, pc: u32);
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
    /// Ports running, innermost last. A panic leaves it as it was, for the report.
    natives: RefCell<Vec<u32>>,
    /// Jump buffers and where they were saved, latest last.
    jump_targets: RefCell<Vec<(u32, jump::Target)>>,
    /// Every jump buffer ever saved to.
    jump_buffers: RefCell<std::collections::BTreeSet<u32>>,
    /// The innermost interpreter run's id, and the last id given out.
    current_run: Cell<u64>,
    last_run: Cell<u64>,
    /// Where the original code that called the running native continues, if not after the call.
    resume_at: Cell<Option<u32>>,
    /// A debugging watchpoint: writes to `[start, start + len)` call `watch_hook`.
    watch: Cell<(u32, u32)>,
    watch_hook: RefCell<Option<WatchHook>>,
    /// How often original code was entered at each address, when counted.
    original_entries: RefCell<Option<HashMap<u32, u64>>>,
    /// Registers a port for an address a call reaches without one, such as code placed at run
    /// time, and says whether it did.
    resolver: RefCell<Option<Resolver>>,
    /// Stack below a lockstep check's r1 that its original run has written, while tracked, and
    /// what hears of reads of the rest.
    shadow: RefCell<Option<StackShadow>>,
    shadow_on: Cell<bool>,
    /// Whether original code, rather than a port, an SDK stand-in or a hook, is running.
    running_original: Cell<bool>,
    uninit_hook: RefCell<Option<UninitHook>>,
    /// The running thread's stack, as (lowest address, top), where the OS layer knows it.
    stack_bounds: RefCell<Option<StackBounds>>,
}

/// See `Ctx::set_stack_bounds`.
pub type StackBounds = Box<dyn Fn(&Ctx) -> Option<(u32, u32)>>;

/// See `Ctx::set_uninit_hook`: the address and length of a read of stack nothing wrote.
pub type UninitHook = Box<dyn Fn(&Ctx, u32, u32)>;

/// Which bytes of `[lo, lo + written.len())` have been written.
struct StackShadow {
    lo: u32,
    written: Vec<bool>,
}

/// See `Ctx::set_resolver`.
pub type Resolver = Box<dyn Fn(&Ctx, u32) -> bool>;

/// Called with the address and size of a write to the watched range.
pub type WatchHook = Rc<dyn Fn(&Ctx, u32, u32)>;

/// `MSR[EE]`: external interrupts enabled.
pub const MSR_EE: u32 = 1 << 15;

/// Instructions between heartbeats.
pub const HEARTBEAT: u64 = 1 << 24;

/// Time base ticks that pass per read of its low word: about 200 CPU cycles.
pub const TB_READ_STEP: u64 = 16;

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
            natives: RefCell::default(),
            jump_targets: RefCell::default(),
            jump_buffers: RefCell::default(),
            current_run: Cell::new(0),
            last_run: Cell::new(0),
            resume_at: Cell::new(None),
            watch: Cell::new((0, 0)),
            watch_hook: RefCell::default(),
            original_entries: RefCell::default(),
            resolver: RefCell::default(),
            shadow: RefCell::default(),
            shadow_on: Cell::new(false),
            running_original: Cell::new(false),
            uninit_hook: RefCell::default(),
            stack_bounds: RefCell::default(),
        }
    }

    /// Sets what tells the running thread's stack, so what lies below it is not taken for
    /// stack.
    pub fn set_stack_bounds(&self, bounds: StackBounds) {
        *self.stack_bounds.borrow_mut() = Some(bounds);
    }

    /// The lowest address of the stack `below` bytes under `sp`, no lower than the end of the
    /// running thread's stack when `sp` is in it.
    pub fn stack_floor(&self, sp: u32, below: u32) -> u32 {
        let floor = sp.saturating_sub(below);
        let bounds = self.stack_bounds.borrow().as_ref().and_then(|f| f(self));
        match bounds {
            Some((end, top)) if end <= sp && sp <= top => floor.max(end),
            _ => floor,
        }
    }

    /// Sets what hears of reads of stack that nothing wrote since tracking began, which
    /// lockstep tracks through the original runs of its checks while this is set.
    pub fn set_uninit_hook(&self, hook: UninitHook) {
        *self.uninit_hook.borrow_mut() = Some(hook);
    }

    pub(crate) fn tracks_uninit(&self) -> bool {
        self.uninit_hook.borrow().is_some()
    }

    /// Starts tracking which bytes of `[lo, hi)` are written, all unwritten so far.
    pub(crate) fn begin_stack_shadow(&self, lo: u32, hi: u32) {
        *self.shadow.borrow_mut() = Some(StackShadow {
            lo,
            written: vec![false; hi.wrapping_sub(lo) as usize],
        });
        self.shadow_on.set(true);
    }

    pub(crate) fn end_stack_shadow(&self) {
        self.shadow_on.set(false);
        *self.shadow.borrow_mut() = None;
    }

    /// A frame is being allocated from `sp` down to `new_sp`: none of its bytes hold anything
    /// of its function's yet, whatever earlier calls left there.
    #[inline]
    pub fn stack_allocated(&self, new_sp: u32, sp: u32) {
        if self.shadow_on.get() {
            self.shadow_unwrite(new_sp, sp);
        }
    }

    #[cold]
    fn shadow_unwrite(&self, lo: u32, hi: u32) {
        if let Some(sh) = self.shadow.borrow_mut().as_mut() {
            for addr in lo..hi {
                let off = addr.wrapping_sub(sh.lo) as usize;
                if off < sh.written.len() {
                    sh.written[off] = false;
                }
            }
        }
    }

    #[cold]
    fn shadow_write(&self, addr: u32, len: u32) {
        if let Some(sh) = self.shadow.borrow_mut().as_mut() {
            for i in 0..len {
                let off = addr.wrapping_add(i).wrapping_sub(sh.lo) as usize;
                if off < sh.written.len() {
                    sh.written[off] = true;
                }
            }
        }
    }

    #[cold]
    fn shadow_read(&self, addr: u32, len: u32) {
        let unwritten = self.shadow.borrow().as_ref().is_some_and(|sh| {
            (0..len).any(|i| {
                let off = addr.wrapping_add(i).wrapping_sub(sh.lo) as usize;
                off < sh.written.len() && !sh.written[off]
            })
        });
        if unwritten && self.running_original.get() {
            // The hook may read memory itself.
            self.shadow_on.set(false);
            if let Some(hook) = self.uninit_hook.borrow().as_ref() {
                hook(self, addr, len);
            }
            self.shadow_on.set(true);
        }
    }

    /// Sets what a call to an address without a port asks first: it may register one there,
    /// for code whose address is only known at run time.
    pub fn set_resolver(&self, resolver: Resolver) {
        *self.resolver.borrow_mut() = Some(resolver);
    }

    /// Starts counting where calls enter original code.
    pub fn count_original_entries(&self) {
        *self.original_entries.borrow_mut() = Some(HashMap::new());
    }

    /// Addresses where calls entered original code, and how often, most often first.
    pub fn original_entries(&self) -> Vec<(u32, u64)> {
        let mut out: Vec<(u32, u64)> = self
            .original_entries
            .borrow()
            .as_ref()
            .map(|m| m.iter().map(|(&a, &n)| (a, n)).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
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

    /// `mftb` of the time base's low word. Time passes between reads, so loops that wait on
    /// the time base finish.
    pub fn read_tbl(&self) -> u32 {
        let v = self.regs.tb.get() as u32;
        self.tick(TB_READ_STEP);
        v
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
        if self.shadow_on.get() {
            self.shadow_read(addr, 1);
        }
        match self.mem.read_u8(addr) {
            Ok(v) => v,
            Err(_) => self.read_slow(addr, 1) as u8,
        }
    }

    #[inline]
    pub fn read_u16(&self, addr: u32) -> u16 {
        if self.shadow_on.get() {
            self.shadow_read(addr, 2);
        }
        match self.mem.read_u16(addr) {
            Ok(v) => v,
            Err(_) => self.read_slow(addr, 2) as u16,
        }
    }

    #[inline]
    pub fn read_u32(&self, addr: u32) -> u32 {
        if self.shadow_on.get() {
            self.shadow_read(addr, 4);
        }
        match self.mem.read_u32(addr) {
            Ok(v) => v,
            Err(_) => self.read_slow(addr, 4) as u32,
        }
    }

    #[inline]
    pub fn read_u64(&self, addr: u32) -> u64 {
        if self.shadow_on.get() {
            self.shadow_read(addr, 8);
        }
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
        if self.shadow_on.get() {
            self.shadow_write(addr, 1);
        }
    }

    #[inline]
    pub fn write_u16(&self, addr: u32, v: u16) {
        if self.mem.write_u16(addr, v).is_err() {
            self.write_slow(addr, 2, u64::from(v));
        }
        self.watched(addr, 2);
        if self.shadow_on.get() {
            self.shadow_write(addr, 2);
        }
    }

    #[inline]
    pub fn write_u32(&self, addr: u32, v: u32) {
        if self.mem.write_u32(addr, v).is_err() {
            self.write_slow(addr, 4, u64::from(v));
        }
        self.watched(addr, 4);
        if self.shadow_on.get() {
            self.shadow_write(addr, 4);
        }
    }

    #[inline]
    pub fn write_u64(&self, addr: u32, v: u64) {
        if self.mem.write_u64(addr, v).is_err() {
            self.write_slow(addr, 4, v >> 32);
            self.write_slow(addr.wrapping_add(4), 4, v & 0xFFFF_FFFF);
        }
        self.watched(addr, 8);
        if self.shadow_on.get() {
            self.shadow_write(addr, 8);
        }
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

    /// Writes `bytes` at `dst`.
    pub fn write_bytes(&self, dst: u32, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            self.write_u8(dst.wrapping_add(i as u32), *b);
        }
    }

    /// Writes a block into memory as a device's DMA does, past any hardware registers, where
    /// the debugging watchpoint and lockstep's stack tracking see it.
    pub fn dma_write(&self, dst: u32, bytes: &[u8]) -> ssbm_mem::Result<()> {
        let result = self.mem.write_bytes(dst, bytes);
        self.watched(dst, bytes.len() as u32);
        if self.shadow_on.get() {
            self.shadow_write(dst, bytes.len() as u32);
        }
        result
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
    /// An external stand-in already there stays: it replaces the original on purpose.
    pub fn register_port(&self, addr: u32, native: Native, returns: Returns) {
        if self.entry(addr).is_some_and(|e| e.external) {
            return;
        }
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
        if !self.lockstep.trace_calls.get() {
            return self.invoke_traced(addr);
        }
        let args = [self.regs.r(3), self.regs.r(4), self.regs.r(5), self.regs.r(6)];
        let trace = self.lockstep.call_starts(addr, args);
        self.invoke_traced(addr);
        self.lockstep.call_ends(trace);
    }

    fn invoke_traced(&self, addr: u32) {
        let resolved = || {
            let resolver = self.resolver.borrow();
            resolver.as_ref().is_some_and(|r| r(self, addr))
        };
        let Some(e) = self
            .entry(addr)
            .or_else(|| resolved().then(|| self.entry(addr)).flatten())
        else {
            return self.run_original(addr);
        };
        match e.mode {
            Mode::Native if e.external && self.lockstep.is_active() => {
                lockstep::external(self, addr, e.native)
            }
            Mode::Native if e.external => {
                let original = self.running_original.replace(false);
                (e.native)(self);
                self.running_original.set(original);
            }
            Mode::Original if self.has_backend() => self.run_original(addr),
            // The original side of a check runs originals throughout; anywhere else a call to a
            // port under lockstep is checked, nested inside any check already running, unless
            // that function is already under check: nested checks of a recursion would rerun
            // the rest of it at every level.
            Mode::Lockstep if self.has_backend() && self.lockstep.in_original() => {
                self.run_original(addr)
            }
            Mode::Lockstep if self.has_backend() && !self.lockstep.is_checking(addr) => {
                lockstep::run(self, addr, e.native, e.returns)
            }
            _ => self.run_native(addr, e.native),
        }
    }

    /// Runs a port, after any hook at its entry, which original code would run on reaching it.
    pub(crate) fn run_native(&self, addr: u32, native: Native) {
        if self.flags_at(addr) & FLAG_HOOK != 0 {
            self.run_hook(addr);
        }
        self.natives.borrow_mut().push(addr);
        let original = self.running_original.replace(false);
        native(self);
        self.running_original.set(original);
        self.natives.borrow_mut().pop();
    }

    /// The ports that were running when a run stopped, innermost last.
    pub fn native_stack(&self) -> Vec<u32> {
        self.natives.borrow().clone()
    }

    pub(crate) fn natives_depth(&self) -> usize {
        self.natives.borrow().len()
    }

    /// Forgets ports a caught panic left running.
    pub(crate) fn truncate_natives(&self, depth: usize) {
        self.natives.borrow_mut().truncate(depth);
    }

    /// Makes the original code that called the running native continue at `addr` rather than
    /// after the call, as a `longjmp` to the original's `setjmp` does.
    pub fn resume_at(&self, addr: u32) {
        self.resume_at.set(Some(addr));
    }

    /// Where the native that just returned asked its caller to continue.
    pub fn take_resume_at(&self) -> Option<u32> {
        self.resume_at.take()
    }

    /// Runs the original code at `addr`, which needs a backend.
    pub fn run_original(&self, addr: u32) {
        let Some(b) = self.backend.get() else {
            panic!("no implementation of {}", self.name_of(addr));
        };
        if let Some(m) = self.original_entries.borrow_mut().as_mut() {
            *m.entry(addr).or_default() += 1;
        }
        let run = self.last_run.get() + 1;
        self.last_run.set(run);
        let outer = self.current_run.replace(run);
        let was_original = self.running_original.get();
        let mut resume = None;
        // A `longjmp` to a buffer saved in this run unwinds to here and goes on from there.
        loop {
            self.running_original.set(true);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match resume {
                None => b.run(self, addr),
                Some(pc) => b.resume(self, pc),
            }));
            match result {
                Ok(()) => break,
                Err(p) => match self.landing(run, p) {
                    Ok(pc) => resume = Some(pc),
                    Err(p) => {
                        self.current_run.set(outer);
                        self.running_original.set(was_original);
                        std::panic::resume_unwind(p);
                    }
                },
            }
        }
        self.current_run.set(outer);
        self.running_original.set(was_original);
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
            let original = self.running_original.replace(false);
            if self.lockstep.is_active() {
                lockstep::hook(self, addr, &hook);
            } else {
                hook(self);
            }
            self.running_original.set(original);
        }
    }

    // Stack.

    /// A stack frame of exactly `size` bytes, as a function's prologue (`stwu r1, -size(r1)`)
    /// makes one; the block for locals starts 8 bytes above r1, past the back chain and the
    /// word a callee saves LR in.
    pub fn stack_frame(&self, size: u32) -> StackFrame<'_> {
        let old = self.regs.r(1);
        let new = old - size;
        self.stack_allocated(new, old);
        self.write_u32(new, old);
        self.regs.set_r(1, new);
        StackFrame {
            ctx: self,
            old,
            base: new + 8,
        }
    }

    /// Stores at `addr` a struct of 4 or 8 bytes that a function returned in r3 and r4, as
    /// the EABI returns structs of up to 8 bytes.
    pub fn put_small_ret(&self, addr: u32, size: u32) {
        self.write_u32(addr, self.regs.r(3));
        if size > 4 {
            self.write_u32(addr + 4, self.regs.r(4));
        }
    }

    /// Returns the struct of 4 or 8 bytes at `addr` in r3 and r4, as the EABI does.
    pub fn take_small_ret(&self, addr: u32, size: u32) {
        self.regs.set_r(3, self.read_u32(addr));
        if size > 4 {
            self.regs.set_r(4, self.read_u32(addr + 4));
        }
    }

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

    /// Saves the argument registers where a variadic function's prologue does: r3 to r10 at
    /// the block's start, then f1 to f8 as doubles if CR bit 6 says the caller passed floats.
    pub fn save_varargs(&self) {
        let ctx = self.ctx;
        for i in 0..8 {
            ctx.write_u32(self.base + 4 * i as u32, ctx.regs.r(3 + i));
        }
        if ctx.regs.cr.get() & (1 << (31 - 6)) != 0 {
            for i in 0..8 {
                ctx.write_u64(self.base + 0x20 + 8 * i as u32, ctx.regs.f(1 + i).to_bits());
            }
        }
    }

    /// Fills the `va_list` at `ap` as MWCC's `__builtin_va_info` does, for a function whose
    /// fixed parameters take `gpr` general and `fpr` float registers: the arguments after them
    /// are in the registers `save_varargs` saved, then in the caller's argument area.
    pub fn va_info(&self, ap: u32, gpr: u8, fpr: u8) {
        let ctx = self.ctx;
        ctx.write_u32(ap, (u32::from(gpr) << 24) | (u32::from(fpr) << 16));
        ctx.write_u32(ap + 4, self.old + 8);
        ctx.write_u32(ap + 8, self.base);
    }
}

impl Drop for StackFrame<'_> {
    fn drop(&mut self) {
        self.ctx.regs.set_r(1, self.old);
    }
}
