// SPDX-License-Identifier: GPL-3.0-or-later

//! Matches of every fighter, stage and item, for exploring under lockstep. In the debug VS mode
//! (`--mode e`), each match gets, from a seed, random fighters with some of them CPUs, one of
//! the VS stages, items and a short time limit, where the mode would give its defaults; `--monkey`
//! plays the humans. `MATCH_STAGES` and `MATCH_FIGHTERS` list other stages and fighters to pick
//! from, by StKind and CKind, and `MATCH_TIME` sets the time limit in seconds: the results screen
//! can't show the bosses, so a run with them needs a match that outlasts it. `MATCH_RULES=1`
//! varies the rules too. Deterministic for a seed.

use std::cell::Cell;
use std::rc::Rc;

use ssbm_rt::{At, Ctx};
use ssbm_types::enums::*;
use ssbm_types::records::{StartMeleeData, gmm_x0};

use crate::monkey::Rng;

/// Time limit of each match, in seconds, unless `MATCH_TIME` says.
const TIME_LIMIT: u32 = 90;

/// The stages the VS stage select offers. Akaneia and Icetop have no VS stage parameters: the
/// game reports them missing and stops there.
const STAGES: [i32; 29] = [
    St_Kind_Izumi, St_Kind_PStadium, St_Kind_Castle, St_Kind_Kongo, St_Kind_Zebes,
    St_Kind_Corneria, St_Kind_Story, St_Kind_Onett, St_Kind_MuteCity, St_Kind_RCruise,
    St_Kind_Garden, St_Kind_GreatBay, St_Kind_Shrine, St_Kind_Kraid, St_Kind_Yoster,
    St_Kind_Greens, St_Kind_Fourside, St_Kind_Inishie1, St_Kind_Inishie2, St_Kind_Venom,
    St_Kind_Pura, St_Kind_BigBlue, St_Kind_Icemt, St_Kind_Flatzone, St_Kind_OldPupupu,
    St_Kind_OldYoshi, St_Kind_OldKongo, St_Kind_Battle, St_Kind_Last,
];

/// Every fighter, by CKind: the first 26 are the playable ones.
const FIGHTERS: [&str; 32] = [
    "Captain Falcon", "Donkey Kong", "Fox", "Mr. Game & Watch", "Kirby", "Bowser", "Link",
    "Luigi", "Mario", "Marth", "Mewtwo", "Ness", "Peach", "Pikachu", "Ice Climbers",
    "Jigglypuff", "Samus", "Yoshi", "Zelda", "Sheik", "Falco", "Young Link", "Dr. Mario", "Roy",
    "Pichu", "Ganondorf", "Master Hand", "Wireframe (male)", "Wireframe (female)", "Giga Bowser",
    "Crazy Hand", "Sandbag",
];
const PLAYABLE: usize = 26;

pub fn install(ctx: &Ctx, seed: u64) {
    let rng = Rng::new(seed);
    let list = |var: &str| -> Option<Vec<i32>> {
        let list = std::env::var(var).ok()?;
        let parse = |s: &str| s.trim().parse().unwrap_or_else(|_| panic!("{var}: {list}"));
        Some(list.split(',').map(parse).collect())
    };
    let stages = list("MATCH_STAGES").unwrap_or_else(|| STAGES.to_vec());
    let fighters = list("MATCH_FIGHTERS").unwrap_or_else(|| (0..PLAYABLE as i32).collect());
    let time = list("MATCH_TIME").map_or(TIME_LIMIT, |t| t[0] as u32);
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
                choose(ctx, &rng, &stages, &fighters, time);
            }
        }),
    );
}

/// Makes the Event mode start at event match `n` (from 0), as if its event select had chosen
/// it: entering the mode loads that event's files, and its character select if it has one.
pub fn install_event(ctx: &Ctx, n: u8) {
    ctx.set_hook(
        ssbm_sdk::sym("gm_Mode_Event_OnLoad"),
        Rc::new(move |ctx| {
            let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
            game.vs().unk_530().set_unk_535(n);
        }),
    );
}

/// Makes the game Japanese, as on a disc without `/usa.ini`: the language it shows, and the one
/// its save data's preferences start with, which the code that resets them takes from it.
pub fn install_japanese(ctx: &Ctx) {
    ctx.set_hook(
        ssbm_sdk::sym("gmMainLib_8015F600"),
        Rc::new(|ctx| {
            let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
            game.set_language(0);
        }),
    );
}

