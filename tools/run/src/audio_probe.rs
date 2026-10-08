//! `AUDIO_TRACE=FILE` (scratch probe, not for commit): where HAL's AX driver runs its audio
//! frame (`AXDriverCallback`) relative to the engine's ticks, and every driver call the game
//! makes, with the time base and the context each happened in.
//!
//! Lines: `S tick tb` / `E tick tb` (tick start/end: HSD_PerfSetStartTime/HSD_PerfSetCPUTime
//! entry), `A n tb ctx active` (an audio frame; ctx: W in the game's wait loop, Z thread sleep,
//! T inside a tick, P inside a tick's perf timer read, O elsewhere), `C name args -> ret tb ctx`
//! (driver calls), `D tick hash...` (driver, synth and AX allocation state hashes at tick end).

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::rc::Rc;

use ssbm_rt::{Ctx, Native};

thread_local! {
    static OUT: RefCell<Option<BufWriter<File>>> = const { RefCell::new(None) };
    static DUMP: RefCell<Option<BufWriter<File>>> = const { RefCell::new(None) };
    static IN_WAIT: Cell<u32> = const { Cell::new(0) };
    static IN_SLEEP: Cell<u32> = const { Cell::new(0) };
    static IN_TICK: Cell<bool> = const { Cell::new(false) };
    static IN_PERF: Cell<bool> = const { Cell::new(false) };
    static TICK: Cell<u32> = const { Cell::new(0) };
    static ORIG: RefCell<std::collections::HashMap<u32, Native>> = RefCell::default();
}

fn sym(n: &str) -> u32 {
    ssbm_sdk::sym(n)
}

fn log(s: String) {
    OUT.with(|o| {
        if let Some(o) = o.borrow_mut().as_mut() {
            let _ = writeln!(o, "{s}");
        }
    });
}

fn context() -> char {
    if IN_WAIT.with(Cell::get) > 0 {
        'W'
    } else if IN_SLEEP.with(Cell::get) > 0 {
        'Z'
    } else if IN_PERF.with(Cell::get) {
        'P'
    } else if IN_TICK.with(Cell::get) {
        'T'
    } else {
        'O'
    }
}

fn orig(ctx: &Ctx, name: &str) {
    let at = sym(name);
    let f = ORIG.with(|o| o.borrow().get(&at).copied()).expect("wrapped");
    f(ctx);
}

fn wrap(ctx: &Ctx, name: &str, f: Native) {
    let at = sym(name);
    let entry = ctx
        .entry(at)
        .unwrap_or_else(|| panic!("AUDIO_TRACE needs {name} registered (--port all)"));
    ORIG.with(|o| o.borrow_mut().insert(at, entry.native));
    ctx.register(at, f);
    ctx.set_mode(at, entry.mode);
}

