//! `DVD_TIMELINE=FILE` (with `--port all`): when the game's disc reads are asked for, start,
//! end and call back, for melee-hd's timing comparisons. One line per event, appended to FILE,
//! each starting with the time base (`tb`, from power-on) and the video field:
//!
//! - `read`: a `DVDReadAsyncPrio` call, at its entry: the file (its start on the disc and its
//!   length, from the `DVDFileInfo`; the info has no entry number), the offset and length read,
//!   the destination, the callback, the priority, `ints` (on in game code, off in an interrupt
//!   handler) and the caller's return address.
//! - `di-start` / `di-done`: a drive command, as the DI control register write starts it and as
//!   the drive finishes it, raising the transfer-complete interrupt; with its command words.
//! - `di-handler`: `__DVDInterruptHandler`'s entry.
//! - `callback NAME`: `HSD_DevComDVDCallback`'s or `HSD_DevComDVDMemCallback`'s entry.
//! - `arq-post`: `ARQPostRequest`, at its entry (`tb`) and its return (`ret`).
//! - `OSGetTime` / `OSGetTick` called from `fn_8001E910` or `fn_8001F13C`: the value returned.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};

use ssbm_rt::{Ctx, Native};
use ssbm_sdk::{Sdk, hw::DvdEvent};

thread_local! {
    static OUT: RefCell<Option<BufWriter<std::fs::File>>> = const { RefCell::new(None) };
    static ORIG: RefCell<HashMap<u32, Native>> = RefCell::default();
}

/// The movie's functions whose clock reads are logged.
const CLOCK_CALLERS: [&str; 2] = ["fn_8001E910", "fn_8001F13C"];

fn sym(name: &str) -> u32 {
    ssbm_sdk::sym(name)
}

fn log(ctx: &Ctx, line: String) {
    let field = ctx.ext::<Sdk>().hw.fields.get();
    OUT.with(|o| {
        if let Some(o) = o.borrow_mut().as_mut() {
            let _ = writeln!(o, "tb {} field {field} {line}", ctx.regs.tb.get());
        }
    });
}

fn orig(ctx: &Ctx, name: &str) {
    let f = ORIG.with(|o| o.borrow().get(&sym(name)).copied()).expect("wrapped");
    f(ctx);
}

fn wrap(ctx: &Ctx, name: &str, f: Native) {
    let at = sym(name);
    let entry = ctx
        .entry(at)
        .unwrap_or_else(|| panic!("DVD_TIMELINE needs {name} registered (--port all)"));
    ORIG.with(|o| o.borrow_mut().insert(at, entry.native));
    ctx.register(at, f);
    ctx.set_mode(at, entry.mode);
}

/// Which of `CLOCK_CALLERS` called the function running now, if one did: by the return
/// address, or by the port that called it.
fn clock_caller(ctx: &Ctx) -> Option<&'static str> {
    let stack = ctx.native_stack();
    let caller = stack.len().checked_sub(2).map(|i| stack[i]);
    CLOCK_CALLERS.into_iter().find(|name| {
        let start = sym(name);
        let (lo, hi) = ssbm_types::function_bounds(start).unwrap_or((start, start + 4));
        caller == Some(start) || (lo..hi).contains(&ctx.regs.lr.get())
    })
}

pub fn install(ctx: &Ctx, path: &str) {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    OUT.with(|o| *o.borrow_mut() = Some(BufWriter::new(file)));
    *ctx.ext::<Sdk>().hw.dvd_observer.borrow_mut() = Some(Box::new(|ctx, event, cmd| {
        let name = match event {
            DvdEvent::Start => "di-start",
            DvdEvent::Done => "di-done",
        };
        log(ctx, format!("{name} cmd {:08x} {:08x} {:08x}", cmd[0], cmd[1], cmd[2]));
    }));
    wrap(ctx, "DVDReadAsyncPrio", |ctx| {
        let r = |i| ctx.regs.r(i);
        let info = r(3);
        let ints = if ctx.regs.msr.get() & ssbm_rt::MSR_EE != 0 { "on" } else { "off" };
        log(
            ctx,
            format!(
                "read file {:08x} len {:x} offset {:x} length {:x} dest {:08x} callback {:08x} \
                 prio {} ints {ints} lr {:08x}",
                ctx.read_u32(info + 0x30),
                ctx.read_u32(info + 0x34),
                r(6),
                r(5),
                r(4),
                r(7),
                r(8) as i32,
                ctx.regs.lr.get(),
            ),
        );
        orig(ctx, "DVDReadAsyncPrio");
    });
    wrap(ctx, "__DVDInterruptHandler", |ctx| {
        log(ctx, "di-handler".into());
        orig(ctx, "__DVDInterruptHandler");
    });
    wrap(ctx, "HSD_DevComDVDCallback", |ctx| {
        log(ctx, "callback HSD_DevComDVDCallback".into());
        orig(ctx, "HSD_DevComDVDCallback");
    });
    wrap(ctx, "HSD_DevComDVDMemCallback", |ctx| {
        log(ctx, "callback HSD_DevComDVDMemCallback".into());
        orig(ctx, "HSD_DevComDVDMemCallback");
    });
    wrap(ctx, "ARQPostRequest", |ctx| {
        let tb = ctx.regs.tb.get();
        let lr = ctx.regs.lr.get();
        orig(ctx, "ARQPostRequest");
        log(ctx, format!("arq-post entry {tb} ret {} lr {lr:08x}", ctx.regs.tb.get()));
    });
    wrap(ctx, "OSGetTime", |ctx| {
        let caller = clock_caller(ctx);
        orig(ctx, "OSGetTime");
        if let Some(caller) = caller {
            let v = (u64::from(ctx.regs.r(3)) << 32) | u64::from(ctx.regs.r(4));
            log(ctx, format!("OSGetTime from {caller} -> {v}"));
        }
    });
    wrap(ctx, "OSGetTick", |ctx| {
        let caller = clock_caller(ctx);
        orig(ctx, "OSGetTick");
        if let Some(caller) = caller {
            log(ctx, format!("OSGetTick from {caller} -> {}", ctx.regs.r(3)));
        }
    });
}

pub fn finish() {
    OUT.with(|o| {
        if let Some(o) = o.borrow_mut().as_mut() {
            let _ = o.flush();
        }
    });
}
