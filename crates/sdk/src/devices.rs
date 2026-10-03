// SPDX-License-Identifier: GPL-3.0-or-later

//! API-level stand-ins for controllers (PAD), memory cards (CARD) when slot A has none, and the
//! DSP.

use std::cell::{Cell, RefCell};

use ssbm_rt::{Ctx, Native};

use crate::{Sdk, TB_HZ, sym};

/// One controller as `PADRead` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadStatus {
    pub connected: bool,
    pub button: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_l: u8,
    pub trigger_r: u8,
    pub analog_a: u8,
    pub analog_b: u8,
}

/// `PAD_ERR_NO_CONTROLLER`.
const PAD_ERR_NO_CONTROLLER: i8 = -1;
/// `CARD_RESULT_NOCARD`.
const CARD_RESULT_NOCARD: i32 = -3;
/// Time the DSP takes to mix one audio frame.
const DSP_FRAME_TICKS: u64 = TB_HZ / 2000;

pub struct Devices {
    pub pads: RefCell<[PadStatus; 4]>,
    /// The DSP task (the AX microcode), and mail it is waiting on.
    dsp_task: Cell<u32>,
    dsp_init: Cell<bool>,
    mail: RefCell<Vec<u32>>,
    /// Whether the DSP mixes sound (AX) rather than only answering the CPU, and the microcode
    /// doing it once its task is added.
    pub mix_audio: Cell<bool>,
    ax: RefCell<Option<ssbm_ax::Ax>>,
    ax_observer: RefCell<Option<AxObserver>>,
}

/// Sees each AX command list before the DSP runs it: memory as it stands, the microcode's hash,
/// and the list's address and size.
pub type AxObserver = Box<dyn FnMut(&dyn ssbm_ax::Bus, u32, u32, u16)>;

impl Devices {
    /// Shows `f` each command list the DSP mixes (with `mix_audio`).
    pub fn observe_ax(&self, f: AxObserver) {
        *self.ax_observer.borrow_mut() = Some(f);
    }
}

impl Default for Devices {
    fn default() -> Self {
        let pad = PadStatus {
            connected: true,
            ..PadStatus::default()
        };
        Self {
            pads: RefCell::new([pad; 4]),
            dsp_task: Cell::new(0),
            dsp_init: Cell::new(false),
            mail: RefCell::default(),
            mix_audio: Cell::new(false),
            ax: RefCell::default(),
            ax_observer: RefCell::default(),
        }
    }
}

pub(crate) fn install(ctx: &Ctx, card: bool) {
    let reg = |name: &str, f: Native| ctx.register(sym(name), f);

    // Controllers.
    reg("PADInit", |ctx| ret(ctx, 1));
    reg("PADReset", |ctx| ret(ctx, 1));
    reg("PADRecalibrate", |ctx| ret(ctx, 1));
    reg("PADRead", pad_read);
    reg("PADControlMotor", |_| {});
    reg("PADSetSpec", |_| {});
    reg("PADSetSamplingRate", |_| {});
    reg("SIRefreshSamplingRate", |_| {});

    // Memory cards: none inserted, unless slot A has one, which the CARD library drives.
    if card {
        return install_dsp(ctx);
    }
    reg("CARDInit", |_| {});
    reg("CARDProbe", |ctx| ret(ctx, 0));
    reg("CARDGetXferredBytes", |ctx| ret(ctx, 0));
    for name in [
        "CARDProbeEx",
        "CARDMountAsync",
        "CARDCheckAsync",
        "CARDFormatAsync",
        "CARDCreateAsync",
        "CARDDeleteAsync",
        "CARDOpen",
        "CARDFastOpen",
        "CARDClose",
        "CARDRead",
        "CARDReadAsync",
        "CARDWrite",
        "CARDWriteAsync",
        "CARDGetStatus",
        "CARDSetStatusAsync",
        "CARDRenameAsync",
        "CARDFreeBlocks",
        "CARDUnmount",
    ] {
        reg(name, |ctx| ret(ctx, CARD_RESULT_NOCARD as u32));
    }
    install_dsp(ctx);
}

fn install_dsp(ctx: &Ctx) {
    let reg = |name: &str, f: Native| ctx.register(sym(name), f);
    // DSP: the only task is the AX microcode, which the stand-in runs.
    reg("DSPInit", |ctx| ctx.ext::<Sdk>().dev.dsp_init.set(true));
    reg("DSPCheckInit", |ctx| {
        ret(ctx, u32::from(ctx.ext::<Sdk>().dev.dsp_init.get()))
    });
    reg("DSPAddTask", dsp_add_task);
    reg("DSPAssertTask", |_| {});
    reg("DSPSendMailToDSP", dsp_send_mail);
    reg("DSPCheckMailToDSP", |ctx| ret(ctx, 0));
    reg("DSPCheckMailFromDSP", |ctx| ret(ctx, 0));
    reg("DSPReadMailFromDSP", |ctx| ret(ctx, 0));
}

