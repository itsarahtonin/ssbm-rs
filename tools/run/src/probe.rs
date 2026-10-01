// SPDX-License-Identifier: GPL-3.0-or-later

//! Probes: lockstep checks of functions no input has reached, called directly on live game
//! objects. `PROBES=FILE` lists them, as `tools/lockstep/probes.py` writes them: address, the
//! GObj class of the first object parameter (-1 for any), a fighter kind (-1 for any), and one
//! letter per parameter: g a game object, F/I/R a fighter's, item's or ground's user data, j/c/l
//! a game object's joint, camera or light, p scratch memory (zeroed, or holding small numbers or
//! floats), i an integer, f a float. Once a video field, outside any check, the next
//! `PROBE_RATE` of them (default 8) whose objects are around run under lockstep, and everything
//! they do is undone; a probe for a fighter kind that isn't around takes another fighter a
//! quarter of the time. `PROBE_LIMIT` (default 20) probes each function at most. Deterministic
//! for `PROBE_SEED`. `PROBE_LOG=1` names each probe as it starts.

use std::cell::Cell;
use std::rc::Rc;

use ssbm_rt::Ctx;
use ssbm_sdk::{Sdk, hw};

use crate::monkey::Rng;

/// GObj fields.
const CLASSIFIER: u32 = 0x0;
const OBJ_KIND: u32 = 0x6;
const NEXT: u32 = 0x8;
const HSD_OBJ: u32 = 0x28;
const USER_DATA: u32 = 0x2C;
/// `Fighter.kind`.
const FIGHTER_KIND: u32 = 0x4;
const CLASS_FIGHTER: u16 = 4;
const CLASS_ITEM: u16 = 6;
const CLASS_GROUND: u16 = 13;
/// Bytes of zeroed scratch memory each pointer parameter gets, and kept clear around them.
const SCRATCH: u32 = 0x100;
const SCRATCH_ROOM: u32 = 0x400;

struct Probe {
    addr: u32,
    class: i32,
    kind: i32,
    params: Vec<u8>,
    done: Cell<u32>,
}

struct State {
    probes: Vec<Probe>,
    next: Cell<usize>,
    rng: Rng,
    rate: usize,
    limit: u32,
    log: bool,
}

/// A live game object.
#[derive(Clone, Copy)]
struct Object {
    gobj: u32,
    class: u16,
    kind: i32,
    obj_kind: u8,
    hsd_obj: u32,
    user_data: u32,
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
    let state = Rc::new(State {
        probes,
        next: Cell::new(0),
        rng: Rng::new(var("PROBE_SEED", 1)),
        rate: var("PROBE_RATE", 8) as usize,
        limit: var("PROBE_LIMIT", 20) as u32,
        log: std::env::var_os("PROBE_LOG").is_some(),
    });
    schedule(sdk, state, 1);
}

fn schedule(sdk: &Rc<Sdk>, state: Rc<State>, field: u64) {
    sdk.schedule(hw::field_start(field), move |ctx| {
        if !ctx.lockstep.is_active() {
            probe_some(ctx, &state);
        }
        schedule(&ctx.ext::<Sdk>(), state, field + 1);
    });
}

fn objects(ctx: &Ctx) -> Vec<Object> {
    let heads = ctx.mem.read_u32(ssbm_sdk::sym("HSD_GObjPLinkHead")).unwrap_or(0);
    let links = ctx.mem.read_u8(ssbm_sdk::sym("HSD_GObjLibInitData")).unwrap_or(0);
    let mut out = Vec::new();
    if heads == 0 {
        return out;
    }
    let word = |a: u32| ctx.mem.read_u32(a).unwrap_or(0);
    for p in 0..u32::from(links) {
        let mut g = word(heads + 4 * p);
        let mut n = 0;
        while g != 0 && n < 4096 {
            let class = ctx.mem.read_u16(g + CLASSIFIER).unwrap_or(0);
            let user_data = word(g + USER_DATA);
            let kind = if class == CLASS_FIGHTER && user_data != 0 {
                word(user_data + FIGHTER_KIND) as i32
            } else {
                -1
            };
            out.push(Object {
                gobj: g,
                class,
                kind,
                obj_kind: ctx.mem.read_u8(g + OBJ_KIND).unwrap_or(0xFF),
                hsd_obj: word(g + HSD_OBJ),
                user_data,
            });
            g = word(g + NEXT);
            n += 1;
        }
    }
    out
}

