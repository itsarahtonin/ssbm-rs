// SPDX-License-Identifier: GPL-3.0-or-later

//! Boots the game headless on original code and reports progress.
//!
//! `ssbm-run [disc] [--fields N] [--replay FILE] [--fp hardware|slippi]` runs for N video fields
//! (default 600, ten seconds), playing back a Slippi replay if given. Floating point follows
//! Slippi's Dolphin for replays and the hardware otherwise, unless `--fp` says. The disc path
//! defaults to `SSBM_DISC`.
//!
//! A replay run checks the replay it records against the original, and fails on any divergence
//! not listed in the `--known FILE` (one per line, as reported; `#` starts a comment).
//! `--write-known FILE` writes this run's divergences in that form.
//!
//! `--port UNITS` runs the Rust ports of those decomp units (comma-separated unit names or
//! prefixes such as `melee/ft`, or `all`) instead of their original code. With `--lockstep`,
//! every call to a ported function runs the original too and compares, and the run fails on
//! any mismatch; `--lockstep-from FIELD` starts checking at that video field.

use std::cell::Cell;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::process::ExitCode;
use std::rc::Rc;

use ssbm_disc::Disc;
use ssbm_ppc::Interpreter;
use ssbm_rt::Ctx;
use ssbm_sdk::{Sdk, boot, hw};

/// Panic payload that ends the run on purpose.
struct Stop;

/// Ports lockstep does not check (see where they are set).
const LOCKSTEP_EXEMPT: &[&str] = &[
    "main",
    "runGameMode",
    "gm_801A4510",
    "gm_801A4014",
    "gm_801A4D34",
    "ARInit",
    "__ARChecksize",
];

/// Stack for the thread that runs the game. Every guest call nests Rust frames, and ports and
/// lockstep checks make them deep.
const STACK_SIZE: usize = 1 << 30;

fn main() -> ExitCode {
    std::thread::Builder::new()
        .name("game".to_owned())
        .stack_size(STACK_SIZE)
        .spawn(run)
        .expect("start the game thread")
        .join()
        .unwrap_or(ExitCode::FAILURE)
}

