// SPDX-License-Identifier: GPL-3.0-or-later
// Register behavior follows Dolphin's hardware emulation (GPL-2.0-or-later).

//! Hardware registers at `0xCC000000` for the devices whose SDK drivers run as original code:
//! VI, the DVD interface, ARAM and audio DMA, the GX FIFO (CP, PE, PI and the write-gather
//! pipe), and EXI with the memory cards in slots A and B when there are any. Other EXI transfers
//! finish as they start, with no device answering: Slippi's device, and the memory cards when
//! there are none, are stood in for above the registers. Registers without a model keep the last
//! value written. `DVD_RETRY=N` makes one data read in N fail once, as on a scratched disc; a
//! read can fail for good, the disc cover open and close, and a memory card come out and go
//! back in.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use ssbm_rt::Ctx;

use crate::card::{self, Card};
use crate::gp::{Effect, Gp};
use crate::{Sdk, TB_HZ, irq};

const BASE: u32 = 0xCC00_0000;
const SIZE: u32 = 0x1_0000;
const PIPE: u32 = 0xCC00_8000;
const ARAM_SIZE: usize = 16 << 20;

// Offsets from BASE.
const CP_STATUS: u32 = 0x0000;
const CP_CTRL: u32 = 0x0002;
const CP_FIFO_BASE: u32 = 0x0020;
const CP_FIFO_END: u32 = 0x0024;
const CP_FIFO_DISTANCE: u32 = 0x0030;
const CP_FIFO_WPTR: u32 = 0x0034;
const CP_FIFO_RPTR: u32 = 0x0038;
const PE_INTERRUPT: u32 = 0x100A;
const PE_TOKEN: u32 = 0x100E;
const VI_DI0: u32 = 0x2030;
const VI_VCT: u32 = 0x202C;
const VI_HCT: u32 = 0x202E;
const PI_INTSR: u32 = 0x3000;
const PI_FIFO_BASE: u32 = 0x300C;
const PI_FIFO_END: u32 = 0x3010;
const PI_FIFO_WPTR: u32 = 0x3014;
const PI_FLIPPER_REV: u32 = 0x302C;
const DSP_CSR: u32 = 0x500A;
const AR_INFO: u32 = 0x5012;
const AR_MODE: u32 = 0x5016;
const AR_DMA_MM: u32 = 0x5020;
const AR_DMA_AR: u32 = 0x5024;
const AR_DMA_CNT_H: u32 = 0x5028;
const AR_DMA_CNT_L: u32 = 0x502A;
const AI_DMA_START: u32 = 0x5030;
const AI_DMA_CONTROL: u32 = 0x5036;
const AI_DMA_LEFT: u32 = 0x503A;
const DI_SR: u32 = 0x6000;
const DI_CVR: u32 = 0x6004;
const DI_CMD: u32 = 0x6008;
const DI_MAR: u32 = 0x6014;
const DI_LENGTH: u32 = 0x6018;
const DI_CR: u32 = 0x601C;
/// Each EXI channel's control register, whose bit 0 starts a transfer and reads as 0 once it
/// is done.
const EXI_CR: [u32; 3] = [0x680C, 0x6820, 0x6834];
/// Each EXI channel's status register, the slot A channel's first.
const EXI_CSR: [u32; 3] = [0x6800, 0x6814, 0x6828];
/// Offsets from a channel's status register of its DMA address, DMA length and immediate data
/// registers.
const EXI_MAR: u32 = 0x4;
const EXI_LENGTH: u32 = 0x8;
const EXI_DATA: u32 = 0x10;
// EXI status register bits.
const EXI_INT: u32 = 0x0002;
const EXI_TCINT: u32 = 0x0008;
const EXI_CS0: u32 = 0x0080;
const EXI_EXTINTMASK: u32 = 0x0400;
const EXI_EXTINT: u32 = 0x0800;
const EXI_EXT: u32 = 0x1000;
const DI_IMM: u32 = 0x6020;
const AI_CR: u32 = 0x6C00;
const AI_SCNT: u32 = 0x6C08;

// DSP CSR bits.
const CSR_AIDINT: u16 = 0x08;
const CSR_AIDINTMSK: u16 = 0x10;
const CSR_ARINT: u16 = 0x20;
const CSR_ARINTMSK: u16 = 0x40;
const CSR_DSPINT: u16 = 0x80;

// PE interrupt register bits.
const PE_TOKEN_ENABLE: u16 = 1;
const PE_FINISH_ENABLE: u16 = 2;
const PE_TOKEN_INT: u16 = 4;
const PE_FINISH_INT: u16 = 8;

