// SPDX-License-Identifier: GPL-3.0-or-later

//! OS stand-ins: reports, alarms, thread sleeps, SRAM, and the hardware setup inside `OSInit`.
//! Everything else in the OS (heaps, arenas, contexts, interrupt masks) runs as original code.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use ssbm_rt::{Args, Ctx, Native};

use crate::printf::{self, VaArgs};
use crate::{Sdk, sym};

/// OS time base offset: added to the time base to get the calendar-based system time.
const SYSTEM_TIME_OFFSET: u32 = 0x8000_30D8;

#[derive(Default)]
pub struct Os {
    /// Scheduled alarms, by address, with the generation of their current schedule.
    alarms: RefCell<HashMap<u32, u64>>,
    generation: Cell<u64>,
    /// The thread queue the (only) thread sleeps on, and whether it was woken.
    sleeping_on: Cell<Option<u32>>,
    woken: Cell<bool>,
    /// A partial `OSReport` line.
    line: RefCell<String>,
}

/// Where the OS keeps the running thread (`__gCurrentThread`), and where an `OSThread` keeps
/// its stack's top and lowest address.
const CURRENT_THREAD: u32 = 0x8000_00E4;
const THREAD_STACK_BASE: u32 = 0x304;
const THREAD_STACK_END: u32 = 0x308;

pub(crate) fn install(ctx: &Ctx, card: bool) {
    ctx.set_stack_bounds(Box::new(|ctx| {
        let thread = ctx.mem.read_u32(CURRENT_THREAD).ok().filter(|&t| t != 0)?;
        let top = ctx.mem.read_u32(thread.wrapping_add(THREAD_STACK_BASE)).ok()?;
        let end = ctx.mem.read_u32(thread.wrapping_add(THREAD_STACK_END)).ok()?;
        (end != 0 && end < top).then_some((end, top))
    }));
    let reg = |name: &str, f: Native| ctx.register(sym(name), f);
    reg("OSReport", os_report);
    reg("OSPanic", os_panic);
    reg("OSResetSystem", os_reset_system);

    reg("OSInitAlarm", |_| {});
    reg("OSSetAlarm", os_set_alarm);
    reg("OSSetPeriodicAlarm", os_set_periodic_alarm);
    reg("OSCancelAlarm", os_cancel_alarm);

    reg("OSSleepThread", os_sleep_thread);
    reg("OSWakeupThread", os_wakeup_thread);
    reg("__OSReschedule", |_| {});

    // Hardware setup inside OSInit, done by the device models instead.
    reg("__OSInitAudioSystem", |_| {});
    reg("__OSInitMemoryProtection", |_| {});
    // With a memory card, EXI runs as the original code, which installs its interrupt
    // handlers.
    if !card {
        reg("EXIInit", |_| {});
    }
    reg("SIInit", |_| {});
    reg("__OSInitSram", os_init_sram);
    reg("WriteSram", |ctx| ctx.regs.set_r(3, 1));
}

fn report(ctx: &Ctx, text: &str) {
    let sdk = ctx.ext::<Sdk>();
    let mut line = sdk.os.line.borrow_mut();
    line.push_str(text);
    while let Some(i) = line.find('\n') {
        let full: String = line.drain(..=i).collect();
        (sdk.report.borrow_mut())(&full);
    }
}

fn os_report(ctx: &Ctx) {
    let text = printf::format(ctx, ctx.regs.r(3), &mut VaArgs::new(ctx, 4));
    report(ctx, &text);
}

fn os_panic(ctx: &Ctx) {
    let file = printf::cstr(ctx, ctx.regs.r(3));
    let line = ctx.regs.r(4) as i32;
    let msg = printf::format(ctx, ctx.regs.r(5), &mut VaArgs::new(ctx, 6));
    report(ctx, &format!(" in \"{file}\" on line {line}.\n"));
    panic!("OSPanic at {file}:{line}: {msg}");
}

fn os_reset_system(ctx: &Ctx) {
    panic!(
        "the game reset the console (reset {}, code {:#x})",
        ctx.regs.r(3),
        ctx.regs.r(4)
    );
}

// Alarms: kept on the SDK layer's event queue instead of the decrementer.

fn system_time(ctx: &Ctx) -> i64 {
    (Sdk::now(ctx) as i64).wrapping_add(ctx.read_u64(SYSTEM_TIME_OFFSET) as i64)
}

fn schedule_alarm(ctx: &Ctx, alarm: u32, handler: u32, fire: i64) {
    let sdk = ctx.ext::<Sdk>();
    ctx.write_u32(alarm, handler);
    ctx.write_u64(alarm + 8, fire as u64);
    let generation = sdk.os.generation.get() + 1;
    sdk.os.generation.set(generation);
    sdk.os.alarms.borrow_mut().insert(alarm, generation);
    let offset = ctx.read_u64(SYSTEM_TIME_OFFSET) as i64;
    let at = fire.wrapping_sub(offset).max(Sdk::now(ctx) as i64) as u64;
    sdk.schedule(at, move |ctx| fire_alarm(ctx, alarm, generation));
}