fn fnv(ctx: &Ctx, addr: u32, len: u32) -> u64 {
    let mut buf = vec![0u8; len as usize];
    let _ = ctx.mem.read_bytes(addr, &mut buf);
    buf.iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The regions the probe hashes and dumps: the driver's voices, map and globals, the synth's
/// nodes, and AX's voices and stacks.
fn regions() -> [(u32, u32); 7] {
    [
        (sym("AXDriver_804C45A0"), 0x1380),
        (sym("AXDriver_804C45A0") + 0x1380, 0x100),
        (sym("vidhigh"), 0x68),
        (sym("hsd_SynthSFXNodes"), 0x1400),
        (sym("__AXVPB"), 0x7E00),
        (sym("__AXStackHead"), 0x100),
        (sym("__AXCallbackStack"), 4),
    ]
}

pub fn install(ctx: &Ctx, path: &str) {
    let file = BufWriter::new(File::create(path).unwrap_or_else(|e| panic!("{path}: {e}")));
    OUT.with(|o| *o.borrow_mut() = Some(file));
    if let Ok(dump) = std::env::var("AUDIO_DUMP") {
        let f = BufWriter::new(File::create(&dump).unwrap_or_else(|e| panic!("{dump}: {e}")));
        DUMP.with(|d| *d.borrow_mut() = Some(f));
    }
    // The game's wait loop: whatever hook the SDK set there, inside our marker.
    let wait = sym("lb_800195D0");
    let (old, _) = ctx.hook(wait).expect("the SDK's wait hook");
    ctx.set_hook(
        wait,
        Rc::new(move |ctx| {
            IN_WAIT.with(|w| w.set(w.get() + 1));
            let tb = ctx.regs.tb.get();
            old(ctx);
            if std::env::var_os("AUDIO_WAITS").is_some() {
                log(format!("w {tb} {} {}", ctx.regs.tb.get(), TICK.with(Cell::get)));
            }
            IN_WAIT.with(|w| w.set(w.get() - 1));
        }),
    );
    wrap(ctx, "OSSleepThread", |ctx| {
        IN_SLEEP.with(|w| w.set(w.get() + 1));
        orig(ctx, "OSSleepThread");
        IN_SLEEP.with(|w| w.set(w.get() - 1));
    });
    wrap(ctx, "HSD_PerfSetStartTime", |ctx| {
        let t = TICK.with(Cell::get);
        let alarm = sym("lb_804329F0") + 0x50;
        let off = ctx.read_u64(0x8000_30D8);
        log(format!(
            "S {t} {} alarm {} period {} axstart {} vi {:08x}",
            ctx.regs.tb.get(),
            ctx.read_u64(alarm + 8).wrapping_sub(off) as i64,
            ctx.read_u64(alarm + 0x18),
            ctx.read_u64(sym("__AXLocalProfile")).wrapping_sub(off) as i64,
            ctx.read_u32(sym("HSD_VIData")),
        ));
        IN_TICK.with(|c| c.set(true));
        IN_PERF.with(|c| c.set(true));
        orig(ctx, "HSD_PerfSetStartTime");
        IN_PERF.with(|c| c.set(false));
    });
    wrap(ctx, "HSD_PerfSetCPUTime", |ctx| {
        let t = TICK.with(Cell::get);
        log(format!("E {t} {}", ctx.regs.tb.get()));
        IN_PERF.with(|c| c.set(true));
        orig(ctx, "HSD_PerfSetCPUTime");
        IN_PERF.with(|c| c.set(false));
        IN_TICK.with(|c| c.set(false));
        let hashes: Vec<String> =
            regions().iter().map(|&(a, l)| format!("{:016x}", fnv(ctx, a, l))).collect();
        log(format!("D {t} {}", hashes.join(" ")));
        DUMP.with(|d| {
            if let Some(d) = d.borrow_mut().as_mut() {
                let _ = d.write_all(&t.to_le_bytes());
                for (a, l) in regions() {
                    let mut buf = vec![0u8; l as usize];
                    let _ = ctx.mem.read_bytes(a, &mut buf);
                    let _ = d.write_all(&buf);
                }
            }
        });
        TICK.with(|c| c.set(t + 1));
    });
    wrap(ctx, "AXDriverCallback", |ctx| {
        let n = ctx.read_u32(sym("AXDriver_804D778C"));
        log(format!(
            "A {n} {} {} {} {}",
            ctx.regs.tb.get(),
            context(),
            ctx.read_u32(sym("AXDriver_804D77D0")),
            TICK.with(Cell::get)
        ));
        orig(ctx, "AXDriverCallback");
    });
    macro_rules! call {
        ($name:literal, $n:expr, $float:expr) => {
            wrap(ctx, $name, |ctx| {
                let args: Vec<String> =
                    (3..3 + $n).map(|i| format!("{:x}", ctx.regs.r(i))).collect();
                let tb = ctx.regs.tb.get();
                let c = context();
                let f1 = ctx.regs.f(1);
                orig(ctx, $name);
                let extra = if $float { format!(" f1={f1}") } else { String::new() };
                log(format!(
                    "C {} {}{} -> {:x} {tb} {c} {}",
                    $name,
                    args.join(" "),
                    extra,
                    ctx.regs.r(3),
                    TICK.with(Cell::get)
                ));
            });
        };
    }
    for name in ["OSGetTime", "OSGetTick"] {
        let f: Native = if name == "OSGetTime" {
            |ctx| {
                if IN_TICK.with(Cell::get) && !IN_PERF.with(Cell::get) {
                    log(format!("R OSGetTime lr {:08x} tb {} {}", ctx.regs.lr.get(), ctx.regs.tb.get(), TICK.with(Cell::get)));
                }
                orig(ctx, "OSGetTime");
            }
        } else {
            |ctx| {
                if IN_TICK.with(Cell::get) && !IN_PERF.with(Cell::get) {
                    log(format!("R OSGetTick lr {:08x} tb {} {}", ctx.regs.lr.get(), ctx.regs.tb.get(), TICK.with(Cell::get)));
                }
                orig(ctx, "OSGetTick");
            }
        };
        wrap(ctx, name, f);
    }
    macro_rules! span {
        ($name:literal) => {
            wrap(ctx, $name, |ctx| {
                let tb = ctx.regs.tb.get();
                orig(ctx, $name);
                let d = ctx.regs.tb.get() - tb;
                if d != 0 && IN_TICK.with(Cell::get) {
                    log(format!("P {} {d} {}", $name, TICK.with(Cell::get)));
                }
            });
        };
    }
    macro_rules! mark {
        ($name:literal, $tag:literal) => {
            wrap(ctx, $name, |ctx| {
                let tb = ctx.regs.tb.get();
                orig(ctx, $name);
                log(format!("{} {tb} {} {}", $tag, ctx.regs.tb.get(), TICK.with(Cell::get)));
            });
        };
    }
    mark!("fn_800195FC", "Q");
    mark!("__VIRetraceHandler", "V");
    mark!("__AXOutAiCallback", "I");
    mark!("__AXDSPResumeCallback", "M");
    mark!("VIWaitForRetrace", "Y");
    mark!("HSD_VIWaitXFBFlush", "X");
    wrap(ctx, "lb_80019894", |ctx| {
        orig(ctx, "lb_80019894");
        if std::env::var_os("AUDIO_WAITS").is_some() {
            log(format!("q {} {} {}", ctx.regs.r(3), ctx.regs.tb.get(), TICK.with(Cell::get)));
        }
    });
    mark!("__DSPHandler", "h");
    mark!("DVDLowRead", "r");
    mark!("__ARHandler", "a");
    mark!("ARQPostRequest", "q");
    wrap(ctx, "HSD_DevComRequest", |ctx| {
        let tb = ctx.regs.tb.get();
        let r = |i: usize| ctx.regs.r(i);
        let args = format!(
            "{:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x} lr {:08x}",
            r(3), r(4), r(5), r(6), r(7), r(8), r(9), r(10), ctx.regs.lr.get()
        );
        orig(ctx, "HSD_DevComRequest");
        log(format!("o {tb} {} {} {args}", ctx.regs.tb.get(), TICK.with(Cell::get)));
    });
    mark!("lbFile_800161C4", "L1");
    mark!("lbFile_8001668C", "L2");
    mark!("lbFile_80016760", "L3");
    mark!("lbFile_800168A0", "L4");
    mark!("HSD_DevComDVDCallback", "b");
    mark!("HSD_SynthPStreamMasterClockCallback", "m");
    mark!("__DVDInterruptHandler", "d");
    mark!("GXCPInterruptHandler", "c");
    mark!("GXTokenInterruptHandler", "k");
    mark!("GXFinishInterruptHandler", "f");
    mark!("EXIIntrruptHandler", "x");
    mark!("TCIntrruptHandler", "t");
    wrap(ctx, "lbArq_80014BD0", |ctx| {
        let tb = ctx.regs.tb.get();
        let len = ctx.regs.r(5);
        orig(ctx, "lbArq_80014BD0");
        let d = ctx.regs.tb.get() - tb;
        if IN_TICK.with(Cell::get) {
            log(format!("P lbArq_80014BD0 {d} {} len {len:x}", TICK.with(Cell::get)));
        }
    });
    span!("lb_800198E0");
    span!("lbAudioAx_80027DF8");
    span!("HSD_GObj_RunProcs");
    span!("lb_80019900");
    span!("gm_EvaluateAllControllerInputs");
    call!("HSD_AudioSFXStartParam", 5, false);
    call!("HSD_AudioSFXKeyOff", 1, false);
    call!("HSD_AudioSFXKeyOffAll", 0, false);
    call!("HSD_AudioSFXKeyOffTrack", 1, false);
    call!("HSD_AudioSFXSetPan", 2, false);
    call!("HSD_AudioSFXSetVolumeEx", 2, false);
    call!("HSD_AudioSFXSetPitchFid", 2, false);
    call!("HSD_AudioSFXSetMix", 3, false);
    call!("HSD_AudioSFXSetMixGroup", 3, false);
    call!("HSD_AudioSFXCheck", 1, false);
    call!("AXDriverKillCallback", 1, false);
    call!("dropcallback", 1, false);
    call!("HSD_SynthSFXPlayWithGroup", 7, false);
}

pub fn finish() {
    OUT.with(|o| {
        if let Some(mut o) = o.borrow_mut().take() {
            let _ = o.flush();
        }
    });
    DUMP.with(|o| {
        if let Some(mut o) = o.borrow_mut().take() {
            let _ = o.flush();
        }
    });
}