/// Time base ticks that pass per register read.
const MMIO_READ_STEP: u64 = 8;
/// Time from starting a DVD command to its interrupt, plus a read's transfer time at the disc
/// rate if one is set (`set_disc_rate`). Slippi runs with fast disc speed.
const DVD_LATENCY: u64 = TB_HZ / 2000;
/// NTSC lines per frame, and the half-line width VCT/HCT count in.
const VI_LINES: u64 = 525;
const VI_HCT_PER_LINE: u64 = 858;

/// Start of video field `n`: NTSC runs at 60000/1001 fields per second.
pub fn field_start(n: u64) -> u64 {
    (u128::from(n) * u128::from(TB_HZ) * 1001 / 60_000) as u64
}

/// The EXI channel an EXI register at `off` belongs to.
fn exi_chan(off: u32) -> u32 {
    (off - 0x6800) / 0x14
}

/// The device registers, with the SDK layer they belong to: every GX command the game sends
/// is a write here, so it doesn't look the layer up in the context each time.
pub(crate) struct Mmio(pub(crate) Weak<Sdk>);

impl Mmio {
    fn sdk(&self) -> Rc<Sdk> {
        self.0.upgrade().expect("the SDK layer outlives its context's MMIO")
    }
}

impl ssbm_rt::Mmio for Mmio {
    fn read(&self, ctx: &Ctx, addr: u32, size: u32) -> u32 {
        self.sdk().hw.read(ctx, addr, size)
    }

    fn write(&self, ctx: &Ctx, addr: u32, size: u32, value: u32) {
        let sdk = self.sdk();
        sdk.hw.write(ctx, &sdk, addr, size, value);
    }
}

pub struct Hw {
    regs: RefCell<Vec<u16>>,
    /// Bytes written to the gather pipe since the last 32-byte burst.
    pipe: RefCell<Vec<u8>>,
    /// BP registers whose writes print who made them (GX_TRACE_BP), and whether the last byte
    /// written to the pipe began a BP command.
    trace_bp: RefCell<Vec<u8>>,
    bp_next: Cell<bool>,
    gp: RefCell<Gp>,
    aram: RefCell<Vec<u8>>,
    audio_out: RefCell<Option<Box<dyn FnMut(&[u8])>>>,
    /// Bumped to cancel scheduled audio DMA interrupts.
    ai_generation: Cell<u64>,
    /// The scheduled end of the current audio DMA block, to cancel when the DMA stops.
    ai_event: Cell<Option<u64>>,
    /// Video fields shown so far, and when the current frame started.
    pub fields: Cell<u64>,
    frame_start: Cell<u64>,
    /// Streaming audio sample counter: its value when the rate or state last changed, and when.
    ais_base: Cell<u32>,
    ais_since: Cell<u64>,
    /// The memory cards in slots A and B (EXI channels 0 and 1), if any, and whether each is in
    /// its slot (`set_card_present`).
    pub cards: [Option<Card>; 2],
    card_present: [Cell<bool>; 2],
    /// One data read in this many fails, as on a scratched disc, with an error the DVD driver
    /// retries (`DVD_RETRY`); the read again works. Zero for none.
    dvd_retry: u32,
    dvd_reads: Cell<u32>,
    /// The error the drive reports when next asked.
    dvd_error: Cell<u32>,
    /// An error the next data read fails with, as `fail_next_dvd_read` sets it.
    dvd_fail_next: Cell<Option<u32>>,
    /// Whether the disc cover is open: DICVR's low bit, which writes don't change.
    dvd_cover_open: Cell<bool>,
    /// Bytes a second disc reads transfer at, if not at once (`set_disc_rate`).
    disc_rate: Cell<Option<u64>>,
}

impl Default for Hw {
    fn default() -> Self {
        let hw = Self {
            regs: RefCell::new(vec![0; (SIZE / 2) as usize]),
            pipe: RefCell::default(),
            trace_bp: RefCell::default(),
            bp_next: Cell::new(false),
            gp: RefCell::default(),
            aram: RefCell::new(vec![0; ARAM_SIZE]),
            audio_out: RefCell::default(),
            ai_generation: Cell::new(0),
            ai_event: Cell::new(None),
            fields: Cell::new(0),
            frame_start: Cell::new(0),
            ais_base: Cell::new(0),
            ais_since: Cell::new(0),
            cards: [None, None],
            card_present: [Cell::new(true), Cell::new(true)],
            dvd_retry: std::env::var("DVD_RETRY")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&n: &u32| n >= 2)
                .unwrap_or(0),
            dvd_reads: Cell::new(0),
            dvd_error: Cell::new(0),
            dvd_fail_next: Cell::new(None),
            dvd_cover_open: Cell::new(false),
            disc_rate: Cell::new(None),
        };
        hw.set32(PI_FLIPPER_REV, 0x2465_00B1);
        hw
    }
}