/// Main memory and ARAM as the DSP reads and writes them.
struct DspBus<'a> {
    ctx: &'a Ctx,
    aram: &'a crate::hw::Hw,
}

impl ssbm_ax::Bus for DspBus<'_> {
    fn read(&self, addr: u32, out: &mut [u8]) {
        if self.ctx.mem.read_bytes((addr & 0x01FF_FFFF) | 0x8000_0000, out).is_err() {
            out.fill(0);
        }
    }

    fn write(&mut self, addr: u32, data: &[u8]) {
        let _ = self.ctx.mem.write_bytes((addr & 0x01FF_FFFF) | 0x8000_0000, data);
    }

    fn aram(&self, addr: u32) -> u8 {
        self.aram.aram_byte(addr)
    }
}

fn ret(ctx: &Ctx, v: u32) {
    ctx.regs.set_r(3, v);
}

fn pad_read(ctx: &Ctx) {
    let status = ctx.regs.r(3);
    let pads = *ctx.ext::<Sdk>().dev.pads.borrow();
    for (i, p) in pads.iter().enumerate() {
        let at = status + 12 * i as u32;
        let bytes = if p.connected {
            let [b0, b1] = p.button.to_be_bytes();
            [
                b0,
                b1,
                p.stick_x as u8,
                p.stick_y as u8,
                p.substick_x as u8,
                p.substick_y as u8,
                p.trigger_l,
                p.trigger_r,
                p.analog_a,
                p.analog_b,
                0,
                0,
            ]
        } else {
            let mut b = [0u8; 12];
            b[10] = PAD_ERR_NO_CONTROLLER as u8;
            b
        };
        let _ = ctx.dma_write(at, &bytes);
    }
    ctx.regs.set_r(3, 0);
}

// DSPTaskInfo offsets.
const TASK_STATE: u32 = 0x00;
const TASK_IRAM_MMEM_ADDR: u32 = 0x0C;
const TASK_IRAM_LENGTH: u32 = 0x10;
const TASK_INIT_CB: u32 = 0x28;
const TASK_RES_CB: u32 = 0x2C;

fn dsp_add_task(ctx: &Ctx) {
    let task = ctx.regs.r(3);
    let sdk = ctx.ext::<Sdk>();
    sdk.dev.dsp_task.set(task);
    if sdk.dev.mix_audio.get() {
        // The microcode, by Dolphin's hash of it, which tells the AX versions apart.
        let len = ctx.read_u32(task + TASK_IRAM_LENGTH) as usize;
        let mut ucode = vec![0; len.min(0x2000)];
        let _ = ctx.mem.read_bytes(ctx.read_u32(task + TASK_IRAM_MMEM_ADDR) | 0x8000_0000, &mut ucode);
        let crc = ssbm_ax::ucode_crc(&ucode);
        *sdk.dev.ax.borrow_mut() = Some(ssbm_ax::Ax::new(crc, Some(ssbm_ax::coefs::table())));
    }
    // The microcode boots and reports in: the SDK's handler would call the init callback.
    ctx.write_u32(task + TASK_STATE, 1);
    let init = ctx.read_u32(task + TASK_INIT_CB);
    let saved = ctx.regs.snapshot();
    if init != 0 {
        ctx.call::<_, ()>(init, (task,));
    }
    let now = ctx.regs.tb.get();
    ctx.regs.restore(&saved);
    ctx.regs.tb.set(now);
    ctx.regs.set_r(3, task);
}

/// AX sends `0xBABE0180` and then a command list address each audio frame. The stand-in
/// finishes the frame after a fixed time and yields, which resumes the task.
fn dsp_send_mail(ctx: &Ctx) {
    let mail = ctx.regs.r(3);
    let sdk = ctx.ext::<Sdk>();
    let mut pending = sdk.dev.mail.borrow_mut();
    pending.push(mail);
    if pending.len() < 2 || pending[0] >> 16 != 0xBABE {
        if pending[0] >> 16 != 0xBABE {
            pending.clear();
        }
        return;
    }
    let (size, list) = (pending[0] as u16, pending[1]);
    pending.clear();
    drop(pending);
    if let Some(ax) = sdk.dev.ax.borrow_mut().as_mut() {
        let mut bus = DspBus { ctx, aram: &sdk.hw };
        if let Some(observe) = sdk.dev.ax_observer.borrow_mut().as_mut() {
            observe(&bus, ax.crc(), list & 0x01FF_FFFF, size);
        }
        ax.run(&mut bus, list & 0x01FF_FFFF, size);
    }
    let task = sdk.dev.dsp_task.get();
    sdk.after(ctx, DSP_FRAME_TICKS, move |ctx| {
        ctx.write_u32(task + TASK_STATE, 1);
        let res = ctx.read_u32(task + TASK_RES_CB);
        if res != 0 {
            ctx.call::<_, ()>(res, (task,));
        }
    });
}
