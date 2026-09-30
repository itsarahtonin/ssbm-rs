// SPDX-License-Identifier: GPL-3.0-or-later

//! API-level stand-ins for controllers (PAD), memory cards (CARD) and the DSP.

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
        }
    }
}

pub(crate) fn install(ctx: &Ctx) {
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

    // Memory cards: none inserted.
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
const TASK_INIT_CB: u32 = 0x28;
const TASK_RES_CB: u32 = 0x2C;

fn dsp_add_task(ctx: &Ctx) {
    let task = ctx.regs.r(3);
    let sdk = ctx.ext::<Sdk>();
    sdk.dev.dsp_task.set(task);
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
    pending.clear();
    drop(pending);
    let task = sdk.dev.dsp_task.get();
    sdk.after(ctx, DSP_FRAME_TICKS, move |ctx| {
        ctx.write_u32(task + TASK_STATE, 1);
        let res = ctx.read_u32(task + TASK_RES_CB);
        if res != 0 {
            ctx.call::<_, ()>(res, (task,));
        }
    });
}
