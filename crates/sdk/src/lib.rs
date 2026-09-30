// SPDX-License-Identifier: GPL-3.0-or-later

//! The SDK layer: what stands in for the GameCube hardware and the parts of Nintendo's SDK that
//! talk to it directly.
//!
//! Most of the SDK runs as ordinary game code. Devices whose SDK drivers are simple register
//! pokes (VI, DVD, the GX FIFO, ARAM and audio DMA) are emulated at the register level, and
//! their interrupts reach the SDK's own handlers. The rest (alarms, thread sleeps, controllers,
//! memory cards, the DSP, SRAM) is replaced at the API level.
//!
//! Time is virtual. It only moves at wait points, such as a thread going to sleep or the game
//! waiting for the next controller poll, which then jump to the next scheduled event. Events
//! and interrupts are only delivered at those points, so runs are deterministic and do not
//! depend on how fast the code runs or on which functions are ported.

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::rc::Rc;
use std::sync::OnceLock;

use ssbm_disc::Disc;
use ssbm_rt::Ctx;

pub mod boot;
mod devices;
mod gp;
pub mod hw;
mod os;
mod printf;

pub use hw::Hw;

/// Time base ticks per second (a quarter of the 162 MHz bus clock).
pub const TB_HZ: u64 = 40_500_000;

/// Address of the OS's interrupt handler table.
const INTERRUPT_TABLE: u32 = 0x8000_3040;
/// OS interrupt masks: global (`__OSMaskInterrupts`) and per-thread (`OSSetInterruptMask`).
const INTERRUPT_MASK_GLOBAL: u32 = 0x8000_00C4;
const INTERRUPT_MASK_USER: u32 = 0x8000_00C8;
/// The current thread's `OSContext`.
const CURRENT_CONTEXT: u32 = 0x8000_00D4;

/// OS interrupt numbers (`__OSInterrupt`).
pub mod irq {
    pub const DSP_AI: u32 = 5;
    pub const DSP_ARAM: u32 = 6;
    pub const DSP_DSP: u32 = 7;
    pub const PI_CP: u32 = 17;
    pub const PI_PE_TOKEN: u32 = 18;
    pub const PI_PE_FINISH: u32 = 19;
    pub const PI_DI: u32 = 21;
    pub const PI_VI: u32 = 24;
}

type Event = Box<dyn FnOnce(&Ctx)>;

/// Receives `OSReport` output a line at a time.
pub type ReportSink = Box<dyn FnMut(&str)>;

struct Scheduled {
    at: u64,
    seq: u64,
    run: Event,
}

impl PartialEq for Scheduled {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.seq) == (other.at, other.seq)
    }
}

impl Eq for Scheduled {}

impl PartialOrd for Scheduled {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scheduled {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.at, self.seq).cmp(&(other.at, other.seq))
    }
}

/// Everything the SDK layer keeps per machine. Lives in the `Ctx` as an extension.
pub struct Sdk {
    pub disc: RefCell<Disc>,
    events: RefCell<BinaryHeap<Reverse<Scheduled>>>,
    next_seq: Cell<u64>,
    /// Raised interrupts not yet delivered, as OS interrupt mask bits.
    pending: Cell<u32>,
    in_service: Cell<bool>,
    pub hw: Hw,
    pub os: os::Os,
    pub dev: devices::Devices,
    /// Where `OSReport` output goes.
    pub report: RefCell<ReportSink>,
}

impl Sdk {
    fn new(disc: Disc) -> Self {
        Self {
            disc: RefCell::new(disc),
            events: RefCell::default(),
            next_seq: Cell::new(0),
            pending: Cell::new(0),
            in_service: Cell::new(false),
            hw: Hw::default(),
            os: os::Os::default(),
            dev: devices::Devices::default(),
            report: RefCell::new(Box::new(|s| print!("{s}"))),
        }
    }

    /// Reads raw bytes from the disc.
    pub fn read_disc(&self, offset: u64, buf: &mut [u8]) {
        self.disc
            .borrow_mut()
            .read_at(offset, buf)
            .unwrap_or_else(|e| panic!("disc read of {} bytes at {offset:#x}: {e}", buf.len()));
    }

    /// The current time base value.
    pub fn now(ctx: &Ctx) -> u64 {
        ctx.regs.tb.get()
    }