impl Hw {
    fn get16(&self, off: u32) -> u16 {
        self.regs.borrow()[(off / 2) as usize]
    }

    fn set16(&self, off: u32, v: u16) {
        self.regs.borrow_mut()[(off / 2) as usize] = v;
    }

    fn get32(&self, off: u32) -> u32 {
        (u32::from(self.get16(off)) << 16) | u32::from(self.get16(off + 2))
    }

    fn set32(&self, off: u32, v: u32) {
        self.set16(off, (v >> 16) as u16);
        self.set16(off + 2, v as u16);
    }

    /// A GX FIFO pointer register pair (low half first).
    fn cp_ptr(&self, off: u32) -> u32 {
        (u32::from(self.get16(off + 2)) << 16) | u32::from(self.get16(off))
    }

    fn set_cp_ptr(&self, off: u32, v: u32) {
        self.set16(off, v as u16);
        self.set16(off + 2, (v >> 16) as u16);
    }

    /// Records a Dolphin FIFO log of `count` frames from frame `start` to `path`.
    /// Draws the GPU's command stream with `sink` (a renderer).
    pub fn set_renderer(&self, sink: Box<dyn ssbm_gx::Sink>) {
        self.gp.borrow_mut().set_sink(sink);
    }

    pub fn record_dff(&self, path: std::path::PathBuf, start: u64, count: usize) {
        self.gp.borrow_mut().record_dff(path, start, count);
    }

    /// Prints who writes the BP registers `regs` to the pipe, and the frame (diagnostics).
    pub fn trace_bp(&self, regs: Vec<u8>) {
        *self.trace_bp.borrow_mut() = regs;
    }

    /// Lets the renderer finish what it was given, at the end of a run.
    pub fn finish_renderer(&self) {
        self.gp.borrow_mut().finish();
    }

    /// Commands the GP has run, for diagnostics.
    pub fn draws(&self) -> u64 {
        self.gp.borrow().draws
    }

    /// Logs the GX command stream frame by frame to `frames`, and frame `detail`'s commands,
    /// decoded, to its writer (gp.rs).
    pub fn record_gx(
        &self,
        frames: Box<dyn std::io::Write>,
        detail: Option<(u64, Box<dyn std::io::Write>)>,
    ) {
        self.gp.borrow_mut().record(frames, detail);
    }

    /// The top-field framebuffer VI scans out.
    pub fn xfb(&self) -> u32 {
        let v = self.get32(0x201C);
        let addr = (v & 0x00FF_FFFF) << if v & 0x1000_0000 != 0 { 5 } else { 0 };
        0x8000_0000 | addr
    }

    fn read(&self, ctx: &Ctx, addr: u32, size: u32) -> u32 {
        // Time passes between reads, so loops that poll a register finish.
        ctx.tick(MMIO_READ_STEP);
        let off = addr - BASE;
        match (off, size) {
            (CP_STATUS, 2) => 0x000E, // FIFO empty, reads and commands idle
            (VI_VCT | VI_HCT, 2) => self.beam(ctx, off),
            (PI_INTSR, 4) => 0x0001_0000, // reset switch released
            (AI_DMA_LEFT, 2) => 0,
            (AR_MODE, 2) => u32::from(self.get16(off)) | 1, // ARAM ready
            (AI_SCNT, 4) => self.ais_count(ctx),
            (DI_CVR, 4) => (self.get32(off) & !1) | u32::from(self.dvd_cover_open.get()),
            // A card in slot A or B is attached.
            (0x6800 | 0x6814, 4) if self.card_in(exi_chan(off)).is_some() => {
                self.get32(off) | EXI_EXT
            }
            (_, 4) => self.get32(off),
            (_, 2) => u32::from(self.get16(off)),
            (_, 1) => u32::from((self.get16(off & !1) >> if off & 1 == 0 { 8 } else { 0 }) as u8),
            _ => unreachable!(),
        }
    }

    /// VI beam position within the current frame.
    fn beam(&self, ctx: &Ctx, off: u32) -> u32 {
        let frame = field_start(2) - field_start(0);
        let t = (Sdk::now(ctx) - self.frame_start.get()).min(frame - 1);
        let line = frame / VI_LINES;
        if off == VI_VCT {
            (1 + t / line) as u32
        } else {
            (1 + (t % line) * VI_HCT_PER_LINE / line) as u32
        }
    }