fn run() -> ExitCode {
    let mut disc_path = std::env::var("SSBM_DISC").ok();
    let mut fields = 600;
    let mut replay_path = None;
    let mut fp_mode = None;
    let mut known_path = None;
    let mut write_known = None;
    let mut ports: Vec<String> = Vec::new();
    let mut lockstep = false;
    let mut lockstep_from = 0u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--fields" => {
                fields = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--fields N")
            }
            "--replay" => replay_path = Some(args.next().expect("--replay FILE")),
            "--known" => known_path = Some(args.next().expect("--known FILE")),
            "--port" => ports.extend(
                args.next()
                    .expect("--port UNITS")
                    .split(',')
                    .map(|s| s.trim().to_owned()),
            ),
            "--lockstep" => lockstep = true,
            "--lockstep-from" => {
                lockstep = true;
                lockstep_from = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--lockstep-from FIELD");
            }
            "--write-known" => write_known = Some(args.next().expect("--write-known FILE")),
            "--fp" => {
                fp_mode = match args.next().as_deref() {
                    Some("hardware") => Some(gekko_fp::FpMode::Hardware),
                    Some("slippi") => Some(gekko_fp::FpMode::Slippi),
                    _ => panic!("--fp hardware|slippi"),
                }
            }
            _ => disc_path = Some(a),
        }
    }
    let Some(disc_path) = disc_path else {
        eprintln!(
            "usage: ssbm-run [disc] [--fields N] [--replay FILE] [--fp hardware|slippi] [--known FILE] [--write-known FILE]  (or set SSBM_DISC)"
        );
        return ExitCode::FAILURE;
    };
    let disc = match Disc::open(&disc_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{disc_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Lockstep catches and reports the panics of the checks it runs.
        if !info.payload().is::<Stop>() && !ssbm_rt::lockstep::checking() {
            default_hook(info);
        }
    }));

    let ctx = Ctx::new();
    let interp = Rc::new(Interpreter::default());
    ctx.set_backend(Box::new(RcBackend(interp.clone())));
    ctx.set_names(Box::new(ssbm_types::describe));
    let sdk = ssbm_sdk::install(&ctx, disc);
    let slippi = replay_path.map(|path| {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let replay = ssbm_slippi::Replay::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!(
            "replay {path}: version {:?}, {} frames",
            replay.version,
            replay.frames.len()
        );
        // Replays carry the floating point of the Dolphin that recorded them.
        gekko_fp::set_fp_mode(gekko_fp::FpMode::Slippi);
        ssbm_slippi::install(&ctx, replay)
    });
    if let Some(mode) = fp_mode {
        gekko_fp::set_fp_mode(mode);
    }
    let slippi: Option<Rc<ssbm_slippi::Device>> = slippi;

    // Ported units replace their original code.
    let before: std::collections::BTreeSet<u32> = ctx.registered().into_iter().collect();
    let units = ssbm_game::register(&ctx, |unit| {
        ports.iter().any(|p| {
            p == "all" || unit == p || unit.starts_with(&format!("{}/", p.trim_end_matches('/')))
        })
    });
    let ported: Vec<u32> = ctx
        .registered()
        .into_iter()
        .filter(|a| !before.contains(a))
        .collect();
    if !ports.is_empty() {
        eprintln!("ported: {units} units, {} functions", ported.len());
    }
    // Playback's Gecko codes patch some functions; those keep running their patched code.
    let mut kept = Vec::new();
    if let Some(dev) = &slippi {
        for (at, len) in ssbm_slippi::patched(dev) {
            for f in ssbm_types::functions_overlapping(at, len) {
                if ported.contains(&f) && !kept.contains(&f) {
                    ctx.set_mode(f, ssbm_rt::Mode::Original);
                    kept.push(f);
                }
            }
        }
        if !kept.is_empty() {
            let names: Vec<String> = kept.iter().map(|&f| ctx.name_of(f)).collect();
            eprintln!(
                "patched by playback's codes, so kept original: {}",
                names.join(", ")
            );
        }
    }
    if lockstep {
        // The game's outer loops never return, so they run as ports while everything they
        // call is checked. ARInit DMAs through a stack buffer whose address the port's frame
        // does not reproduce yet.
        let exempt: Vec<u32> = LOCKSTEP_EXEMPT.iter().map(|n| ssbm_sdk::sym(n)).collect();
        let checked: Vec<u32> = ported
            .iter()
            .copied()
            .filter(|a| !kept.contains(a) && !exempt.contains(a))
            .collect();
        let enable = move |ctx: &Ctx| {
            for &addr in &checked {
                ctx.set_mode(addr, ssbm_rt::Mode::Lockstep);
            }
        };
        if lockstep_from == 0 {
            enable(&ctx);
        } else {
            sdk.schedule(hw::field_start(lockstep_from), enable);
        }
        ctx.lockstep.keep_per_function.set(2);
    }

    // CALLS=name,... logs each call to these functions (symbols or hex addresses) with its
    // first four arguments.
    if let Ok(names) = std::env::var("CALLS") {
        for name in names.split(',').map(str::trim) {
            let dev = slippi.clone();
            let addr = u32::from_str_radix(name.trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| ssbm_sdk::sym(name));
            let name = name.to_owned();
            ctx.set_hook(
                addr,
                Rc::new(move |ctx| {
                    let when = match &dev {
                        Some(dev) => format!(
                            "frame {}",
                            dev.frames_read.get() as i32 + ssbm_slippi::replay::GAME_FIRST_FRAME
                                - 1
                        ),
                        None => format!("field {}", ctx.ext::<Sdk>().hw.fields.get()),
                    };
                    let r = &ctx.regs;
                    eprintln!(
                        "{when}: {name}({:08X}, {:08X}, {:08X}, {:08X}) from {}",
                        r.r(3),
                        r.r(4),
                        r.r(5),
                        r.r(6),
                        ctx.name_of(r.lr.get())
                    );
                }),
            );
        }
    }

    // WATCH_ADDR=addr[,len] logs every write to that memory, with the writer and the ports
    // running, and whether a lockstep check's original side made it.
    if let Ok(spec) = std::env::var("WATCH_ADDR") {
        let mut parts = spec.split(',');
        let addr = u32::from_str_radix(parts.next().unwrap().trim_start_matches("0x"), 16)
            .expect("WATCH_ADDR=addr[,len]");
        let len = parts.next().map_or(4, |l| l.parse().unwrap());
        let interp4 = interp.clone();
        ctx.set_watch(
            addr,
            len,
            Rc::new(move |ctx, at, n| {
                let ports: Vec<String> = ctx
                    .native_stack()
                    .iter()
                    .rev()
                    .take(3)
                    .map(|a| ctx.name_of(*a))
                    .collect();
                eprintln!(
                    "write {:0w$X} to {at:08X} at {} (ports: {}){}",
                    ctx.read_be(at, n),
                    ctx.name_of(interp4.pc.get()),
                    ports.join(" < "),
                    if ctx.lockstep.in_original() {
                        " [original side]"
                    } else {
                        ""
                    },
                    w = 2 * n as usize
                );
            }),
        );
    }

    // WATCH=port,offset,from,to logs writes to a fighter field during replay frames
    // [from, to], with the writing instruction and the jumps that led there.
    if let (Some(spec), Some(dev)) = (std::env::var("WATCH").ok(), slippi.clone()) {
        let v: Vec<i64> = spec
            .split(',')
            .map(|x| {
                let x = x.trim();
                x.strip_prefix("0x").map_or_else(
                    || x.parse().unwrap(),
                    |h| i64::from_str_radix(h, 16).unwrap(),
                )
            })
            .collect();
        let (port, offset, from, to) = (v[0] as u32, v[1] as u32, v[2] as i32, v[3] as i32);
        let interp3 = interp.clone();
        let dev2 = dev.clone();
        ctx.set_watch(
            0,
            0,
            Rc::new(move |ctx, addr, len| {
                let frame =
                    dev2.frames_read.get() as i32 + ssbm_slippi::replay::GAME_FIRST_FRAME - 1;
                if (from..=to).contains(&frame) {
                    let value = ctx.read_be(addr, len);
                    eprintln!(
                        "frame {frame}: write {value:0w$X} to {addr:08X} at {}",
                        ctx.name_of(interp3.pc.get()),
                        w = 2 * len as usize
                    );
                    let f: Vec<String> = (0..4)
                        .map(|i| format!("f{i}={:016X}", ctx.regs.f(i).to_bits()))
                        .collect();
                    eprintln!("    {}", f.join(" "));
                    for (a, b) in interp3.recent_jumps().iter().rev().take(4) {
                        eprintln!("    after {} -> {}", ctx.name_of(*a), ctx.name_of(*b));
                    }
                }
            }),
        );
        // Follow the fighter's data as it is created.
        let slots = ssbm_sdk::sym("player_slots");
        ctx.set_hook(
            ssbm_sdk::sym("Fighter_procInput"),
            Rc::new(move |ctx| {
                let gobj = ctx.read_u32(slots + 0xE90 * port + 0xB0);
                if gobj != 0 {
                    ctx.move_watch(ctx.read_u32(gobj + 0x2C) + offset, 4);
                }
            }),
        );
    }

    // A replay run also stops once the game ends, a second after the recording codes send it.
    if let Some(dev) = slippi.clone() {
        fn check(dev: Rc<ssbm_slippi::Device>, field: u64) -> impl FnOnce(&Ctx) + 'static {
            move |ctx| {
                if dev.ended.get() || dev.terminated.get() {
                    let sdk = ctx.ext::<Sdk>();
                    sdk.after(ctx, ssbm_sdk::TB_HZ, |_| panic::panic_any(Stop));
                } else {
                    let sdk = ctx.ext::<Sdk>();
                    sdk.schedule(hw::field_start(field + 60), check(dev, field + 60));
                }
            }
        }
        sdk.schedule(hw::field_start(60), check(dev, 60));
    }

    // Stop after the requested number of fields.
    sdk.schedule(hw::field_start(fields), |_| panic::panic_any(Stop));

    // Every heartbeat, report progress; if no field passed in a while, show where it spins.
    let last_fields = Cell::new(0u64);
    let stuck = Cell::new(0u32);
    let pcs: std::cell::RefCell<HashMap<u32, u32>> = Default::default();
    let interp2 = interp.clone();
    ctx.set_heartbeat(move |ctx, pc| {
        let sdk = ctx.ext::<Sdk>();
        let f = sdk.hw.fields.get();
        if f == last_fields.get() {
            stuck.set(stuck.get() + 1);
            *pcs.borrow_mut().entry(pc).or_default() += 1;
            if stuck.get() >= 8 {
                let mut hot: Vec<_> = pcs.borrow().iter().map(|(&pc, &n)| (n, pc)).collect();
                hot.sort_unstable_by(|a, b| b.cmp(a));
                for (n, pc) in hot.iter().take(8) {
                    eprintln!("  {n:4} x {}", ctx.name_of(*pc));
                }
                panic!(
                    "stuck: no video field for {} instructions",
                    8 * ssbm_rt::HEARTBEAT
                );
            }
        } else {
            stuck.set(0);
            pcs.borrow_mut().clear();
            last_fields.set(f);
        }
        eprintln!(
            "field {f:6}  {:>6} M instructions  {} draws  at {}",
            interp2.executed.get() / 1_000_000,
            sdk.hw.draws(),
            ctx.name_of(pc)
        );
    });

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let entry = boot::boot(&ctx, boot::DEFAULT_CLOCK);
        // FILL_ARENA=byte fills the heap arena with that byte as main starts, to show whether a
        // divergence depends on memory the game never initializes.
        if let Ok(byte) = std::env::var("FILL_ARENA") {
            let byte = u8::from_str_radix(&byte, 16).unwrap();
            ctx.set_hook(
                ssbm_sdk::sym("main"),
                Rc::new(move |ctx| {
                    let lo = ctx.read_u32(ssbm_sdk::sym("__OSArenaLo"));
                    let hi = ctx.read_u32(ssbm_sdk::sym("__OSArenaHi"));
                    eprintln!("filling arena {lo:08X}..{hi:08X} with {byte:02X}");
                    ctx.fill(lo, byte, hi - lo);
                }),
            );
        }
        if slippi.is_some() {
            ssbm_slippi::apply_bootloader(&ctx);
        }
        eprintln!("booting {} at {entry:#010X}", disc_path);
        ctx.invoke(entry);
    }));
    let executed = interp.executed.get();
    let mut replay_ok = true;
    if let Some(dev) = &slippi {
        for line in dev.log.borrow().iter() {
            eprintln!("slippi: {line}");
        }
        eprintln!(
            "slippi: {} frames read, {} bytes recorded, terminated {}",
            dev.frames_read.get(),
            dev.recorded.borrow().len(),
            dev.terminated.get()
        );
        let report = ssbm_slippi::compare::compare(&dev.replay.events, &dev.recorded.borrow());
        let known: Vec<String> = known_path
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect();
        let lines: Vec<String> = report.divergences.iter().map(|d| d.to_string()).collect();
        let unexpected: Vec<_> = report
            .divergences
            .iter()
            .zip(&lines)
            .filter(|(_, l)| !known.contains(l))
            .map(|(d, _)| d)
            .collect();
        let gone = known.iter().filter(|k| !lines.contains(k)).count();
        replay_ok = unexpected.is_empty() && report.missing == 0;
        eprintln!(
            "replay check: {} events compared through frame {:?}, {} not reached, {} diverge ({} known), {} known no longer diverge",
            report.compared,
            report.last_frame_compared,
            report.missing,
            report.divergences.len(),
            report.divergences.len() - unexpected.len(),
            gone
        );
        if let Some(path) = &write_known {
            let mut text = lines.join("\n");
            text.push('\n');
            std::fs::write(path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
        }
        let mut kinds: Vec<(&str, &str, usize, i32)> = Vec::new();
        for d in &unexpected {
            match kinds.iter_mut().find(|k| k.0 == d.event && k.1 == d.field) {
                Some(k) => k.2 += 1,
                None => kinds.push((d.event, d.field, 1, d.frame)),
            }
        }
        for (event, field, n, first) in &kinds {
            eprintln!("  {n:6} x {event} {field}, first on frame {first}");
        }
        for d in unexpected.iter().take(6) {
            eprintln!("  {d}");
        }
        for (event, frame, port) in &report.first_missing {
            eprintln!("  not reached: frame {frame}: {event} port {}", port + 1);
        }
    }
    let mut lockstep_ok = true;
    if lockstep {
        let stats = ctx.lockstep.stats.borrow();
        let calls: u64 = stats.values().map(|s| s.calls).sum();
        let bad: Vec<_> = stats.iter().filter(|(_, s)| s.mismatches > 0).collect();
        let unverifiable: Vec<_> = stats.iter().filter(|(_, s)| s.uninitialized > 0).collect();
        eprintln!(
            "lockstep: {calls} calls to {} of {} ported functions, {} functions mismatch",
            stats.len(),
            ported.len(),
            bad.len()
        );
        if !unverifiable.is_empty() {
            let names: Vec<String> = unverifiable
                .iter()
                .map(|(a, s)| format!("{} ({})", ctx.name_of(**a), s.uninitialized))
                .collect();
            eprintln!(
                "  calls whose original reads uninitialized stack, not compared: {}",
                names.join(", ")
            );
        }
        for (addr, s) in &bad {
            eprintln!(
                "  {}: {} of {} calls mismatch",
                ctx.name_of(**addr),
                s.mismatches,
                s.calls
            );
        }
        for m in ctx.lockstep.mismatches.borrow().iter() {
            eprintln!("  {} call {}:", ctx.name_of(m.function), m.call);
            for d in m.diffs.iter().take(6) {
                eprintln!("    {d:X?}");
            }
        }
        lockstep_ok = bad.is_empty();
    }
    eprintln!(
        "{} fields, {} M instructions, {} draws",
        sdk.hw.fields.get(),
        executed / 1_000_000,
        sdk.hw.draws()
    );
    match result {
        Ok(()) => {
            eprintln!("the game returned from its entry point");
            ExitCode::FAILURE
        }
        Err(p) if p.is::<Stop>() => {
            if replay_ok && lockstep_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .or_else(|| p.downcast_ref::<ssbm_rt::Fault>().map(|f| f.to_string()))
                .unwrap_or_else(|| "panic".to_owned());
            eprintln!("stopped: {msg}");
            eprintln!("  {}", sdk.describe(&ctx));
            for (from, to) in interp.recent_jumps() {
                eprintln!("  jump {} -> {}", ctx.name_of(from), ctx.name_of(to));
            }
            for addr in ctx.native_stack().iter().rev() {
                eprintln!("  in port {}", ctx.name_of(*addr));
            }
            eprintln!("  pc {}", ctx.name_of(interp.pc.get()));
            eprintln!("  lr {}", ctx.name_of(ctx.regs.lr.get()));
            // Walk the guest stack's back chain.
            let mut sp = ctx.regs.r(1);
            for _ in 0..16 {
                let Ok(next) = ctx.mem.read_u32(sp) else {
                    break;
                };
                let Ok(lr) = ctx.mem.read_u32(next.wrapping_add(4)) else {
                    break;
                };
                if next == 0 || next <= sp {
                    break;
                }
                eprintln!("  from {}", ctx.name_of(lr));
                sp = next;
            }
            ExitCode::FAILURE
        }
    }
}

/// Lets the runner keep a handle on the interpreter for its counters.
struct RcBackend(Rc<Interpreter>);

impl ssbm_rt::Backend for RcBackend {
    fn run(&self, ctx: &Ctx, addr: u32) {
        self.0.run(ctx, addr)
    }
}
