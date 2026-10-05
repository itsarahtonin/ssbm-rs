// SPDX-License-Identifier: GPL-3.0-or-later

//! Matches of every fighter, stage and item, for exploring under lockstep. In the debug VS mode
//! (`--mode e`), each match gets, from a seed, random fighters with some of them CPUs, one of
//! the VS stages, items and a short time limit, where the mode would give its defaults; `--monkey`
//! plays the humans. `MATCH_STAGES` and `MATCH_FIGHTERS` list other stages and fighters to pick
//! from, by StKind and CKind, and `MATCH_TIME` sets the time limit in seconds: the results screen
//! can't show the bosses, so a run with them needs a match that outlasts it. `MATCH_RULES=1`
//! varies the rules too. `MATCH_CPU_KIND=K[,K...]` gives the CPUs CPU kinds (the AI's routine,
//! `CpuKind`) drawn from that list, `MATCH_STAMINA=HP` makes the matches stamina matches with
//! that HP, `MATCH_TEAMS=1` makes them team matches and `MATCH_PLAYERS=N` leaves all but the
//! first N ports empty. `MATCH_ITEMS=K[,K...]` spawns only those item kinds (ItemKind) and
//! `MATCH_HUMAN_HANDS=1` has Master Hand and Crazy Hand join as humans, played from ports 3 and
//! 4 as their code reads them. A match with the hands has them as Classic does (see
//! `hands_lineup`). `MATCH_VS=1` gives the VS mode's matches (`--mode 2`) the same, as each
//! starts past its character and stage select, so that their results go through the VS mode's
//! records: all but the fighters and stage, which stay those selects' choices. Deterministic for
//! a seed.

use std::cell::Cell;
use std::rc::Rc;

use ssbm_rt::{At, Ctx, Hook};
use ssbm_types::enums::*;
use ssbm_types::records::{GmStats, MenuEnterData, StartMeleeData, gmm_x0};

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

/// What `choose` gives a match: the stages and fighters to draw from, the time limit and the
/// MATCH_* rules.
struct Rules {
    stages: Vec<i32>,
    fighters: Vec<i32>,
    time: u32,
    cpu_kinds: Option<Vec<u8>>,
    stamina: Option<u16>,
    teams: bool,
    players: Option<usize>,
    items: Option<u64>,
    human_hands: bool,
}

/// A comma-separated list of numbers in `var`, if it is set.
fn list(var: &str) -> Option<Vec<i32>> {
    let list = std::env::var(var).ok()?;
    let parse = |s: &str| s.trim().parse().unwrap_or_else(|_| panic!("{var}: {list}"));
    Some(list.split(',').map(parse).collect())
}

/// MATCH_CPU_KIND=K[,K...]: the CPU kinds (CpuKind, 0 to 29) CPUs get, each drawn from these.
fn cpu_kinds() -> Option<Vec<u8>> {
    list("MATCH_CPU_KIND").map(|l| l.into_iter().map(|k| k as u8).collect())
}