    fn write(&self, ctx: &Ctx, sdk: &Rc<Sdk>, addr: u32, size: u32, value: u32) {
        if (PIPE..PIPE + 8).contains(&addr) {
            return self.pipe_write(ctx, sdk, size, value);
        }
        let off = addr - BASE;
        // CP, PE, VI, MI and DSP registers are 16-bit; the rest are 32-bit.
        let narrow = matches!(off >> 12, 0 | 1 | 2 | 4 | 5);
        match (size, narrow) {
            (4, true) => {
                self.write16(ctx, sdk, off, (value >> 16) as u16);
                self.write16(ctx, sdk, off + 2, value as u16);
            }
            (2, true) => self.write16(ctx, sdk, off, value as u16),
            (4, false) => self.write32(ctx, sdk, off, value),
            (2, false) => {
                let word = off & !3;
                let old = self.get32(word);
                let shift = if off & 2 == 0 { 16 } else { 0 };
                let v = (old & !(0xFFFF << shift)) | ((value & 0xFFFF) << shift);
                self.write32(ctx, sdk, word, v);
            }
            _ => {
                let word = off & !1;
                let shift = if off & 1 == 0 { 8 } else { 0 };
                let v = (self.get16(word) & !(0xFF << shift)) | ((value as u16 & 0xFF) << shift);
                self.set16(word, v);
            }
        }
    }

    fn write16(&self, ctx: &Ctx, sdk: &Rc<Sdk>, off: u32, v: u16) {
        let old = self.get16(off);
        self.set16(off, v);
        match off {
            PE_INTERRUPT => {
                // Enables take the written value; pending flags clear when written as 1.
                let pending = PE_TOKEN_INT | PE_FINISH_INT;
                let enables = PE_TOKEN_ENABLE | PE_FINISH_ENABLE;
                self.set16(off, (v & enables) | (old & pending & !v));
            }
            DSP_CSR => {
                // Interrupt flags clear when written as 1.
                let flags = CSR_AIDINT | CSR_ARINT | CSR_DSPINT;
                self.set16(off, (v & !flags) | (old & flags & !v));
            }
            AR_DMA_CNT_L => self.aram_dma(ctx, sdk),
            AI_DMA_CONTROL => {
                let was = old & 0x8000 != 0;
                let now = v & 0x8000 != 0;
                if now && !was {
                    self.ai_play(ctx);
                    self.ai_schedule(ctx, sdk);
                } else if !now {
                    self.ai_generation.set(self.ai_generation.get() + 1);
                    if let Some(seq) = self.ai_event.take() {
                        sdk.cancel(seq);
                    }
                }
            }
            _ => {}
        }
    }

    fn write32(&self, ctx: &Ctx, sdk: &Rc<Sdk>, off: u32, v: u32) {
        let old = self.get32(off);
        self.set32(off, v);
        match off {
            DI_SR | DI_CVR => {
                // DEINT, TCINT, BRKINT (and CVRINT) clear when written as 1.
                let flags = if off == DI_SR { 0x54 } else { 0x4 };
                self.set32(off, (v & !flags) | (old & flags & !v));
            }
            DI_CR if v & 1 != 0 => self.dvd_command(ctx, sdk),
            AI_CR => {
                // Rebase the stream sample counter on the old settings, then apply the new.
                self.set32(off, old);
                let count = if v & 0x20 != 0 {
                    0
                } else {
                    self.ais_count(ctx)
                };
                self.ais_base.set(count);
                self.ais_since.set(Sdk::now(ctx));
                self.set32(off, v & !0x28); // AIINT and SCRESET read as 0
            }
            PI_FIFO_WPTR => self.set32(off, v & 0x03FF_FFE0),
            _ if EXI_CSR.contains(&off) => self.exi_status(ctx, sdk, off, old, v),
            0x680C | 0x6820 if v & 1 != 0 && self.selected_card(exi_chan(off)) => {
                self.card_transfer(ctx, sdk, exi_chan(off), v)
            }
            _ if EXI_CR.contains(&off) => self.set32(off, v & !1),
            _ => {}
        }
    }

    // EXI and the memory cards in slots A and B, on channels 0 and 1.

    /// The card in channel `chan`'s slot, if there is one and it is in.
    fn card_in(&self, chan: u32) -> Option<&Card> {
        let chan = chan as usize;
        let present = self.card_present.get(chan)?.get();
        self.cards.get(chan)?.as_ref().filter(|_| present)
    }

    fn selected_card(&self, chan: u32) -> bool {
        self.card_in(chan).is_some() && self.get32(EXI_CSR[chan as usize]) & EXI_CS0 != 0
    }

    /// Pulls the card out of slot A (`slot` 0) or B (1), or puts it back. Its EXT line changes,
    /// which raises the channel's external interrupt when the driver has unmasked it.
    pub fn set_card_present(&self, ctx: &Ctx, slot: usize, present: bool) {
        if self.cards[slot].is_none() || self.card_present[slot].replace(present) == present {
            return;
        }
        let csr = self.get32(EXI_CSR[slot]) | EXI_EXTINT;
        self.set32(EXI_CSR[slot], csr);
        if csr & EXI_EXTINTMASK != 0 {
            ctx.ext::<Sdk>().raise(ctx, irq::EXI0_EXT + 3 * slot as u32);
        }
    }

