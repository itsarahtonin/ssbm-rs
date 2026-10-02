// SPDX-License-Identifier: GPL-3.0-or-later

//! Saved calls (ssbm_rt::capture) as files: CAPTURE=DIR saves each function's first two
//! mismatching checks, and with CAPTURE_FUNCS=NAME,... a few real calls of those functions and
//! their first mutated checks, named for how each ended, as DIR/NAME-KIND-N.call (zstd);
//! `--call FILE` checks one again apart from any run. The files hold captured game memory: keep
//! them under local/.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ssbm_rt::Ctx;
use ssbm_rt::capture::Call;

/// Mismatching checks saved per function.
const MISMATCHES: u32 = 2;

/// Mutated checks of a function CAPTURE_FUNCS names that are saved, to tell whether checking one
/// again apart from its run ends as it did there.
const MUTATED: u64 = 20;

/// Which real calls of a function CAPTURE_FUNCS asks for are saved, by count: spread out, so
/// they find the game in different states.
const SAVED_CALLS: [u64; 5] = [1, 10, 100, 1000, 10000];

pub fn read(path: &Path) -> Call {
    let packed = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let bytes = zstd::decode_all(&packed[..]).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Call::from_bytes(&bytes).unwrap_or_else(|| panic!("{}: not a saved call", path.display()))
}

fn write(path: &Path, call: &Call) {
    let packed = zstd::encode_all(&call.to_bytes()[..], 3).expect("zstd");
    std::fs::write(path, packed).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Saves calls into `dir` as the module says: mismatches, and real calls of `funcs`.
pub fn install_capture(ctx: &Ctx, dir: PathBuf, funcs: Vec<u32>) {
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    // Per function: mismatches saved, real calls seen, mutated checks seen.
    let counts: RefCell<BTreeMap<u32, (u32, u64, u64)>> = RefCell::default();
    let saved = RefCell::new(0u32);
    *ctx.lockstep.capture.borrow_mut() = Some(Rc::new(move |ctx, addr, mismatched, take| {
        let mutated = ctx.lockstep.is_mutating();
        let mut counts = counts.borrow_mut();
        let (mismatches, calls, mutations) = counts.entry(addr).or_default();
        let wanted = if mismatched {
            *mismatches += 1;
            *mismatches <= MISMATCHES
        } else if funcs.contains(&addr) && mutated {
            *mutations += 1;
            *mutations <= MUTATED
        } else if funcs.contains(&addr) {
            *calls += 1;
            SAVED_CALLS.contains(calls)
        } else {
            false
        };
        if !wanted {
            return;
        }
        let mut n = saved.borrow_mut();
        *n += 1;
        let kind = match (mismatched, mutated) {
            (true, true) => "mutated-mismatch",
            (true, false) => "mismatch",
            (false, true) => "mutated-ok",
            (false, false) => "call",
        };
        let path = dir.join(format!("{}-{kind}{}.call", ctx.name_of(addr), *n));
        write(&path, &take());
        eprintln!("saved {}", path.display());
    }));
}

/// Checks the saved call at `path` again, `repeat` times, each from the state it was saved in;
/// LOCKSTEP_MUTATE adds mutated checks of each as a run's checks do.
pub fn replay(ctx: &Ctx, path: &Path, repeat: u32) {
    let call = read(path);
    eprintln!(
        "checking {} again{}, {repeat} times",
        ctx.name_of(call.addr),
        if call.mutated { " as a mutated check" } else { "" }
    );
    for _ in 0..repeat {
        call.load(ctx);
        ctx.lockstep.resume(call.mutated, call.stub);
        ctx.invoke(call.addr);
        ctx.lockstep.resume(false, None);
    }
}
