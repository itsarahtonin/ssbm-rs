//! `AUDIO_TIMELINE=DIR` (scratch probe, not for commit): what melee-hd's audio schedule check
//! replays and compares. `DIR/log.txt` has, in order: `S tick tb` and `E tick tb` (a tick's
//! game code starts; ends, at HSD_PerfSetCPUTime's entry), `A tb` (an audio frame), `R len`
//! (an ARAM load inside a tick) and `C name r3 r4 r5 r6 r7 r8 r9 r10 f1 -> r3 tb` (an audio
//! library call from game code: outermost and with interrupts enabled, so not the interrupt
//! handlers' own; `HSD_AudioPStreamStartChParam`'s r3 is the stream's entry number). `DIR/dump2.zst` holds, at each tick's end, the tick number
//! (u32 LE), the memory ranges in `RANGES`, and the device
//! command queue's nodes (a count, then each node's address and 0x24 bytes); `DIR/heap.zst`
//! the audio heap at each tick's end that changed it: the heaps' descriptors and the cells'
//! span, each as (tick, start, length, bytes). With `AUDIO_START=K,...`, `DIR/start-K.call` is
//! the whole machine at tick K's end, a saved call of HSD_PerfSetCPUTime.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use ssbm_rt::{Ctx, Native};

/// Static data, the lbaudio banks' state, AX's and the synth's and driver's arrays, and the
/// small data globals.
const RANGES: [(u32, u32); 5] = [
    (0x803B_B300, 0x803B_B3C0),
    (0x8040_7F00, 0x8040_8100),
    (0x8043_3700, 0x8043_3C30),
    (0x804A_8D00, 0x804C_6400),
    (0x804D_6000, 0x804D_7900),
];

/// The audio library's functions game code calls.
const CALLS: &[&str] = &[
    "HSD_AudioSFXStartParam",
    "HSD_AudioSFXKeyOff",
    "HSD_AudioSFXKeyOffAll",
    "HSD_AudioSFXKeyOffTrack",
    "HSD_AudioSFXSetPan",
    "HSD_AudioSFXSetVolumeEx",
    "HSD_AudioSFXSetPitchFid",
    "HSD_AudioSFXSetMix",
    "HSD_AudioSFXSetMixGroup",
    "HSD_AudioSFXCheck",
    "HSD_AudioSFXSetupAux",
    "HSD_AudioPStreamPauseCh",
    "HSD_AudioPStreamResumeCh",
    "HSD_AudioPStreamStartChParam",
    "HSD_AudioInitMultiPStream",
    "HSD_SynthSFXUpdateAllVolume",
    "HSD_SynthStreamSetVolume",
    "HSD_SynthSFXLoad",
    "HSD_SynthSFXUnloadBank",
    "HSD_SynthSFXAllocateBank",
    "HSD_SynthSetSoundMode",
    "HSD_SynthSFXGroupDataRemove",
    "HSD_SynthSFXCancelLoad",
    "HSD_SynthSFXBankDeflag",
    "HSD_SynthSFXBankDeflagSync",
    "AXDriverStop",
    "AXDriverPause",
    "AXDriverResume",
    "AXDriver_8038DA70",
    "AXDriver_8038DCFC",
    "HSD_DevComRequest",
    "HSD_SynthSFXWaitForLoadCompletion",
    "lbAudioAx_80026F2C",
    "lbAudioAx_8002702C",
    "lbAudioAx_80027168",
    "lbAudioAx_80027648",
    "lbAudioAx_8002785C",
];