pub fn install(ctx: &Ctx, seed: u64) {
    let rng = Rc::new(Rng::new(seed));
    let rules = Rc::new(Rules {
        stages: list("MATCH_STAGES").unwrap_or_else(|| STAGES.to_vec()),
        fighters: list("MATCH_FIGHTERS").unwrap_or_else(|| (0..PLAYABLE as i32).collect()),
        time: list("MATCH_TIME").map_or(TIME_LIMIT, |t| t[0] as u32),
        cpu_kinds: cpu_kinds(),
        // MATCH_STAMINA=HP: stamina matches, each fighter with HP hit points and one life, as
        // the Special Melee's Stamina mode starts them (gm_801B931C).
        stamina: list("MATCH_STAMINA").map(|h| h[0] as u16),
        // MATCH_TEAMS=1: team matches.
        teams: std::env::var_os("MATCH_TEAMS").is_some(),
        // MATCH_PLAYERS=N: the debug VS mode's matches with only the first N ports playing.
        players: list("MATCH_PLAYERS").map(|n| n[0] as usize),
        // MATCH_ITEMS=K[,K...]: only these item kinds (ItemKind) spawn, as the VS item switch
        // leaves the rules' mask (x20, which the item spawner reads through gm_8016AEA4).
        items: list("MATCH_ITEMS").map(|l| l.iter().fold(0u64, |m, &k| m | 1 << k)),
        // MATCH_HUMAN_HANDS=1: Master Hand and Crazy Hand as humans, not CPUs. Their code reads
        // their inputs from ports 3 and 4 (the monkey's there).
        human_hands: std::env::var_os("MATCH_HUMAN_HANDS").is_some(),
    });
    // The debug VS mode sets its defaults, then loads the announcer's voice clips: its
    // defaults are set by then.
    let entered = Rc::new(Cell::new(false));
    let armed = entered.clone();
    ctx.set_hook(
        ssbm_sdk::sym("onEnterDebugVs"),
        Rc::new(move |_| armed.set(true)),
    );
    let (debug_rng, debug_rules) = (rng.clone(), rules.clone());
    ctx.set_hook(
        ssbm_sdk::sym("gm_LoadAnnouncer"),
        Rc::new(move |ctx| {
            if entered.replace(false) {
                let data = StartMeleeData(At::new(ctx, ssbm_sdk::sym("gmVsMelee_StartData")));
                choose(ctx, &debug_rng, data, &debug_rules, false);
            }
        }),
    );
    // MATCH_VS=1: the VS mode's matches too, as the match scene takes them (fn_8016E730), past
    // the stage select's choice: the debug VS mode has no results to go on to, the VS mode's
    // results screen and records (gm_801623A4) take them from there.
    if std::env::var_os("MATCH_VS").is_some() {
        add_hook(
            ctx,
            "fn_8016E730",
            Rc::new(move |ctx| {
                // Its match state (gmVsMode_State_Vs), not the sudden death that follows a tie.
                let state = ctx.read_u8(ssbm_sdk::sym("state_machine") + 3);
                if i32::from(current_mode(ctx)) == GM_VS && state == 2 {
                    choose(ctx, &rng, StartMeleeData(At::new(ctx, ctx.regs.r(3))), &rules, true);
                }
            }),
        );
    }
}

thread_local! {
    /// The game mode runGameMode last started, when the runner watches it (--mode, MODES).
    static RUNNING: Cell<Option<u8>> = const { Cell::new(None) };
}

/// Notes the mode runGameMode starts (its argument, which --mode changes).
pub fn mode_started(mode: u8) {
    RUNNING.with(|m| m.set(Some(mode)));
}

/// The game mode running (GameModeKind): the one runGameMode started, else the state machine's
/// routing's (it starts with it).
fn current_mode(ctx: &Ctx) -> u8 {
    RUNNING.with(Cell::get).unwrap_or_else(|| ctx.read_u8(ssbm_sdk::sym("state_machine")))
}

/// Runs `hook` on reaching the function `name`, after the hook already there if there is one.
fn add_hook(ctx: &Ctx, name: &str, hook: Hook) {
    let addr = ssbm_sdk::sym(name);
    let before = ctx.hook(addr).map(|(h, _)| h);
    ctx.set_hook(
        addr,
        Rc::new(move |ctx| {
            if let Some(before) = &before {
                before(ctx);
            }
            hook(ctx);
        }),
    );
}

/// MATCH_LOG=1 logs each match as it starts (fn_8016E730): the mode, the stage and each
/// player's fighter, costume, kind (0 human, 1 CPU), CPU kind and level, team and stamina.
pub fn install_match_log(ctx: &Ctx) {
    add_hook(
        ctx,
        "fn_8016E730",
        Rc::new(|ctx| {
            let data = StartMeleeData(At::new(ctx, ctx.regs.r(3)));
            let rules = data.rules();
            let mut players = Vec::new();
            for i in 0..6 {
                let p = data.players().get(i);
                if p.slot_type() == Gm_PKind_NA as u8 {
                    continue;
                }
                players.push(format!(
                    "p{} ckind {} color {} kind {} cpu {}/{} team {} hp {}",
                    i + 1,
                    p.ckind(),
                    p.color(),
                    p.slot_type(),
                    p.cpu_kind(),
                    p.cpu_level(),
                    p.team(),
                    if p.xC_b7() != 0 { p.hp() } else { 0 },
                ));
            }
            let field = ctx.ext::<ssbm_sdk::Sdk>().hw.fields.get();
            eprintln!(
                "field {field}: match in mode {:#04x} on stage {:#x}, kind {}{}: {}",
                current_mode(ctx),
                rules.stkind(),
                rules.match_kind(),
                if rules.is_teams() != 0 { ", teams" } else { "" },
                players.join("; ")
            );
        }),
    );
}

