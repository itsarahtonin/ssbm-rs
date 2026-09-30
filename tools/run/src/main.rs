// SPDX-License-Identifier: GPL-3.0-or-later

//! Boots the game headless on original code and reports progress.
//!
//! `ssbm-run [disc] [--fields N] [--replay FILE]` runs for N video fields (default 600, ten
//! seconds), playing back a Slippi replay if given. The disc
//! path defaults to `SSBM_DISC`.

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

fn main() -> ExitCode {
    let mut disc_path = std::env::var("SSBM_DISC").ok();
    let mut fields = 600;
    let mut replay_path = None;
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
            _ => disc_path = Some(a),
        }
    }
    let Some(disc_path) = disc_path else {
        eprintln!("usage: ssbm-run [disc] [--fields N] [--replay FILE]  (or set SSBM_DISC)");
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
        if !info.payload().is::<Stop>() {
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
        // Replays recorded in Slippi Dolphin carry its rounding of fused multiply-adds.
        gekko_fp::set_fma_mode(gekko_fp::FmaMode::SlippiDolphin);
        ssbm_slippi::install(&ctx, replay)
    });
    let slippi: Option<Rc<ssbm_slippi::Device>> = slippi;

    if std::env::var_os("LBMEM_TRACE").is_some() {
        // Allocation requests from the game's file heaps: handle, size, free space.
        ctx.set_hook(
            ssbm_sdk::sym("lbMemory_80014FC8"),
            Rc::new(|ctx| {
                let h = ctx.regs.r(3);
                let size = ctx.regs.r(4);
                let (lo, hi) = (ctx.read_u32(h + 4), ctx.read_u32(h + 8));
                eprintln!(
                    "lbMemory alloc {size:#x} from heap {h:08X} [{lo:08X}..{hi:08X}] lr {}",
                    ctx.name_of(ctx.regs.lr.get())
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
                    let f: Vec<String> = [0, 1, 2, 31]
                        .iter()
                        .map(|&i| format!("f{i}={:016X}", ctx.regs.f(i).to_bits()))
                        .collect();
                    let base = addr & !0xFF;
                    eprintln!(
                        "    {}  kb_vel {:08X} {:08X}",
                        f.join(" "),
                        ctx.read_u32(base + 0x8C),
                        ctx.read_u32(base + 0x90)
                    );
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
        if slippi.is_some() {
            ssbm_slippi::apply_bootloader(&ctx);
        }
        eprintln!("booting {} at {entry:#010X}", disc_path);
        ctx.invoke(entry);
    }));
    let executed = interp.executed.get();
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
        eprintln!(
            "replay check: {} events compared through frame {:?}, {} not reached, {} diverge",
            report.compared,
            report.last_frame_compared,
            report.missing,
            report.divergences.len()
        );
        let mut kinds: Vec<(&str, &str, usize, i32)> = Vec::new();
        for d in &report.divergences {
            match kinds.iter_mut().find(|k| k.0 == d.event && k.1 == d.field) {
                Some(k) => k.2 += 1,
                None => kinds.push((d.event, d.field, 1, d.frame)),
            }
        }
        for (event, field, n, first) in &kinds {
            eprintln!("  {n:6} x {event} {field}, first on frame {first}");
        }
        for d in report.divergences.iter().take(6) {
            eprintln!("  {d}");
        }
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
        Err(p) if p.is::<Stop>() => ExitCode::SUCCESS,
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