thread_local! {
    static DIR: RefCell<PathBuf> = RefCell::default();
    static OUT: RefCell<Option<BufWriter<File>>> = const { RefCell::new(None) };
    static DUMP: RefCell<Option<zstd::Encoder<'static, BufWriter<File>>>> = const { RefCell::new(None) };
    static HEAP: RefCell<Option<zstd::Encoder<'static, BufWriter<File>>>> = const { RefCell::new(None) };
    static HEAP_LAST: RefCell<Vec<u8>> = RefCell::default();
    static ORIG: RefCell<HashMap<u32, Native>> = RefCell::default();
    static NAMES: RefCell<HashMap<u32, &'static str>> = RefCell::default();
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static IN_TICK: Cell<bool> = const { Cell::new(false) };
    static TICK: Cell<u32> = const { Cell::new(0) };
    static START: RefCell<Vec<u32>> = RefCell::default();
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

fn orig_at(ctx: &Ctx, at: u32) {
    let f = ORIG.with(|o| o.borrow().get(&at).copied()).expect("wrapped");
    f(ctx);
}

fn wrap(ctx: &Ctx, name: &'static str, f: Native) {
    let at = sym(name);
    let entry = ctx
        .entry(at)
        .unwrap_or_else(|| panic!("AUDIO_TIMELINE needs {name} registered (--port all)"));
    ORIG.with(|o| o.borrow_mut().insert(at, entry.native));
    NAMES.with(|n| n.borrow_mut().insert(at, name));
    ctx.register(at, f);
    ctx.set_mode(at, entry.mode);
}

/// The audio calls' wrapper: which function this is comes from the link register's caller...
/// so each call gets its own trampoline below.
fn call(ctx: &Ctx, at: u32) {
    // lbAudioAx_8002785C's calls are logged too (it inlines its wait, lbAudioAx_80027648,
    // which its line, logged as it returns, stands for).
    let transparent = at == sym("lbAudioAx_8002785C");
    let depth = DEPTH.with(|d| {
        let v = d.get();
        if !transparent {
            d.set(v + 1);
        }
        v
    });
    let args: Vec<String> = (3..=10).map(|i| format!("{:x}", ctx.regs.r(i))).collect();
    let f1 = ctx.regs.f(1).to_bits();
    let enabled = ctx.regs.msr.get() & 0x8000 != 0;
    let tb = ctx.regs.tb.get();
    orig_at(ctx, at);
    DEPTH.with(|d| d.set(depth));
    // Interrupt handlers run with interrupts disabled; game code calls these with them on.
    let name = NAMES.with(|n| n.borrow()[&at]);
    // The game's audio library's functions only game code calls, some with interrupts off.
    if depth == 0 && (enabled || name.starts_with("lbAudioAx_")) {
        let mut args = args;
        if name == "HSD_AudioPStreamStartChParam" {
            // The stream's path, as the entry number the start looked it up as.
            args[0] = format!("{:x}", ctx.read_u32(sym("HSD_Synth_804D7764")));
        }
        log(format!("C {name} {} {f1:x} -> {:x} {tb}", args.join(" "), ctx.regs.r(3)));
    }
}

macro_rules! trampolines {
    ($($i:literal),*) => {
        [$(|ctx: &Ctx| call(ctx, sym(CALLS[$i]))),*]
    };
}

pub fn install(ctx: &Ctx, dir: &str) {
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let open = |n: &str| BufWriter::new(File::create(dir.join(n)).expect("timeline file"));
    OUT.with(|o| *o.borrow_mut() = Some(open("log.txt")));
    DUMP.with(|d| *d.borrow_mut() = Some(zstd::Encoder::new(open("dump2.zst"), 3).unwrap()));
    HEAP.with(|d| *d.borrow_mut() = Some(zstd::Encoder::new(open("heap.zst"), 3).unwrap()));
    DIR.with(|d| *d.borrow_mut() = dir.clone());
    if let Ok(s) = std::env::var("AUDIO_START") {
        START.with(|v| *v.borrow_mut() = s.split(',').map(|x| x.trim().parse().unwrap()).collect());
    }
    wrap(ctx, "HSD_PerfSetStartTime", |ctx| {
        let t = TICK.with(Cell::get);
        if std::env::var("AUDIO_START_BEGIN").is_ok_and(|v| v.split(',').any(|x| x.trim() == t.to_string())) {
            let at = sym("HSD_PerfSetStartTime");
            let call = ssbm_rt::capture::Call::take(ctx, at, &ctx.regs.snapshot());
            let packed = zstd::encode_all(&call.to_bytes()[..], 3).expect("zstd");
            let path = DIR.with(|d| d.borrow().join(format!("begin-{t}.call")));
            std::fs::write(&path, packed).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        log(format!("S {} {}", TICK.with(Cell::get), ctx.regs.tb.get()));
        IN_TICK.with(|c| c.set(true));
        orig_at(ctx, sym("HSD_PerfSetStartTime"));
    });
    wrap(ctx, "HSD_PerfSetCPUTime", |ctx| {
        let t = TICK.with(Cell::get);
        if START.with(|s| s.borrow().contains(&t)) {
            let at = sym("HSD_PerfSetCPUTime");
            let call = ssbm_rt::capture::Call::take(ctx, at, &ctx.regs.snapshot());
            let packed = zstd::encode_all(&call.to_bytes()[..], 3).expect("zstd");
            let path = DIR.with(|d| d.borrow().join(format!("start-{t}.call")));
            std::fs::write(&path, packed).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        DUMP.with(|d| {
            if let Some(d) = d.borrow_mut().as_mut() {
                d.write_all(&t.to_le_bytes()).unwrap();
                for (lo, hi) in RANGES {
                    let mut buf = vec![0u8; (hi - lo) as usize];
                    let _ = ctx.mem.read_bytes(lo, &mut buf);
                    d.write_all(&buf).unwrap();
                }
                // The device command queue's nodes, in the audio heap: every one reached from
                // its roots, as (address, 0x24 bytes).
                let mut nodes = std::collections::BTreeSet::new();
                let mut roots = vec![ctx.read_u32(sym("HSD_DevCom_804D77F0"))];
                for q in 0..4 {
                    roots.push(ctx.read_u32(sym("devComStatus") + 4 * q));
                    roots.push(ctx.read_u32(sym("HSD_DevCom_804C6330") + 4 * q));
                }
                roots.push(ctx.read_u32(sym("dvdDC")));
                roots.push(ctx.read_u32(sym("aramDC")));
                roots.push(ctx.read_u32(sym("HSD_DevCom_804D77FC")));
                roots.push(ctx.read_u32(sym("HSD_DevCom_804D77FC") + 4));
                for mut a in roots {
                    while a != 0 && nodes.insert(a) {
                        a = ctx.read_u32(a);
                    }
                }
                d.write_all(&(nodes.len() as u32).to_le_bytes()).unwrap();
                for a in nodes {
                    let mut buf = vec![0u8; 0x24];
                    let _ = ctx.mem.read_bytes(a, &mut buf);
                    d.write_all(&a.to_le_bytes()).unwrap();
                    d.write_all(&buf).unwrap();
                }
            }
        });
        // The audio heap, where it changed since the last tick that changed it: its cells'
        // span (free and allocated), as (tick, start, length, bytes).
        HEAP.with(|h| {
            if let Some(h) = h.borrow_mut().as_mut() {
                let heap = ctx.read_u32(sym("HSD_Synth_804D6018"));
                let heaps = ctx.read_u32(sym("HeapArray"));
                let hd = heaps + 0xC * heap;
                let (mut lo, mut hi) = (u32::MAX, 0);
                for list in [ctx.read_u32(hd + 4), ctx.read_u32(hd + 8)] {
                    let mut c = list;
                    while c != 0 {
                        lo = lo.min(c);
                        hi = hi.max(c + ctx.read_u32(c + 8));
                        c = ctx.read_u32(c + 4);
                    }
                }
                if lo < hi {
                    // The heaps' descriptors, then the cells' span.
                    let mut key = Vec::new();
                    let mut regions = Vec::new();
                    for (at, len) in [(heaps, 0xC * ctx.read_u32(sym("NumHeaps"))), (lo, hi - lo)] {
                        let mut buf = vec![0u8; len as usize];
                        let _ = ctx.mem.read_bytes(at, &mut buf);
                        key.extend_from_slice(&at.to_le_bytes());
                        key.extend_from_slice(&buf);
                        regions.push((at, buf));
                    }
                    let changed = HEAP_LAST.with(|l| *l.borrow() != key);
                    if changed {
                        for (at, buf) in regions {
                            h.write_all(&t.to_le_bytes()).unwrap();
                            h.write_all(&at.to_le_bytes()).unwrap();
                            h.write_all(&(buf.len() as u32).to_le_bytes()).unwrap();
                            h.write_all(&buf).unwrap();
                        }
                        HEAP_LAST.with(|l| *l.borrow_mut() = key);
                    }
                }
            }
        });
        log(format!("E {t} {}", ctx.regs.tb.get()));
        orig_at(ctx, sym("HSD_PerfSetCPUTime"));
        IN_TICK.with(|c| c.set(false));
        TICK.with(|c| c.set(t + 1));
    });
    wrap(ctx, "AXDriverCallback", |ctx| {
        log(format!("A {}", ctx.regs.tb.get()));
        orig_at(ctx, sym("AXDriverCallback"));
    });
    wrap(ctx, "lbArq_80014BD0", |ctx| {
        if IN_TICK.with(Cell::get) {
            log(format!("R {:x}", ctx.regs.r(5)));
        }
        orig_at(ctx, sym("lbArq_80014BD0"));
    });
    let tramps: [Native; 37] = trampolines!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36
    );
    for (i, name) in CALLS.iter().enumerate() {
        wrap(ctx, name, tramps[i]);
    }
}

pub fn finish() {
    OUT.with(|o| {
        if let Some(mut o) = o.borrow_mut().take() {
            let _ = o.flush();
        }
    });
    for f in [&DUMP, &HEAP] {
        f.with(|d| {
            if let Some(d) = d.borrow_mut().take() {
                let mut w = d.finish().unwrap();
                let _ = w.flush();
            }
        });
    }
}