/// Makes every match's human players level 9 CPUs, so the modes that end when the player
/// loses go on further than random play takes them.
pub fn install_cpu_players(ctx: &Ctx) {
    ctx.set_hook(
        ssbm_sdk::sym("fn_8016E730"),
        Rc::new(|ctx| {
            let data = StartMeleeData(At::new(ctx, ctx.regs.r(3)));
            for i in 0..4 {
                let p = data.players().get(i);
                if p.slot_type() == Gm_PKind_Human as u8 {
                    p.set_slot_type(Gm_PKind_Cpu as u8);
                    p.set_cpu_level(9);
                }
            }
        }),
    );
}

fn choose(ctx: &Ctx, rng: &Rng, stages: &[i32], fighters: &[i32], time: u32) {
    let data = StartMeleeData(At::new(ctx, ssbm_sdk::sym("gmVsMelee_StartData")));
    let rules = data.rules();
    let stage = stages[rng.below(stages.len() as u64) as usize];
    rules.set_stkind(stage as u16);
    rules.set_match_kind(MatchKind_Time as u32);
    rules.set_timer_enabled(1);
    rules.set_time_limit(time);
    // Random presses of Start would keep the game paused.
    rules.set_disable_pausing(1);
    // Items from none to very high, mostly high.
    rules.set_item_freq(if rng.chance(15) {
        -1
    } else {
        rng.below(5) as i8
    });
    // MATCH_RULES=1 varies the rules as the VS rules and Special Melee do: stock and coin
    // matches, teams with and without friendly fire, damage ratios, Slo-Mo and Lightning
    // speeds and single-button play; and for each player, metal, invisible, Giant and Tiny.
    let varied = std::env::var_os("MATCH_RULES").is_some();
    if varied {
        let kind = match rng.below(10) {
            0..=5 => MatchKind_Time,
            6..=8 => MatchKind_Stock,
            _ => MatchKind_Coin,
        };
        rules.set_match_kind(kind as u32);
        rules.set_is_stock(u32::from(kind == MatchKind_Stock));
        let teams = rng.chance(30);
        rules.set_is_teams(u8::from(teams));
        rules.set_friendly_fire(u32::from(teams && rng.chance(50)));
        rules.set_single_button(u32::from(rng.chance(10)));
        // Each match's rules start from the defaults, not the last match's.
        rules.set_x30(if rng.chance(20) {
            0.5 + rng.below(16) as f64 * 0.1
        } else {
            1.0
        });
        rules.set_game_speed(match rng.below(20) {
            0..=1 => 0.5,
            2 => 1.25,
            _ => 1.0,
        });
    }
    let mut chosen = Vec::new();
    for i in 0..4 {
        let p = data.players().get(i);
        if varied {
            p.set_stocks(1 + rng.below(4) as i8);
            p.set_team(rng.below(3) as u8);
            p.set_vs_metal(u8::from(rng.chance(10)));
            p.set_vs_invisible(u8::from(rng.chance(10)));
            p.set_model_scale(match rng.below(14) {
                0 => 1.8,
                1 => 0.35,
                _ => 1.0,
            });
        }
        let fighter = fighters[rng.below(fighters.len() as u64) as usize] as usize;
        // Bosses and the other special fighters only play as CPUs, as in the modes that have
        // them: the game crashes with some of them under a player's control.
        let cpu = fighter >= PLAYABLE || rng.chance(50);
        p.set_ckind(fighter as i8);
        p.set_slot_type(if cpu { Gm_PKind_Cpu } else { Gm_PKind_Human } as u8);
        p.set_cpu_level(if cpu { 1 + rng.below(9) as u8 } else { 0 });
        p.set_color(0);
        // The special fighters start standing, as the modes that have them start them (the
        // entry animation they lack faults), and the hands with the stamina Classic gives them.
        if fighter >= PLAYABLE {
            p.set_xD_b2(1);
        }
        if [CKind_MasterH, CKind_CrezyH].contains(&(fighter as i32)) {
            p.set_xC_b7(1);
            p.set_hp(300);
            p.set_xD_b0(1);
            p.set_spawn_dir(-1);
        }
        if fighter as i32 == CKind_GKoops {
            p.set_xC_b1(0);
        }
        chosen.push(format!("{}{}", FIGHTERS[fighter], if cpu { " (CPU)" } else { "" }));
    }
    let field = ctx.ext::<ssbm_sdk::Sdk>().hw.fields.get();
    eprintln!("field {field}: match on stage {stage}: {}", chosen.join(", "));
}
