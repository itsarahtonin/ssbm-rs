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
//! prefixes such as `melee/ft`, or `all`, and `-UNIT` to leave one out) instead of their
//! original code. With `--lockstep`, every call to a ported function runs the original too and
//! compares, and the run fails on any mismatch; `--lockstep-from FIELD` starts checking at that
//! video field.
//!
//! For debugging, `LOCKSTEP_TRACE` shows the jumps original code made before a panic inside a
//! lockstep check, and `STUCK_TRACE` the ports running at each heartbeat of a stall.

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
    "__start",
    "main",
    "runGameMode",
    "gm_801A4510",
    "gm_801A4014",
    "gm_801A4D34",
    "ARInit",
    "__ARChecksize",
    "__setjmp",
    "__longjmp",
];

thread_local! {
    /// The interpreter, for the jumps LOCKSTEP_TRACE shows.
    static INTERP: std::cell::RefCell<Option<Rc<Interpreter>>> = const { std::cell::RefCell::new(None) };
}

/// Addresses of the `bl` instructions in the executable's code that call `target`.
fn calls_to(dol: &ssbm_disc::Dol, target: u32) -> Vec<u32> {
    let mut sites = Vec::new();
    for section in dol.sections.iter().filter(|s| s.kind == ssbm_disc::SectionKind::Text) {
        for (i, w) in dol.section_data(section).chunks_exact(4).enumerate() {
            let w = u32::from_be_bytes([w[0], w[1], w[2], w[3]]);
            let at = section.addr + 4 * i as u32;
            // bl: opcode 18 with LK set and AA clear; the offset is 26 bits, sign-extended.
            if w & 0xFC00_0003 == 0x4800_0001
                && at.wrapping_add((((w & 0x03FF_FFFC) << 6) as i32 >> 6) as u32) == target
            {
                sites.push(at);
            }
        }
    }
    sites
}

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
    let trace = std::env::var_os("LOCKSTEP_TRACE").is_some();
    panic::set_hook(Box::new(move |info| {
        // Lockstep catches and reports the panics of the checks it runs. LOCKSTEP_TRACE shows
        // where original code was jumping before one.
        if ssbm_rt::lockstep::checking() {
            if trace {
                eprintln!("lockstep panic: {info}");
                INTERP.with(|i| {
                    if let Some(i) = &*i.borrow() {
                        for (from, to) in i.recent_jumps().iter().rev().take(24).rev() {
                            let name = |a: u32| ssbm_types::describe(a).unwrap_or(format!("{a:#010X}"));
                            eprintln!("  jump {} -> {}", name(*from), name(*to));
                        }
                    }
                });
            }
        } else if !info.payload().is::<Stop>() {
            default_hook(info);
        }
    }));

    let ctx = Ctx::new();
    // ORIGINAL_ENTRIES=1 reports where calls entered original code.
    let count_entries = std::env::var_os("ORIGINAL_ENTRIES").is_some();
    if count_entries {
        ctx.count_original_entries();
    }
    let interp = Rc::new(Interpreter::default());
    INTERP.with(|i| *i.borrow_mut() = Some(interp.clone()));
    ctx.set_backend(Box::new(RcBackend(interp.clone())));
    ctx.set_names(Box::new(ssbm_types::describe));
    let dol = disc.main_dol().ok();
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
    // `-unit` in the list leaves that unit (or prefix) out.
    let covers = |p: &str, unit: &str| {
        p == "all" || unit == p || unit.starts_with(&format!("{}/", p.trim_end_matches('/')))
    };
    let units = ssbm_game::register(&ctx, |unit| {
        ports.iter().any(|p| !p.starts_with('-') && covers(p, unit))
            && !ports.iter().any(|p| p.strip_prefix('-').is_some_and(|p| covers(p, unit)))
    });
    let ported: Vec<u32> = ctx
        .registered()
        .into_iter()
        .filter(|a| !before.contains(a))
        .collect();
    if !ports.is_empty() {
        eprintln!("ported: {units} units, {} functions", ported.len());
    }
    // Playback's Gecko codes patch some functions. Those run ports made with the codes where
    // tools/c2rs/patched.py has seen these codes, and their patched original code elsewhere.
    // PATCHED_ORIGINAL=1 keeps them all original.
    // Whether lockstep checks ports yet, for those registered as the run goes.
    let checking = Rc::new(Cell::new(false));
    let mut kept = Vec::new();
    let mut with_codes: Vec<u32> = Vec::new();
    if let Some(dev) = &slippi {
        let codes = ssbm_slippi::applied_codes(dev);
        let mut by_function: std::collections::BTreeMap<u32, Vec<ssbm_game::playback::Applied>> =
            Default::default();
        let mut other_codes = Vec::new();
        for c in &codes {
            let len = match c.kind {
                0x00 => 1,
                0x02 => 2 * ((c.first >> 16) + 1),
                0x06 => c.first,
                _ => 4,
            };
            let at = ssbm_types::functions_overlapping(c.addr, len);
            match c.kind {
                // A code outside any function stands behind a stub called at its address.
                0x04 | 0xC2 => match at.first() {
                    Some(&f) => by_function.entry(f).or_default(),
                    None if c.kind == 0xC2 => by_function.entry(c.addr).or_default(),
                    None => continue,
                }
                .push(ssbm_game::playback::Applied {
                    kind: c.kind,
                    addr: c.addr,
                    first: c.first,
                    words: &c.words,
                }),
                0x00 | 0x02 | 0x06 => other_codes.extend(at),
                _ => {}
            }
        }
        if std::env::var_os("PATCHED_ORIGINAL").is_none() && !ports.is_empty() {
            for (&f, cs) in &by_function {
                let stub = ssbm_types::functions_overlapping(f, 4).is_empty();
                // Code the SDK or the Slippi device stands in for, such as the EXI transfer
                // function, keeps its stand-in.
                if (!stub && !ported.contains(&f))
                    || other_codes.contains(&f)
                    || ctx.entry(f).is_some_and(|e| e.external)
                {
                    continue;
                }
                if let Some(p) = ssbm_game::playback::find(f, cs) {
                    ctx.register_port(f, p.port, ssbm_rt::Returns::Unknown);
                    with_codes.push(f);
                }
            }
            // Callers of a code that returns past its function's caller resume where it says.
            for at in ssbm_slippi::returns_past_caller(dev) {
                for f in ssbm_types::functions_overlapping(at, 4) {
                    for site in dol.as_ref().map_or_else(Vec::new, |d| calls_to(d, f)) {
                        for caller in ssbm_types::functions_overlapping(site, 4) {
                            if ported.contains(&caller)
                                && !with_codes.contains(&caller)
                                && let Some(p) = ssbm_game::playback::find(caller, &[])
                            {
                                ctx.register_port(caller, p.port, ssbm_rt::Returns::Unknown);
                                with_codes.push(caller);
                            }
                        }
                    }
                }
            }
            // Places in injected code that other code calls get ports when first called, at
            // wherever playback placed the code.
            let applied: Vec<ssbm_game::playback::Applied> =
                by_function.values().flatten().copied().collect();
            let entries = ssbm_game::playback::entries(&applied);
            let checked = checking.clone();
            ctx.set_resolver(Box::new(move |ctx, addr| {
                let Some(e) = entries.iter().find(|e| {
                    ssbm_game::playback::placed(ctx, e.site)
                        .is_some_and(|at| at.wrapping_add(e.offset) == addr)
                }) else {
                    return false;
                };
                ctx.register_port(addr, e.port, ssbm_rt::Returns::Unknown);
                if checked.get() {
                    ctx.set_mode(addr, ssbm_rt::Mode::Lockstep);
                }
                true
            }));
        }
        let mut keep = |f: u32| {
            if ported.contains(&f) && !kept.contains(&f) && !with_codes.contains(&f) {
                ctx.set_mode(f, ssbm_rt::Mode::Original);
                kept.push(f);
            }
        };
        for (at, len) in ssbm_slippi::patched(dev) {
            for f in ssbm_types::functions_overlapping(at, len) {
                keep(f);
            }
        }
        // Some codes return past the instructions after the call to their function, so its
        // callers must be the game's code too.
        for at in ssbm_slippi::returns_past_caller(dev) {
            for f in ssbm_types::functions_overlapping(at, 4) {
                for site in dol.as_ref().map_or_else(Vec::new, |d| calls_to(d, f)) {
                    for caller in ssbm_types::functions_overlapping(site, 4) {
                        keep(caller);
                    }
                }
            }
        }
        if !with_codes.is_empty() {
            eprintln!(
                "ported with playback's codes: {} functions and stubs",
                with_codes.len()
            );
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
        // does not reproduce yet. `__setjmp` and `__longjmp` return elsewhere than to their
        // caller.
        let exempt: Vec<u32> = LOCKSTEP_EXEMPT.iter().map(|n| ssbm_sdk::sym(n)).collect();
        let checked: Vec<u32> = ported
            .iter()
            .chain(&with_codes)
            .copied()
            .filter(|a| !kept.contains(a) && !exempt.contains(a))
            .collect();
        let checking = checking.clone();
        let enable = move |ctx: &Ctx| {
            for &addr in &checked {
                ctx.set_mode(addr, ssbm_rt::Mode::Lockstep);
            }
            checking.set(true);
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
    let last_fields = Cell::new((0u64, 0usize));
    let stuck = Cell::new(0u32);
    let pcs: std::cell::RefCell<HashMap<u32, u32>> = Default::default();
    let interp2 = interp.clone();
    ctx.set_heartbeat(move |ctx, pc| {
        let sdk = ctx.ext::<Sdk>();
        let f = sdk.hw.fields.get();
        // A port under lockstep replays the original's interrupts, so fields stand still
        // while it runs; its progress through the replay shows it is not stuck.
        let progress = (f, ctx.lockstep.progress());
        if progress == last_fields.get() {
            stuck.set(stuck.get() + 1);
            *pcs.borrow_mut().entry(pc).or_default() += 1;
            // STUCK_TRACE samples the Rust stack at each stalled heartbeat.
            if std::env::var_os("STUCK_TRACE").is_some() {
                let trace = std::backtrace::Backtrace::force_capture().to_string();
                let ports: Vec<&str> = trace
                    .lines()
                    .filter_map(|l| l.trim().split_once(": ssbm_game::tu::").map(|x| x.1))
                    .collect();
                eprintln!("stalled at {}: {}", ctx.name_of(pc), ports.join(" < "));
            }
            if stuck.get() >= 8 {
                let mut hot: Vec<_> = pcs.borrow().iter().map(|(&pc, &n)| (n, pc)).collect();
                hot.sort_unstable_by(|a, b| b.cmp(a));
                for (n, pc) in hot.iter().take(8) {
                    eprintln!("  {n:4} x {}", ctx.name_of(*pc));
                }
                // Lockstep may catch this and go on, from before the port that spun.
                stuck.set(0);
                pcs.borrow_mut().clear();
                panic!(
                    "stuck: no video field for {} instructions",
                    8 * ssbm_rt::HEARTBEAT
                );
            }
        } else {
            stuck.set(0);
            pcs.borrow_mut().clear();
            last_fields.set(progress);
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
                "  calls that differ only because the original reads stack it never wrote: {}",
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
    if count_entries {
        // Injected code sits where the branch at its code's site leads.
        let placed: Vec<(u32, u32, u32)> = slippi
            .as_ref()
            .map(|d| ssbm_slippi::applied_codes(d))
            .unwrap_or_default()
            .iter()
            .filter(|c| c.kind == 0xC2)
            .filter_map(|c| {
                let w = ctx.mem.read_u32(c.addr).ok()?;
                (w & 0xFC00_0003 == 0x4800_0000).then(|| {
                    let at = c.addr.wrapping_add((((w & 0x03FF_FFFC) << 6) as i32 >> 6) as u32);
                    (c.addr, at, 4 * c.words.len() as u32)
                })
            })
            .collect();
        let entries = ctx.original_entries();
        eprintln!("original code entered at {} addresses:", entries.len());
        for (addr, n) in entries.iter().take(40) {
            let code = placed
                .iter()
                .find(|&&(_, at, len)| addr.wrapping_sub(at) < len)
                .map(|&(site, at, _)| format!(" (code at {site:#010x} +{:#x})", addr - at))
                .unwrap_or_default();
            eprintln!("  {:>9}  {}{code}", n, ctx.name_of(*addr));
        }
    }
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

    fn resume(&self, ctx: &Ctx, pc: u32) {
        self.0.resume(ctx, pc)
    }
}