/// The value a probe passes for parameter letter `k`: `first` is whether it is the first object
/// parameter, which takes the probe's class and kind, or only its class with `any_kind`. None
/// if no object fits.
fn object_for(
    ctx: &Ctx,
    p: &Probe,
    k: u8,
    first: bool,
    any_kind: bool,
    objects: &[Object],
    rng: &Rng,
) -> Option<u32> {
    let kind_of = |name: &str| ctx.mem.read_u8(ssbm_sdk::sym(name)).unwrap_or(0xFE);
    let pick = |fits: Vec<u32>| (!fits.is_empty()).then(|| fits[rng.below(fits.len() as u64) as usize]);
    let class_fits = |o: &Object, class: Option<u16>| {
        let (want, kind) = if first { (p.class, p.kind) } else { (-1, -1) };
        class.is_none_or(|c| o.class == c)
            && (want < 0 || i32::from(o.class) == want)
            && (kind < 0 || any_kind || o.kind == kind)
    };
    match k {
        b'g' => pick(objects.iter().filter(|o| class_fits(o, None)).map(|o| o.gobj).collect()),
        b'F' | b'I' | b'R' => {
            let class = match k {
                b'F' => CLASS_FIGHTER,
                b'I' => CLASS_ITEM,
                _ => CLASS_GROUND,
            };
            pick(
                objects
                    .iter()
                    .filter(|o| class_fits(o, Some(class)) && o.user_data != 0)
                    .map(|o| o.user_data)
                    .collect(),
            )
        }
        _ => {
            let want = kind_of(match k {
                b'j' => "HSD_GObj_JObjKind",
                b'c' => "HSD_GObj_CameraKind",
                _ => "HSD_GObj_LightKind",
            });
            pick(
                objects
                    .iter()
                    .filter(|o| o.obj_kind == want && o.hsd_obj != 0)
                    .map(|o| o.hsd_obj)
                    .collect(),
            )
        }
    }
}

/// An integer argument: mostly small, as kinds, indices and flags are, and now and then any.
fn int(rng: &Rng) -> u32 {
    match rng.below(10) {
        0..=4 => rng.below(5) as u32,
        5 | 6 => rng.below(64) as u32,
        7 => u32::MAX,
        8 => rng.below(0x1_0000) as u32,
        _ => rng.below(1 << 32) as u32,
    }
}

/// A float argument: mostly one of a few round values, otherwise any up to a hundred.
fn float(rng: &Rng) -> f32 {
    match rng.below(8) {
        0..=5 => [0.0, 1.0, -1.0, 0.5, 2.0, 10.0][rng.below(6) as usize],
        _ => (rng.below(20_001) as f32 - 10_000.0) / 100.0,
    }
}

fn probe_some(ctx: &Ctx, state: &State) {
    if state.probes.is_empty() {
        return;
    }
    let objects = objects(ctx);
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
        // The objects first, so a probe without them doesn't run.
        let choose = |any_kind: bool| -> Option<Vec<Option<u32>>> {
            let mut values = Vec::new();
            let mut first = true;
            for &k in &p.params {
                if b"gFIRjcl".contains(&k) {
                    let is_first = first && b"gFIR".contains(&k);
                    let v = object_for(ctx, p, k, is_first, any_kind, &objects, &state.rng)?;
                    values.push(Some(v));
                    first &= !b"gFIR".contains(&k);
                } else {
                    values.push(None);
                }
            }
            Some(values)
        };
        let Some(values) = choose(false)
            .or_else(|| (p.kind >= 0 && state.rng.chance(25)).then(|| choose(true)).flatten())
        else {
            continue;
        };
        if state.log {
            eprintln!("probe {} on {values:08X?}", ctx.name_of(p.addr));
        }
        let rng = &state.rng;
        let ran_it = ssbm_rt::lockstep::probe(ctx, p.addr, |ctx| {
            // Scratch memory sits above a lowered stack pointer, with zeroed room around it for
            // the headers and neighbors code may reach for, so the callee's frames stay apart.
            let scratch = p.params.iter().filter(|&&k| k == b'p').count() as u32 * SCRATCH;
            let sp = ctx.regs.r(1);
            let base = (sp - SCRATCH_ROOM - scratch) & !0x1F;
            let low = base - SCRATCH_ROOM;
            let _ = ctx.mem.write_bytes(low, &vec![0; (sp - low) as usize]);
            // Half the time the scratch memory holds small numbers or floats instead of zeros.
            match rng.below(4) {
                2 => (base..base + scratch)
                    .step_by(4)
                    .for_each(|a| ctx.write_u32(a, rng.below(8) as u32)),
                3 => (base..base + scratch)
                    .step_by(4)
                    .for_each(|a| ctx.write_u32(a, float(rng).to_bits())),
                _ => {}
            }
            ctx.regs.set_r(1, (low - 0x20) & !0xF);
            let (mut r, mut f, mut next) = (3, 1, base);
            for (&k, v) in p.params.iter().zip(&values) {
                match k {
                    b'f' => {
                        ctx.regs.set_f(f, f64::from(float(rng)));
                        f += 1;
                    }
                    b'i' => {
                        ctx.regs.set_r(r, int(rng));
                        r += 1;
                    }
                    b'p' => {
                        ctx.regs.set_r(r, next);
                        next += SCRATCH;
                        r += 1;
                    }
                    _ => {
                        ctx.regs.set_r(r, v.unwrap_or(0));
                        r += 1;
                    }
                }
            }
        });
        if ran_it {
            p.done.set(p.done.get() + 1);
            ran += 1;
        }
    }
}