    /// A write to an EXI status register: its interrupt flags clear when written as 1, and
    /// selecting or deselecting the card in slot A or B starts or ends a command.
    fn exi_status(&self, ctx: &Ctx, sdk: &Rc<Sdk>, off: u32, old: u32, v: u32) {
        let flags = EXI_INT | EXI_TCINT | EXI_EXTINT;
        self.set32(off, (v & !flags) | (old & flags & !v));
        // A flag cleared before its interrupt was taken takes the interrupt back.
        let chan = exi_chan(off);
        for (flag, n) in [(EXI_INT, 9), (EXI_TCINT, 10), (EXI_EXTINT, 11)] {
            if v & flag != 0 {
                sdk.withdraw(n + 3 * chan);
            }
        }
        let Some(card) = self.card_in(chan) else {
            return;
        };
        if (old ^ v) & EXI_CS0 == 0 {
            return;
        }
        if v & EXI_CS0 != 0 {
            card.select();
        } else if let card::Done::Busy = card.deselect() {
            // The erase or program is done at once: the game's waits for the card spin
            // without letting time pass.
            card.finish();
            if card.interrupts() {
                self.set32(off, self.get32(off) | EXI_INT);
                sdk.raise(ctx, irq::EXI0_EXI + 3 * chan);
            }
        }
    }

    /// A transfer with the card in channel `chan`'s slot, done at once, with its transfer
    /// interrupt raised.
    fn card_transfer(&self, ctx: &Ctx, sdk: &Rc<Sdk>, chan: u32, cr: u32) {
        let Some(card) = self.card_in(chan) else {
            return;
        };
        let csr = EXI_CSR[chan as usize];
        let write = (cr >> 2) & 3;
        if cr & 2 == 0 {
            let n = ((cr >> 4) & 3) as usize + 1;
            let data = self.get32(csr + EXI_DATA).to_be_bytes();
            if write != 0 {
                card.write(&data[..n]);
            }
            if write != 1 {
                let mut out = [0u8; 4];
                out[..n].copy_from_slice(&card.read(n));
                self.set32(csr + EXI_DATA, u32::from_be_bytes(out));
            }
            self.exi_done(ctx, sdk, chan);
            return;
        }
        let mar = self.get32(csr + EXI_MAR) & 0x03FF_FFE0;
        let len = self.get32(csr + EXI_LENGTH) as usize;
        if write == 1 {
            let mut data = vec![0; len];
            let _ = ctx.mem.read_bytes(0x8000_0000 | mar, &mut data);
            card.write(&data);
        } else {
            let _ = ctx.dma_write(0x8000_0000 | mar, &card.read(len));
        }
        self.exi_done(ctx, sdk, chan);
    }

    /// Channel `chan`'s transfer is done: its control register reads idle and its transfer
    /// interrupt is raised.
    fn exi_done(&self, ctx: &Ctx, sdk: &Rc<Sdk>, chan: u32) {
        let (csr, cr) = (EXI_CSR[chan as usize], EXI_CR[chan as usize]);
        self.set32(cr, self.get32(cr) & !1);
        self.set32(csr, self.get32(csr) | EXI_TCINT);
        sdk.raise(ctx, irq::EXI0_TC + 3 * chan);
    }

    // Write-gather pipe and GX FIFO.

    fn pipe_write(&self, ctx: &Ctx, sdk: &Rc<Sdk>, size: u32, value: u32) {
        if !self.trace_bp.borrow().is_empty() {
            if size == 4 && self.bp_next.get() && self.trace_bp.borrow().contains(&((value >> 24) as u8)) {
                let callers: Vec<String> =
                    ctx.native_stack().iter().rev().take(4).map(|&a| ctx.name_of(a)).collect();
                eprintln!(
                    "gx bp frame {} {:08X} from {}",
                    self.gp.borrow().frame(),
                    value,
                    callers.join(" < ")
                );
            }
            self.bp_next.set(size == 1 && value == 0x61);
        }
        let mut pipe = self.pipe.borrow_mut();
        pipe.extend_from_slice(&value.to_be_bytes()[(4 - size) as usize..]);
        while pipe.len() >= 32 {
            let mut burst = [0; 32];
            burst.copy_from_slice(&pipe[..32]);
            pipe.drain(..32);
            self.burst(ctx, sdk, &burst);
        }
    }