    /// Runs `run` at time base value `at`, at the first wait point after it.
    pub fn schedule(&self, at: u64, run: impl FnOnce(&Ctx) + 'static) {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        self.events.borrow_mut().push(Reverse(Scheduled {
            at,
            seq,
            run: Box::new(run),
        }));
    }

    /// Runs `run` `delay` ticks from now.
    pub fn after(&self, ctx: &Ctx, delay: u64, run: impl FnOnce(&Ctx) + 'static) {
        self.schedule(Self::now(ctx) + delay, run);
    }

    /// Raises OS interrupt `n`; its handler runs at the next wait point where it is unmasked.
    pub fn raise(&self, n: u32) {
        self.pending.set(self.pending.get() | (0x8000_0000 >> n));
    }

    fn next_due(&self, now: u64) -> Option<Event> {
        let mut events = self.events.borrow_mut();
        if events.peek().is_some_and(|e| e.0.at <= now) {
            events.pop().map(|e| e.0.run)
        } else {
            None
        }
    }

    /// Delivers one unmasked pending interrupt to the OS's handler. Returns false if none.
    fn deliver_one(&self, ctx: &Ctx) -> bool {
        let masked = ctx.read_u32(INTERRUPT_MASK_GLOBAL) | ctx.read_u32(INTERRUPT_MASK_USER);
        let ready = self.pending.get() & !masked;
        if ready == 0 {
            return false;
        }
        let n = ready.leading_zeros();
        self.pending.set(self.pending.get() & !(0x8000_0000 >> n));
        let handler = ctx.read_u32(INTERRUPT_TABLE + 4 * n);
        if handler != 0 {
            let context = ctx.read_u32(CURRENT_CONTEXT);
            ctx.call::<_, ()>(handler, (n as i16, context));
        }
        true
    }

    /// Runs everything due at the current time: events, then the interrupts they raised.
    fn service(&self, ctx: &Ctx) {
        loop {
            if let Some(run) = self.next_due(Self::now(ctx)) {
                run(ctx);
            } else if !self.deliver_one(ctx) {
                break;
            }
        }
    }

    /// Runs `f` the way an interrupt would: interrupts off, and every register restored after,
    /// except the time base.
    fn as_interrupt(&self, ctx: &Ctx, f: impl FnOnce()) {
        assert!(
            !self.in_service.get(),
            "a wait point was reached from inside an interrupt or event"
        );
        self.in_service.set(true);
        let saved = ctx.regs.snapshot();
        ctx.regs.msr.set(saved.msr & !MSR_EE);
        f();
        let now = ctx.regs.tb.get();
        ctx.regs.restore(&saved);
        ctx.regs.tb.set(now);
        self.in_service.set(false);
    }

    /// Wait point that does not let time pass: delivers what is already due.
    pub fn poll(ctx: &Ctx) {
        let sdk = ctx.ext::<Sdk>();
        if sdk.in_service.get() {
            return;
        }
        sdk.as_interrupt(ctx, || sdk.service(ctx));
    }

    /// Wait point for code that cannot continue until something happens: jumps to the next
    /// event and delivers everything due then.
    pub fn idle(ctx: &Ctx) {
        Self::wait(ctx, u64::MAX);
    }

    /// Like `idle`, but lets at most `max_ticks` pass. For polling loops that also run once
    /// outside the loop, where a long jump would skip work the game does between polls.
    pub fn wait(ctx: &Ctx, max_ticks: u64) {
        let sdk = ctx.ext::<Sdk>();
        sdk.as_interrupt(ctx, || {
            let now = ctx.regs.tb.get();
            let next = sdk.events.borrow().peek().map(|e| e.0.at);
            match next {
                Some(at) => ctx
                    .regs
                    .tb
                    .set(now.max(at.min(now.saturating_add(max_ticks)))),
                None if sdk.pending.get() == 0 => panic!(
                    "deadlock: the game is waiting at {:#010X} and nothing is scheduled",
                    ctx.regs.lr.get()
                ),
                None => {}
            }
            sdk.service(ctx);
        });
    }
}

const MSR_EE: u32 = 1 << 15;

/// The address of a decomp symbol. Panics on unknown names.
pub fn sym(name: &str) -> u32 {
    static MAP: OnceLock<HashMap<&'static str, u32>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        ssbm_types::symbols::SYMBOLS
            .iter()
            .map(|&(addr, _, name, _)| (name, addr))
            .collect()
    });
    *map.get(name)
        .unwrap_or_else(|| panic!("no symbol named {name}"))
}

/// Puts the SDK layer into `ctx`: device registers, stand-in functions and wait points.
pub fn install(ctx: &Ctx, disc: Disc) -> Rc<Sdk> {
    let sdk = ctx.set_ext(Sdk::new(disc));
    ctx.set_mmio(Box::new(hw::Mmio));
    os::install(ctx);
    devices::install(ctx);
    hw::install(ctx);
    // The game's own wait loops (for loads, and for the next controller poll) all call
    // lb_800195D0 on each spin.
    ctx.set_hook(
        sym("lb_800195D0"),
        Rc::new(|ctx| Sdk::wait(ctx, GAME_WAIT_STEP)),
    );
    sdk
}

/// Most time that passes per spin of one of the game's wait loops: a quarter millisecond.
const GAME_WAIT_STEP: u64 = TB_HZ / 4000;
