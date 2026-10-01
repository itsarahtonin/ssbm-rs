// SPDX-License-Identifier: GPL-3.0-or-later

//! Probes: lockstep checks of functions no input has reached, called directly on live game
//! objects. `PROBES=FILE` lists them, as `tools/lockstep/probes.py` writes them (address,
//! object class, fighter kind or -1, one letter per further parameter: i an integer, f a
//! float). Once a video field, outside any check, the next `PROBE_RATE` of them (default 8)
//! whose object is around run under lockstep, each on an object of its class (and fighter kind)
//! with small numbers for the rest, and everything they do is undone; `PROBE_LIMIT` (default
//! 20) probes each function at most. Deterministic for `PROBE_SEED`.

use std::cell::Cell;
use std::rc::Rc;

use ssbm_rt::Ctx;
use ssbm_sdk::{Sdk, hw};

use crate::monkey::Rng;

/// GObj fields.
const CLASSIFIER: u32 = 0x0;
const NEXT: u32 = 0x8;
const USER_DATA: u32 = 0x2C;
/// `Fighter.kind`.
const FIGHTER_KIND: u32 = 0x4;

struct Probe {
    addr: u32,
    class: u16,
    kind: i32,
    params: Vec<u8>,
    done: Cell<u32>,
}

pub fn install(sdk: &Rc<Sdk>, path: &str) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let probes: Vec<Probe> = text
        .lines()
        .filter_map(|l| {
            let l = l.split('#').next()?.trim();
            let mut f = l.split_whitespace();
            let addr = u32::from_str_radix(f.next()?.trim_start_matches("0x"), 16).ok()?;
            let class = f.next()?.parse().ok()?;
            let kind = f.next()?.parse().ok()?;
            let params = f.next()?.bytes().filter(|&b| b != b'-').collect();
            Some(Probe {
                addr,
                class,
                kind,
                params,
                done: Cell::new(0),
            })
        })
        .collect();
    let var = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let rate = var("PROBE_RATE", 8) as usize;
    let limit = var("PROBE_LIMIT", 20) as u32;
    let state = Rc::new(State {
        probes,
        next: Cell::new(0),
        rng: Rng::new(var("PROBE_SEED", 1)),
        rate,
        limit,
    });
    schedule(sdk, state, 1);
}

struct State {
    probes: Vec<Probe>,
    next: Cell<usize>,
    rng: Rng,
    rate: usize,
    limit: u32,
}

fn schedule(sdk: &Rc<Sdk>, state: Rc<State>, field: u64) {
    sdk.schedule(hw::field_start(field), move |ctx| {
        if !ctx.lockstep.is_active() {
            probe_some(ctx, &state);
        }
        schedule(&ctx.ext::<Sdk>(), state, field + 1);
    });
}

/// Live game objects: (GObj, its class, the fighter kind for fighters).
fn objects(ctx: &Ctx) -> Vec<(u32, u16, i32)> {
    let heads = ctx.mem.read_u32(ssbm_sdk::sym("HSD_GObjPLinkHead")).unwrap_or(0);
    let links = ctx.mem.read_u8(ssbm_sdk::sym("HSD_GObjLibInitData")).unwrap_or(0);
    let mut out = Vec::new();
    if heads == 0 {
        return out;
    }
    for p in 0..u32::from(links) {
        let mut g = ctx.mem.read_u32(heads + 4 * p).unwrap_or(0);
        let mut n = 0;
        while g != 0 && n < 4096 {
            let class = ctx.mem.read_u16(g + CLASSIFIER).unwrap_or(0);
            let kind = if class == 4 {
                ctx.mem
                    .read_u32(g + USER_DATA)
                    .ok()
                    .filter(|&d| d != 0)
                    .and_then(|d| ctx.mem.read_u32(d + FIGHTER_KIND).ok())
                    .map_or(-1, |k| k as i32)
            } else {
                -1
            };
            out.push((g, class, kind));
            g = ctx.mem.read_u32(g + NEXT).unwrap_or(0);
            n += 1;
        }
    }
    out
}

fn probe_some(ctx: &Ctx, state: &State) {
    if state.probes.is_empty() {
        return;
    }
    let objects = objects(ctx);
    if objects.is_empty() {
        return;
    }
    let mut ran = 0;
    // A full pass over the list at most, so fields without the right objects stay short.
    for _ in 0..state.probes.len() {
        if ran == state.rate {
            break;
        }
        let i = state.next.get();
        state.next.set((i + 1) % state.probes.len());
        let p = &state.probes[i];
        if p.done.get() >= state.limit {
            continue;
        }
        let fits: Vec<u32> = objects
            .iter()
            .filter(|&&(_, class, kind)| class == p.class && (p.kind < 0 || kind == p.kind))
            .map(|&(g, ..)| g)
            .collect();
        if fits.is_empty() {
            continue;
        }
        let gobj = fits[state.rng.below(fits.len() as u64) as usize];
        let rng = &state.rng;
        let ran_it = ssbm_rt::lockstep::probe(ctx, p.addr, |ctx| {
            ctx.regs.set_r(3, gobj);
            let (mut r, mut f) = (4, 1);
            for &k in &p.params {
                if k == b'f' {
                    let v = [0.0, 1.0, -1.0, 0.5, 2.0, 10.0][rng.below(6) as usize];
                    ctx.regs.set_f(f, v);
                    f += 1;
                } else {
                    ctx.regs.set_r(r, rng.below(5) as u32);
                    r += 1;
                }
            }
        });
        if ran_it {
            p.done.set(p.done.get() + 1);
            ran += 1;
        }
    }
}