    /// Writes 32 bytes to the CPU FIFO in memory and, when the GP reads the same FIFO, runs
    /// the commands.
    fn burst(&self, ctx: &Ctx, sdk: &Rc<Sdk>, data: &[u8]) {
        let wptr = self.get32(PI_FIFO_WPTR);
        let _ = ctx.dma_write(0x8000_0000 | wptr, data);
        let next = if wptr == self.get32(PI_FIFO_END) & 0x03FF_FFE0 {
            self.get32(PI_FIFO_BASE) & 0x03FF_FFE0
        } else {
            wptr + 32
        };
        self.set32(PI_FIFO_WPTR, next);

        let ctrl = self.get16(CP_CTRL);
        if ctrl & 0x10 == 0 {
            return; // GP not linked: the CPU is building a display list
        }
        self.set_cp_ptr(CP_FIFO_WPTR, next);
        if ctrl & 1 == 0 {
            let d = self.cp_ptr(CP_FIFO_DISTANCE) + 32;
            self.set_cp_ptr(CP_FIFO_DISTANCE, d);
            return; // GP reads disabled
        }
        // Run from the GP's read pointer up to the write pointer.
        let mut effects = Vec::new();
        let (base, end) = (
            self.cp_ptr(CP_FIFO_BASE) & 0x03FF_FFE0,
            self.cp_ptr(CP_FIFO_END) & 0x03FF_FFE0,
        );
        let mut rptr = self.cp_ptr(CP_FIFO_RPTR) & 0x03FF_FFE0;
        self.gp.borrow_mut().set_fifo(base, end);
        let mut chunk = [0u8; 32];
        for _ in 0..(1 << 20) {
            if rptr == next {
                break;
            }
            let _ = ctx.mem.read_bytes(0x8000_0000 | rptr, &mut chunk);
            self.gp.borrow_mut().feed(ctx, &chunk, &mut effects);
            rptr = if rptr == end { base } else { rptr + 32 };
        }
        self.set_cp_ptr(CP_FIFO_RPTR, rptr);
        self.set_cp_ptr(CP_FIFO_DISTANCE, 0);
        for e in effects {
            self.pe_effect(ctx, sdk, e);
        }
    }

    fn pe_effect(&self, ctx: &Ctx, sdk: &Sdk, e: Effect) {
        let int = self.get16(PE_INTERRUPT);
        match e {
            Effect::Finish => {
                if int & PE_FINISH_ENABLE != 0 {
                    self.set16(PE_INTERRUPT, int | PE_FINISH_INT);
                    sdk.raise(ctx, irq::PI_PE_FINISH);
                }
            }
            Effect::Token { token, interrupt } => {
                self.set16(PE_TOKEN, token);
                if interrupt && int & PE_TOKEN_ENABLE != 0 {
                    self.set16(PE_INTERRUPT, int | PE_TOKEN_INT);
                    sdk.raise(ctx, irq::PI_PE_TOKEN);
                }
            }
        }
    }

    // DVD interface.

    fn dvd_command(&self, ctx: &Ctx, sdk: &Rc<Sdk>) {
        let cmd = [
            self.get32(DI_CMD),
            self.get32(DI_CMD + 4),
            self.get32(DI_CMD + 8),
        ];
        let mar = self.get32(DI_MAR) & 0x03FF_FFE0;
        let len = self.get32(DI_LENGTH);
        let transfer = match self.disc_rate.get() {
            Some(rate) if cmd[0] >> 24 == 0xA8 => u64::from(len) * TB_HZ / rate,
            _ => 0,
        };
        sdk.after(ctx, DVD_LATENCY + transfer, move |ctx| {
            let sdk = ctx.ext::<Sdk>();
            let hw = &sdk.hw;
            let dma = |data: &[u8]| {
                let _ = ctx.dma_write(0x8000_0000 | mar, data);
                hw.set32(DI_MAR, mar + data.len() as u32);
                hw.set32(DI_LENGTH, 0);
            };
            let data_read = cmd[0] >> 24 == 0xA8 && cmd[0] & 0xFF != 0x40;
            let error = if hw.dvd_cover_open.get() && !matches!(cmd[0] >> 24, 0xE0 | 0xE3) {
                // With the cover open, the drive takes nothing but requests for its error and
                // to stop its motor.
                Some(0x0102_3A00)
            } else if data_read && let Some(error) = hw.dvd_fail_next.take() {
                Some(error)
            } else if data_read && hw.dvd_retry != 0 {
                let n = hw.dvd_reads.get() + 1;
                hw.dvd_reads.set(n);
                // An unrecovered read error, which the driver asks for and retries.
                (n % hw.dvd_retry == 0).then_some(0x0003_0200)
            } else {
                None
            };
            if let Some(error) = error {
                // The command ends with a device error, which the driver then asks for.
                hw.dvd_error.set(error);
                hw.set32(DI_CR, hw.get32(DI_CR) & !1);
                let sr = hw.get32(DI_SR) | 0x04;
                hw.set32(DI_SR, sr);
                if sr & 0x02 != 0 {
                    sdk.raise(ctx, irq::PI_DI);
                }
                return;
            }
            match cmd[0] >> 24 {
                0xA8 => {
                    let offset = if cmd[0] & 0xFF == 0x40 {
                        0
                    } else {
                        u64::from(cmd[1]) << 2
                    };
                    let mut data = vec![0; len as usize];
                    sdk.read_disc(offset, &mut data);
                    dma(&data);
                }
                0x12 => {
                    // Drive info, as Dolphin reports it.
                    let mut info = [0u8; 0x20];
                    info[..8].copy_from_slice(&[0x00, 0x02, 0x00, 0x06, 0x20, 0x02, 0x04, 0x02]);
                    dma(&info[..len.min(0x20) as usize]);
                }
                0xE0 => hw.set32(DI_IMM, hw.dvd_error.replace(0)),
                0xE2 => hw.set32(DI_IMM, 0),
                0xAB | 0xE1 | 0xE3 | 0xE4 => {}
                op => panic!("unsupported DVD command {op:#04X} ({:08X?})", cmd),
            }
            hw.set32(DI_CR, hw.get32(DI_CR) & !1);
            let sr = hw.get32(DI_SR) | 0x10;
            hw.set32(DI_SR, sr);
            if sr & 0x08 != 0 {
                sdk.raise(ctx, irq::PI_DI);
            }
        });
    }

