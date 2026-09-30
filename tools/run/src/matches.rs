// SPDX-License-Identifier: GPL-3.0-or-later

//! Matches of every fighter, stage and item, for exploring under lockstep. In the debug VS mode
//! (`--mode e`), each match gets, from a seed, random fighters with some of them CPUs, a VS
//! stage, items and a short time limit, where the mode would give its defaults; `--monkey`
//! plays the humans. Deterministic for a seed.

use std::cell::Cell;
use std::rc::Rc;

use ssbm_rt::{At, Ctx};
use ssbm_types::enums::*;
use ssbm_types::records::StartMeleeData;

use crate::monkey::Rng;

/// Time limit of each match, in seconds.
const TIME_LIMIT: u32 = 90;

/// The playable fighters, by CKind.
const FIGHTERS: [&str; 26] = [
    "Captain Falcon", "Donkey Kong", "Fox", "Mr. Game & Watch", "Kirby", "Bowser", "Link",
    "Luigi", "Mario", "Marth", "Mewtwo", "Ness", "Peach", "Pikachu", "Ice Climbers",
    "Jigglypuff", "Samus", "Yoshi", "Zelda", "Sheik", "Falco", "Young Link", "Dr. Mario", "Roy",
    "Pichu", "Ganondorf",
];

pub fn install(ctx: &Ctx, seed: u64) {
    let rng = Rng::new(seed);
    // The debug VS mode sets its defaults, then loads the announcer's voice clips: its
    // defaults are set by then.
    let entered = Rc::new(Cell::new(false));
    let armed = entered.clone();
    ctx.set_hook(
        ssbm_sdk::sym("onEnterDebugVs"),
        Rc::new(move |_| armed.set(true)),
    );
    ctx.set_hook(
        ssbm_sdk::sym("gm_LoadAnnouncer"),
        Rc::new(move |ctx| {
            if entered.replace(false) {
                choose(ctx, &rng);
            }
        }),
    );
}

fn choose(ctx: &Ctx, rng: &Rng) {
    let data = StartMeleeData(At::new(ctx, ssbm_sdk::sym("gmVsMelee_StartData")));
    let rules = data.rules();
    let stage = St_Kind_Izumi + rng.below((St_Kind_Last - St_Kind_Izumi + 1) as u64) as i32;
    rules.set_stkind(stage as u16);
    rules.set_match_kind(MatchKind_Time as u32);
    rules.set_timer_enabled(1);
    rules.set_time_limit(TIME_LIMIT);
    // Random presses of Start would keep the game paused.
    rules.set_disable_pausing(1);
    // Items from none to very high, mostly high.
    rules.set_item_freq(if rng.chance(15) {
        -1
    } else {
        rng.below(5) as i8
    });
    let mut chosen = Vec::new();
    for i in 0..4 {
        let p = data.players().get(i);
        let fighter = rng.below(FIGHTERS.len() as u64) as usize;
        let cpu = rng.chance(50);
        p.set_ckind(fighter as i8);
        p.set_slot_type(if cpu { Gm_PKind_Cpu } else { Gm_PKind_Human } as u8);
        p.set_cpu_level(if cpu { 1 + rng.below(9) as u8 } else { 0 });
        p.set_color(0);
        chosen.push(format!("{}{}", FIGHTERS[fighter], if cpu { " (CPU)" } else { "" }));
    }
    let field = ctx.ext::<ssbm_sdk::Sdk>().hw.fields.get();
    eprintln!("field {field}: match on stage {stage}: {}", chosen.join(", "));
}
