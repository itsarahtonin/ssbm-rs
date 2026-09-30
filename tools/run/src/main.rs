// SPDX-License-Identifier: GPL-3.0-or-later

//! Boots the game headless on original code and reports progress.
//!
//! `ssbm-run [disc] [--fields N]` runs for N video fields (default 600, ten seconds). The disc
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
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--fields" => {
                fields = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--fields N")
            }
            _ => disc_path = Some(a),
        }
    }
    let Some(disc_path) = disc_path else {
        eprintln!("usage: ssbm-run [disc] [--fields N]  (or set SSBM_DISC)");
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
        eprintln!("booting {} at {entry:#010X}", disc_path);
        ctx.invoke(entry);
    }));
    let executed = interp.executed.get();
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