    /// Opens or closes the disc cover. The cover's state is DICVR's low bit, and a change raises
    /// the DVD interface's interrupt when the driver waits for one, as it does once a command
    /// found the cover open.
    pub fn set_dvd_cover(&self, ctx: &Ctx, open: bool) {
        if self.dvd_cover_open.replace(open) == open {
            return;
        }
        let cvr = self.get32(DI_CVR) | 4;
        self.set32(DI_CVR, cvr);
        if cvr & 2 != 0 {
            ctx.ext::<Sdk>().raise(ctx, irq::PI_DI);
        }
    }

    /// Bytes a second disc reads transfer at, if not at once (`set_disc_rate`).
    pub fn disc_rate(&self) -> Option<u64> {
        self.disc_rate.get()
    }

    /// Gives disc reads a transfer time at `rate` bytes a second, on top of each command's
    /// latency, as a drive takes; `None` (the default) finishes each command in that latency,
    /// as Slippi's fast disc nearly does. The game's music needs a drive's pace: HSD starts a
    /// stream with AXSetVoiceAddr, which carries its loop flag, and sets the stream's addresses
    /// after five more reads (0x20 bytes, then four of 0x4000). If both land within one 5 ms AX
    /// frame, the SDK's __AXSyncPBs copies only the addresses to the DSP, the stale loop flag
    /// stops the voice at the end of its first buffer, and the song cuts out 1.8 s in. Those
    /// reads must take longer than a frame: any rate under about 13 MB/s does it.
    pub fn set_disc_rate(&self, rate: Option<u64>) {
        self.disc_rate.set(rate);
    }

    /// Fails the next data read with `error`, as a drive that can't go on reports: 0x00020400
    /// is one the driver treats as fatal.
    pub fn fail_next_dvd_read(&self, error: u32) {
        self.dvd_fail_next.set(Some(error));
    }

    // ARAM and audio DMA.

    /// ARAM DMA, including the address aliasing the SDK's ARAM size probe looks for: while the
    /// ARAM mode is 4, writes below 4 MB are mirrored 4 MB up, and addresses past the 16 MB of
    /// ARAM reach the (empty) expansion port.
    fn aram_dma(&self, ctx: &Ctx, sdk: &Rc<Sdk>) {
        let mm = 0x8000_0000 | (self.get32(AR_DMA_MM) & 0x03FF_FFE0);
        let ar = (self.get32(AR_DMA_AR) & 0x03FF_FFFF) as usize;
        let cnt_h = self.get16(AR_DMA_CNT_H);
        let len = ((u32::from(cnt_h & 0x3FF) << 16) | u32::from(self.get16(AR_DMA_CNT_L))) as usize;
        let to_mram = cnt_h & 0x8000 != 0;
        let probe_mirror = self.get16(AR_INFO) & 0xF == 4;
        let mut aram = self.aram.borrow_mut();
        for i in 0..len {
            let a = ar + i;
            let m = mm + i as u32;
            if to_mram {
                let v = if a < ARAM_SIZE { aram[a] } else { 0 };
                ctx.write_u8(m, v);
            } else if a < ARAM_SIZE {
                let v = ctx.read_u8(m);
                aram[a] = v;
                if probe_mirror && a < 0x40_0000 {
                    aram[a + 0x40_0000] = v;
                }
            }
        }
        drop(aram);
        self.set16(AR_DMA_CNT_H, cnt_h & 0x8000);
        self.set16(AR_DMA_CNT_L, 0);
        let csr = self.get16(DSP_CSR) | CSR_ARINT;
        self.set16(DSP_CSR, csr);
        if csr & CSR_ARINTMSK != 0 {
            sdk.raise(ctx, irq::DSP_ARAM);
        }
    }