/// When a periodic alarm fires next, as `InsertAlarm` computes it.
fn next_periodic(ctx: &Ctx, alarm: u32) -> i64 {
    let start = ctx.read_u64(alarm + 0x20) as i64;
    let period = ctx.read_u64(alarm + 0x18) as i64;
    let time = system_time(ctx);
    if start < time {
        start + period * ((time - start) / period + 1)
    } else {
        start
    }
}

fn fire_alarm(ctx: &Ctx, alarm: u32, generation: u64) {
    let sdk = ctx.ext::<Sdk>();
    if sdk.os.alarms.borrow().get(&alarm) != Some(&generation) {
        return; // cancelled or set again
    }
    sdk.os.alarms.borrow_mut().remove(&alarm);
    let handler = ctx.read_u32(alarm);
    ctx.write_u32(alarm, 0);
    if ctx.read_u64(alarm + 0x18) as i64 > 0 {
        schedule_alarm(ctx, alarm, handler, next_periodic(ctx, alarm));
    }
    let context = ctx.read_u32(0x8000_00D4);
    ctx.call::<_, ()>(handler, (alarm, context));
}

fn os_set_alarm(ctx: &Ctx) {
    let (alarm, tick, handler): (u32, i64, u32) = Args::take_all(ctx);
    ctx.write_u64(alarm + 0x18, 0);
    schedule_alarm(ctx, alarm, handler, system_time(ctx).wrapping_add(tick));
}

fn os_set_periodic_alarm(ctx: &Ctx) {
    let (alarm, start, period, handler): (u32, i64, i64, u32) = Args::take_all(ctx);
    ctx.write_u64(alarm + 0x18, period as u64);
    let offset = ctx.read_u64(SYSTEM_TIME_OFFSET) as i64;
    ctx.write_u64(alarm + 0x20, start.wrapping_add(offset) as u64);
    schedule_alarm(ctx, alarm, handler, next_periodic(ctx, alarm));
}

fn os_cancel_alarm(ctx: &Ctx) {
    let alarm = ctx.regs.r(3);
    let sdk = ctx.ext::<Sdk>();
    if ctx.read_u32(alarm) == 0 {
        return;
    }
    sdk.os.alarms.borrow_mut().remove(&alarm);
    ctx.write_u32(alarm, 0);
}

// Threads: the game only ever runs one, so sleeping means waiting for an event to wake it.

fn os_sleep_thread(ctx: &Ctx) {
    let sdk = ctx.ext::<Sdk>();
    sdk.os.sleeping_on.set(Some(ctx.regs.r(3)));
    sdk.os.woken.set(false);
    while !sdk.os.woken.get() {
        Sdk::idle(ctx);
    }
    sdk.os.sleeping_on.set(None);
}

fn os_wakeup_thread(ctx: &Ctx) {
    let sdk = ctx.ext::<Sdk>();
    if sdk.os.sleeping_on.get() == Some(ctx.regs.r(3)) {
        sdk.os.woken.set(true);
    }
}

// SRAM, as Dolphin initializes it: English, stereo, no progressive scan.

/// The 64 bytes of `OSSram` and `OSSramEx`.
pub fn default_sram() -> [u8; 64] {
    let mut s = [0u8; 64];
    s[19] = 0x2C; // flags: stereo, setup done, bit 5
    s[20..32].copy_from_slice(b"DOLPHINSLOTA");
    s[32..44].copy_from_slice(b"DOLPHINSLOTB");
    s[58] = 0x6E;
    s[59] = 0x6D;
    // Checksums over counterBias through flags.
    let (mut sum, mut inv) = (0u16, 0u16);
    for i in (12..20).step_by(2) {
        let v = u16::from_be_bytes([s[i], s[i + 1]]);
        sum = sum.wrapping_add(v);
        inv = inv.wrapping_add(!v);
    }
    s[0..2].copy_from_slice(&sum.to_be_bytes());
    s[2..4].copy_from_slice(&inv.to_be_bytes());
    s
}

fn os_init_sram(ctx: &Ctx) {
    let scb = sym("Scb");
    let _ = ctx.dma_write(scb, &default_sram());
    ctx.write_u32(scb + 0x40, 0x40); // offset
    ctx.write_u32(scb + 0x44, 0); // enabled
    ctx.write_u32(scb + 0x48, 0); // locked
    ctx.write_u32(scb + 0x4C, 1); // sync
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_sram_checksums_match_dolphin() {
        let s = super::default_sram();
        assert_eq!(&s[..4], &[0x00, 0x2C, 0xFF, 0xD0]);
    }
}