/// Makes the Event mode start at event match `n` (from 0), as if its event select had chosen
/// it: entering the mode loads that event's files, and its character select if it has one. With
/// `fighter` (CKind, costume), the player plays that fighter in that costume, as if the
/// character select had chosen it, and the mode skips the character select (gm_Mode_Event_OnLoad
/// does for the events that set the player's fighter): onEnterVs takes the event data's choice
/// (x2, x3) for a player the event leaves free.
pub fn install_event(ctx: &Ctx, n: u8, fighter: Option<(i8, u8)>) {
    ctx.set_hook(
        ssbm_sdk::sym("gm_Mode_Event_OnLoad"),
        Rc::new(move |ctx| {
            let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
            let event = game.vs().unk_530();
            event.set_unk_535(n);
            if let Some((ckind, color)) = fighter {
                event.set_x2(ckind);
                event.set_x3(color);
                // gm_SetGameModeStateId(1): the mode's match, its state 1. runGameMode has just
                // started the mode at its state 0, the character select.
                let routing = ssbm_sdk::sym("state_machine");
                ctx.write_u8(routing + 3, 1);
                ctx.write_u8(routing + 4, 1);
            }
        }),
    );
}

/// Makes Classic (`gm_Mode_Classic_OnLoad`, `slot` 0) or Adventure (`gm_Mode_Adventure_OnLoad`,
/// `slot` 1) start at stage `n` (from 0) once its character select is done, as continuing from
/// a game over does: the character select's exit goes on to the mode's state `n` << 3, from the
/// stage the mode's VS data keeps (x5 of gmMainLib_8015CDC8's and gmMainLib_8015CDD4's).
pub fn install_stage_start(ctx: &Ctx, on_load: &str, slot: u8, n: u8) {
    ctx.set_hook(
        ssbm_sdk::sym(on_load),
        Rc::new(move |ctx| {
            let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
            let vs = game.vs();
            if slot == 0 { vs.unk_51C() } else { vs.unk_522() }.set_x5(n);
        }),
    );
}