    /// Schedules the end of the current audio DMA block.
    fn ai_schedule(&self, ctx: &Ctx, sdk: &Sdk) {
        let generation = self.ai_generation.get() + 1;
        self.ai_generation.set(generation);
        if let Some(seq) = self.ai_event.take() {
            sdk.cancel(seq);
        }
        let blocks = u64::from(self.get16(AI_DMA_CONTROL) & 0x7FFF).max(1);
        // 32-byte blocks of 16-bit stereo at 32 kHz.
        let ticks = blocks * 32 / 4 * TB_HZ / 32_000;
        let seq = sdk.after(ctx, ticks, move |ctx| {
            let sdk = ctx.ext::<Sdk>();
            let hw = &sdk.hw;
            if hw.ai_generation.get() != generation {
                return;
            }
            hw.ai_event.set(None);
            // The next block starts from the registers as they stand.
            hw.ai_play(ctx);
            let csr = hw.get16(DSP_CSR) | CSR_AIDINT;
            hw.set16(DSP_CSR, csr);
            if csr & CSR_AIDINTMSK != 0 {
                sdk.raise(ctx, irq::DSP_AI);
            }
            hw.ai_schedule(ctx, &sdk);
        });
        self.ai_event.set(Some(seq));
    }

    /// The streaming audio sample counter, which counts while a stream plays.
    fn ais_count(&self, ctx: &Ctx) -> u32 {
        let cr = self.get32(AI_CR);
        if cr & 1 == 0 {
            return self.ais_base.get();
        }
        let rate = if cr & 2 != 0 { 48_000 } else { 32_000 };
        let elapsed = Sdk::now(ctx) - self.ais_since.get();
        self.ais_base
            .get()
            .wrapping_add((elapsed * rate / TB_HZ) as u32)
    }

    /// A byte of ARAM, as the DSP's accelerator reads it.
    pub fn aram_byte(&self, addr: u32) -> u8 {
        self.aram.borrow().get(addr as usize).copied().unwrap_or(0)
    }

    /// Hands each audio DMA block to `sink` as it starts playing: 16-bit big-endian pairs at
    /// 32 kHz, as the audio interface reads them.
    pub fn set_audio_out(&self, sink: Box<dyn FnMut(&[u8])>) {
        *self.audio_out.borrow_mut() = Some(sink);
    }

    /// The block the audio DMA starts playing, to the audio sink.
    fn ai_play(&self, ctx: &Ctx) {
        let mut sink = self.audio_out.borrow_mut();
        let Some(sink) = sink.as_mut() else { return };
        let blocks = usize::from(self.get16(AI_DMA_CONTROL) & 0x7FFF).max(1);
        let mut data = vec![0; blocks * 32];
        if ctx.mem.read_bytes(self.ai_dma_start(), &mut data).is_err() {
            data.fill(0);
        }
        sink(&data);
    }

    /// The audio DMA block most recently started.
    pub fn ai_dma_start(&self) -> u32 {
        0x8000_0000 | (self.get32(AI_DMA_START) & 0x03FF_FFE0)
    }

    // Video.

    /// Starts video field `n` and raises the display interrupts that fall at its start.
    fn vi_field(&self, ctx: &Ctx, sdk: &Sdk, n: u64) {
        self.fields.set(n + 1);
        let second_half = n % 2 == 1;
        if !second_half {
            self.frame_start.set(field_start(n));
        }
        let mut raised = false;
        for i in 0..4 {
            let off = VI_DI0 + 4 * i;
            let di = self.get16(off);
            let vct = u64::from(di & 0x7FF);
            if di & 0x1000 != 0 && (vct > VI_LINES / 2) == second_half {
                self.set16(off, di | 0x8000);
                raised = true;
            }
        }
        if raised {
            sdk.raise(ctx, irq::PI_VI);
        }
        let next = n + 1;
        sdk.schedule(field_start(next), move |ctx| {
            let sdk = ctx.ext::<Sdk>();
            sdk.hw.vi_field(ctx, &sdk, next);
        });
    }
}

pub(crate) fn install(ctx: &Ctx) {
    let sdk = ctx.ext::<Sdk>();
    sdk.schedule(field_start(1), |ctx| {
        let sdk = ctx.ext::<Sdk>();
        sdk.hw.vi_field(ctx, &sdk, 1);
    });
}