/// Makes the tournament's bracket one of `entrants` (4, 6, 8, 12, 16, 24, 32, 48 or 64), an
/// elimination tournament, whatever its settings screen was left at, as the bracket is drawn up
/// from them (fn_80192938): TmData's entrants is an index into the counts the screen offers
/// (lbl_803D9D20.x0).
pub fn install_tournament(ctx: &Ctx, entrants: u32) {
    const COUNTS: [u32; 9] = [4, 6, 8, 12, 16, 24, 32, 48, 64];
    let index = COUNTS
        .iter()
        .position(|&c| c == entrants)
        .unwrap_or_else(|| panic!("TOU_ENTRANTS: one of {COUNTS:?}"));
    ctx.set_hook(
        ssbm_sdk::sym("fn_80192938"),
        Rc::new(move |ctx| {
            let tm = ssbm_sdk::sym("gm_804771C4");
            // match_type 0, and entrants.
            ctx.write_u32(tm + 4, 0);
            ctx.write_u32(tm + 0xC, index as u32);
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

/// Whenever the main menu comes up: with `unlock`, unlocks every character, stage and trophy,
/// as the save data of the game's debug levels is, so the menus and modes have all of them to
/// show; with `menu` (MenuKind, selection), opens that menu with that item under the cursor, as
/// coming back from one of its modes does, for input to go on from there. Returns what a run
/// that starts in a mode past the main menu applies to the save data there instead
/// (`SaveSetup::apply`).
pub fn install_main_menu(
    ctx: &Ctx,
    unlock: bool,
    menu: Option<(u8, u8)>,
    records: Option<u64>,
    names: usize,
    records_1p: Option<u64>,
    pokes: Vec<Poke>,
) -> Rc<SaveSetup> {
    let setup = Rc::new(SaveSetup {
        unlock,
        records: records.map(Rng::new),
        names,
        records_1p: records_1p.map(Rng::new),
        pokes,
    });
    let on_menu = setup.clone();
    ctx.set_hook(
        ssbm_sdk::sym("mnMain_Scene_OnEnter"),
        Rc::new(move |ctx| {
            on_menu.apply(ctx);
            if let Some((kind, selection)) = menu {
                let data = MenuEnterData(At::new(ctx, ctx.regs.r(3)));
                data.set_menu_kind(kind);
                data.set_hovered_selection(selection);
            }
        }),
    );
    setup
}

/// What UNLOCK_ALL, RECORDS, NAMES, RECORDS_1P and SAVE_POKE change in the save data.
pub struct SaveSetup {
    unlock: bool,
    records: Option<Rng>,
    names: usize,
    records_1p: Option<Rng>,
    pokes: Vec<Poke>,
}

impl SaveSetup {
    pub fn apply(&self, ctx: &Ctx) {
        if let Some(rng) = &self.records {
            fill_records(ctx, rng);
        }
        if self.names > 0 {
            fill_names(ctx, self.names);
        }
        if self.unlock {
            let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
            let save = game.thing().save_data();
            save.set_unlocked_characters(save.unlocked_characters() | 0x7FF);
            save.set_x186A(save.x186A() | 0x7FF);
            save.set_x186C(0xFF);
            // The 300 trophies, a bit each.
            for i in 0..10 {
                save.x1B58().set(i, if i < 9 { u32::MAX } else { 0xFFF });
            }
            // And each of the 293 owned, as the Gallery and Lottery count them (Toy_SetUnlockState's
            // award): the low byte the copies owned, 1 to 3 here, and 0x8000 one not yet
            // looked at, every fourth.
            let owned = save.trophy_flags();
            for i in 0..293 {
                owned.set(i, (1 + i as u16 % 3) | if i % 4 == 0 { 0x8000 } else { 0 });
            }
            save.set_trophy_count(293);
        }
        if let Some(rng) = &self.records_1p {
            fill_records_1p(ctx, rng);
        }
        // Last, so that they have the last word over the others.
        for poke in &self.pokes {
            poke.apply(ctx);
        }
    }
}

/// A write to the game's data at *gmMainLib_804D3EE0 (gmm_x0): the VS rules (x1850, GameRules),
/// then the save data (from 0x1868, GmCardData).
pub enum Poke {
    /// `size` bytes (1, 2, 4 or 8) at `offset`, big-endian.
    At { offset: u32, size: u32, value: u64 },
    /// Every fighter's and name's KO counts and stats (FighterData, NameTagData), the fighter's
    /// play count (x78) and the save's match counts (time_matches to match_resets), each to
    /// `value` or the most its width holds, as the results' saturating adds (fn_80161C90,
    /// gm_80162574) leave them.
    Stats(u64),
}

/// The named fields SAVE_POKE knows: offset in gmm_x0 and size.
const POKE_NAMES: [(&str, u32, u32); 18] = [
    // GameRules at 0x1850, the VS rules screen's.
    ("mode", 0x1852, 1),             // 0 time, 1 stock, 2 coin, 3 bonus
    ("time_limit", 0x1853, 1),       // minutes, 0 none
    ("stock_count", 0x1854, 1),
    ("handicap", 0x1855, 1),         // 0 off, 1 auto, 2 on
    ("damage_ratio", 0x1856, 1),     // tenths
    ("stage_sel", 0x1857, 1),        // StageSelectMode: 0 any, 1 random, 2 ordered, 3, 4
    ("stock_time_limit", 0x1858, 1), // minutes, 0 none
    ("friendly_fire", 0x1859, 1),
    ("pause", 0x185A, 1),
    ("score_display", 0x185B, 1),
    ("sd_penalty", 0x185C, 1), // unk_xc (gmMainLib_8015ED30): 0 is -1, 1 is 0, 2 is -2
    // The save data, GmSaveData at 0x1868.
    ("characters", 0x1868, 2), // unlocked_characters
    ("stages", 0x186A, 2),     // x186A, the unlocked stages
    ("features", 0x186C, 1),   // x186C
    ("coins", 0x1A48, 4),      // x1A48 (gmMainLib_8015CCF0), at most 9999 as the game keeps it
    // GamePrefs x1CB0.
    ("item_freq", 0x1CB0, 1),
    ("item_mask", 0x1CB8, 8),
    ("stage_mask", 0x1CC8, 4), // the random stage select's stages
];

impl Poke {
    /// OFF:SIZE:VALUE (OFF and VALUE in hex with 0x, else decimal; VALUE may be negative) or
    /// NAME=VALUE, NAME one of POKE_NAMES or stats (stats=max for the most each field holds).
    pub fn parse(spec: &str) -> Self {
        let num = |v: &str| -> u64 {
            let v = v.trim();
            let (neg, v) = v.strip_prefix('-').map_or((false, v), |v| (true, v));
            let n = match v.strip_prefix("0x") {
                Some(h) => u64::from_str_radix(h, 16),
                None => v.parse(),
            }
            .unwrap_or_else(|_| panic!("SAVE_POKE: {spec}"));
            if neg { n.wrapping_neg() } else { n }
        };
        if let Some((name, value)) = spec.split_once('=') {
            let name = name.trim();
            if name == "stats" {
                return Poke::Stats(if value.trim() == "max" { u64::MAX } else { num(value) });
            }
            let &(_, offset, size) = POKE_NAMES
                .iter()
                .find(|(n, ..)| *n == name)
                .unwrap_or_else(|| panic!("SAVE_POKE: no field {name}"));
            return Poke::At { offset, size, value: num(value) };
        }
        let parts: Vec<&str> = spec.split(':').collect();
        let [offset, size, value] = parts[..] else {
            panic!("SAVE_POKE=OFF:SIZE:VALUE or NAME=VALUE: {spec}")
        };
        let size = num(size) as u32;
        assert!([1, 2, 4, 8].contains(&size), "SAVE_POKE: size 1, 2, 4 or 8: {spec}");
        Poke::At { offset: num(offset) as u32, size, value: num(value) }
    }

    fn apply(&self, ctx: &Ctx) {
        let base = ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"));
        match *self {
            Poke::At { offset, size, value } => {
                let at = base + offset;
                match size {
                    1 => ctx.write_u8(at, value as u8),
                    2 => ctx.write_u16(at, value as u16),
                    4 => ctx.write_u32(at, value as u32),
                    _ => {
                        ctx.write_u32(at, (value >> 32) as u32);
                        ctx.write_u32(at + 4, value as u32);
                    }
                }
            }
            Poke::Stats(value) => {
                let (w16, w32) = (value.min(0xFFFF) as u16, value.min(0xFFFF_FFFF) as u32);
                let stats = |st: GmStats| {
                    st.set_sd_count(w16);
                    st.set_attacks_hit(w32);
                    st.set_attacks_total(w32);
                    st.set_damage_dealt(w32 as i32);
                    st.set_damage_taken(w32 as i32);
                    st.set_damage_recovered(w32 as i32);
                    st.set_peak_damage(w16);
                    st.set_match_count(w16);
                    st.set_victories(w16);
                    st.set_losses(w16);
                    st.set_play_time(w32);
                    st.set_total_player_count(w32);
                    st.set_walk_distance(w32 as i32);
                    st.set_run_distance(w32 as i32);
                    st.set_fall_distance(w32 as i32);
                    st.set_peak_height(w32 as i32);
                    st.set_coins_collected(w32 as i32);
                    st.set_coins_swiped(w32 as i32);
                    st.set_coins_lost(w32 as i32);
                };
                let game = gmm_x0(At::new(ctx, base));
                let save = game.thing().save_data();
                for i in 0..25 {
                    let fd = save.x1F2C().get(i);
                    for j in 0..25 {
                        fd.fighter_kos().set(j, w16);
                    }
                    stats(fd.stats());
                    // x78 and x79 are a u16 count of the fighter's VS matches.
                    ctx.write_u16(fd.0.addr + 0x78, w16);
                }
                let banks = game.thing().nametag_banks();
                for i in 0..120 {
                    let name = banks.get(i / 19).inner().get(i % 19);
                    for j in 0..120 {
                        name.vs_kos().set(j, w16);
                    }
                    stats(name.stats());
                    for j in 0..25 {
                        name.play_time_by_fighter().set(j, w32);
                    }
                }
                // time_matches, stock_matches, coin_matches, bonus_matches, stamina_matches and
                // match_resets.
                for k in 0..6 {
                    ctx.write_u32(save.0.addr + 0x1B0 + 4 * k, w32);
                }
            }
        }
    }
}

/// Enters names in the first `n` of the save's 120 name slots (19 to a bank, as
/// GetPersistentNameData finds them), for the screens that list names: two full-width letters
/// each in Shift-JIS, AA, BA, CA, ..., as the name entry screen writes them.
fn fill_names(ctx: &Ctx, n: usize) {
    let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
    let banks = game.thing().nametag_banks();
    for i in 0..n.min(120) as i32 {
        let name = banks.get(i / 19).inner().get(i % 19).namedata();
        let bytes = [0x82, 0x60 + (i % 26) as u8, 0x82, 0x60 + (i / 26) as u8, 0, 0, 0, 0];
        for (j, b) in bytes.into_iter().enumerate() {
            name.set(j as i32, b as i8);
        }
    }
}

/// Gives each fighter's records random KO counts and stats, a third of them none, for the Data
/// screens to show what a played save does.
fn fill_records(ctx: &Ctx, rng: &Rng) {
    let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
    let fighters = game.thing().save_data().x1F2C();
    for i in 0..25 {
        if rng.chance(33) {
            continue;
        }
        let fd = fighters.get(i);
        for j in 0..25 {
            fd.fighter_kos().set(j, rng.below(400) as u16);
        }
        let st = fd.stats();
        let total = rng.below(100_000) as u32;
        st.set_attacks_total(total);
        st.set_attacks_hit(rng.below(u64::from(total) + 1) as u32);
        st.set_sd_count(rng.below(500) as u16);
        st.set_damage_dealt(rng.below(1_000_000) as i32);
        st.set_damage_taken(rng.below(1_000_000) as i32);
        st.set_damage_recovered(rng.below(10_000) as i32);
        st.set_peak_damage(rng.below(999) as u16);
        let matches = rng.below(5000) as u16;
        st.set_match_count(matches);
        let won = rng.below(u64::from(matches) + 1) as u16;
        st.set_victories(won);
        st.set_losses(matches - won);
        st.set_play_time(rng.below(10_000_000) as u32);
        st.set_total_player_count(rng.below(20_000) as u32);
        st.set_walk_distance(rng.below(10_000_000) as i32);
        st.set_run_distance(rng.below(10_000_000) as i32);
        st.set_fall_distance(rng.below(10_000_000) as i32);
        st.set_peak_height(rng.below(100_000) as i32);
        st.set_coins_collected(rng.below(10_000) as i32);
        st.set_coins_swiped(rng.below(10_000) as i32);
        st.set_coins_lost(rng.below(10_000) as i32);
    }
}

/// Gives each fighter's 1P records (FighterData x7C) random values, a third of them none: the
/// cleared flags (b0 to b6: Target Test, 10-Man and 100-Man Melee, the 3-Minute Melee, then
/// Classic, Adventure and All-Star), the difficulties and stocks they were cleared on, the
/// Home-Run Contest distance (x7E), the Classic, Adventure and All-Star scores (x88 to x90),
/// the Target Test and Multi-Man times (x94 to x9C) and KO counts (xA0 to xA8); and the save's
/// masks of the fighters that cleared them (gmMainLib_8015ED98's), which go with the flags.
fn fill_records_1p(ctx: &Ctx, rng: &Rng) {
    let game = gmm_x0(At::new(ctx, ctx.read_u32(ssbm_sdk::sym("gmMainLib_804D3EE0"))));
    let save = game.thing().save_data();
    let masks = save.unk_8();
    let mut cleared = [0i32; 7];
    for i in 0..25 {
        if rng.chance(33) {
            continue;
        }
        let r = save.x1F2C().get(i).x7C();
        for k in 0..7 {
            let on = u16::from(rng.chance(60));
            match k {
                0 => r.set_b0(on),
                1 => r.set_b1(on),
                2 => r.set_b2(on),
                3 => r.set_b3(on),
                4 => r.set_b4(on),
                5 => r.set_b5(on),
                _ => r.set_b6(on),
            }
            cleared[k] |= i32::from(on) << i;
        }
        r.set_b789(rng.below(8) as u16);
        r.set_b10_to_12(rng.below(8) as u16);
        r.set_b13_to_15(rng.below(8) as u16);
        r.set_x7E(rng.below(0x10000) as u16);
        r.set_x80(rng.below(5) as u8);
        r.set_x81(rng.below(5) as u8);
        r.set_x82(rng.below(5) as u8);
        r.set_x84(rng.below(1_000_000) as i32);
        r.set_x88(rng.below(10_000_000) as i32);
        r.set_x8C(rng.below(10_000_000) as i32);
        r.set_x90(rng.below(10_000_000) as i32);
        // Times in frames, up to an hour.
        r.set_x94(rng.below(216_000) as u32);
        r.set_x98(rng.below(216_000) as i32);
        r.set_x9C(rng.below(216_000) as i32);
        r.set_xA0(rng.below(1000) as u16);
        r.set_xA2(rng.below(1000) as u16);
        r.set_xA4(rng.below(10_000) as i32);
        r.set_xA8(rng.below(1000) as i32);
    }
    // gmm_retval_ED98: xC to x1C, the masks gmMainLib_8015D00C, 8015D134, 8015D25C, 8015D384
    // and the 8015D4A8 test keep.
    masks.set_x10(masks.x10() | cleared[4]);
    masks.set_x14(masks.x14() | cleared[5]);
    masks.set_x18(masks.x18() | cleared[6]);
    masks.set_x1C(masks.x1C() | cleared[0]);
}

/// Makes every match's human players level 9 CPUs, so the modes that end when the player
/// loses go on further than random play takes them. With MATCH_CPU_KIND, each gets a CPU kind
/// drawn from its list (from `seed`) rather than the one the mode gave its player.
pub fn install_cpu_players(ctx: &Ctx, seed: u64) {
    let kinds = cpu_kinds().map(|k| (k, Rng::new(seed)));
    add_hook(
        ctx,
        "fn_8016E730",
        Rc::new(move |ctx| {
            let data = StartMeleeData(At::new(ctx, ctx.regs.r(3)));
            for i in 0..4 {
                let p = data.players().get(i);
                if p.slot_type() == Gm_PKind_Human as u8 {
                    p.set_slot_type(Gm_PKind_Cpu as u8);
                    p.set_cpu_level(9);
                    if let Some((kinds, rng)) = &kinds {
                        p.set_cpu_kind(kinds[rng.below(kinds.len() as u64) as usize]);
                    }
                }
            }
        }),
    );
}

/// Whether `fighter` (CKind) is Master Hand or Crazy Hand.
fn is_hand(fighter: usize) -> bool {
    [CKind_MasterH, CKind_CrezyH].contains(&(fighter as i32))
}

/// The fighters of `ports` ports, drawn from `fighters`, as the game has the hands: Master Hand
/// once at most, Crazy Hand once at most and only with Master Hand (Classic's last stage brings
/// both), and at least one other fighter for them to face.
fn hands_lineup(rng: &Rng, fighters: &[i32], ports: usize) -> Vec<usize> {
    let others: Vec<usize> =
        fighters.iter().map(|&f| f as usize).filter(|&f| !is_hand(f)).collect();
    assert!(!others.is_empty(), "MATCH_FIGHTERS: the hands need another fighter to face");
    let other = || others[rng.below(others.len() as u64) as usize];
    let mut lineup: Vec<usize> =
        (0..ports).map(|_| fighters[rng.below(fighters.len() as u64) as usize] as usize).collect();
    for i in 0..ports {
        if is_hand(lineup[i]) && lineup[..i].contains(&lineup[i]) {
            lineup[i] = other();
        }
    }
    let (master, crazy) = (CKind_MasterH as usize, CKind_CrezyH as usize);
    if let Some(i) = lineup.iter().position(|&f| f == crazy)
        && !lineup.contains(&master)
    {
        lineup[i] = master;
    }
    if lineup.iter().all(|&f| is_hand(f)) {
        let last = lineup.len() - 1;
        lineup[last] = other();
    }
    lineup
}

/// Sets up a match: with `keep`, of the VS mode, its players' fighters and its stage stay the
/// character and stage selects' (the files the VS mode preloads for them are all the match has
/// room for: other fighters run the heap out), and the players it has stay its only ones.
fn choose(ctx: &Ctx, rng: &Rng, data: StartMeleeData, chosen_rules: &Rules, keep: bool) {
    let Rules { stages, fighters, time, .. } = chosen_rules;
    let time = *time;
    let rules = data.rules();
    let stage = if keep {
        i32::from(rules.stkind())
    } else {
        let stage = stages[rng.below(stages.len() as u64) as usize];
        rules.set_stkind(stage as u16);
        stage
    };
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
    if let Some(mask) = chosen_rules.items {
        rules.set_x20(mask);
        if rules.item_freq() < 0 {
            rules.set_item_freq(3);
        }
    }
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
    // A match drawn with the hands in it gets its line-up from hands_lineup, and is a time match
    // without teams or one-life stamina for the others: the hands aim at the nearest opponent
    // (ftLib_FindNearestOpponent) and fault when no fighter on another team is left.
    let lineup = (!keep && fighters.iter().any(|&f| is_hand(f as usize)))
        .then(|| hands_lineup(rng, fighters, chosen_rules.players.unwrap_or(4).min(4)));
    if lineup.is_some() {
        rules.set_match_kind(MatchKind_Time as u32);
        rules.set_is_stock(0);
        rules.set_is_teams(0);
    }
    let mut chosen = Vec::new();
    let mut playing = Vec::new();
    for i in 0..4 {
        let p = data.players().get(i);
        if keep && p.slot_type() == Gm_PKind_NA as u8 {
            continue;
        }
        if !keep && chosen_rules.players.is_some_and(|n| i as usize >= n) {
            p.set_slot_type(Gm_PKind_NA as u8);
            continue;
        }
        playing.push(p);
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
        let fighter = if keep {
            p.ckind() as usize
        } else if let Some(lineup) = &lineup {
            lineup[i as usize]
        } else {
            fighters[rng.below(fighters.len() as u64) as usize] as usize
        };
        // Bosses and the other special fighters only play as CPUs, as in the modes that have
        // them: the game crashes with some of them under a player's control. MATCH_HUMAN_HANDS
        // makes an exception of the hands.
        let hand = is_hand(fighter);
        let cpu = if hand && chosen_rules.human_hands {
            false
        } else {
            fighter >= PLAYABLE || rng.chance(50)
        };
        if !keep {
            p.set_ckind(fighter as i8);
        }
        p.set_slot_type(if cpu { Gm_PKind_Cpu } else { Gm_PKind_Human } as u8);
        p.set_cpu_level(if cpu { 1 + rng.below(9) as u8 } else { 0 });
        if !keep {
            p.set_color(0);
        }
        // The special fighters start standing, as the modes that have them start them (the
        // entry animation they lack faults), and the hands with the stamina Classic gives them.
        if fighter >= PLAYABLE {
            p.set_xD_b2(1);
        }
        if hand {
            p.set_xC_b7(1);
            p.set_hp(300);
            p.set_xD_b0(1);
            p.set_spawn_dir(-1);
        }
        if fighter as i32 == CKind_GKoops {
            p.set_xC_b1(0);
        }
        if cpu && let Some(kinds) = &chosen_rules.cpu_kinds {
            p.set_cpu_kind(kinds[rng.below(kinds.len() as u64) as usize]);
        }
        if let Some(hp) = chosen_rules.stamina
            && lineup.is_none()
        {
            p.set_xC_b7(1);
            p.set_hp(hp);
            p.set_stocks(1);
        }
        if chosen_rules.teams && !varied && lineup.is_none() {
            p.set_team(rng.below(3) as u8);
        }
        chosen.push(format!("{}{}", FIGHTERS[fighter], if cpu { " (CPU)" } else { "" }));
    }
    if chosen_rules.stamina.is_some() && lineup.is_none() {
        // A stock match, as the Stamina mode's (gm_801B931C), on the time limit still.
        rules.set_match_kind(MatchKind_Stock as u32);
    }
    if chosen_rules.teams && lineup.is_none() {
        rules.set_is_teams(1);
        // Not all on one team, or the match is over as it starts.
        if let [first, .., last] = playing[..]
            && playing.iter().all(|p| p.team() == first.team())
        {
            last.set_team((last.team() + 1) % 3);
        }
    }
    let field = ctx.ext::<ssbm_sdk::Sdk>().hw.fields.get();
    eprintln!("field {field}: match on stage {stage}: {}", chosen.join(", "));
}
