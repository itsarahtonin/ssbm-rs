"""Writes jobs for the recipes mkjobs-recipes-menus.py and mkjobs-recipes-matches.py left, from
recipes.md and the per-unit notes (local/lockstep/notes-gr3): the trophy fall and the ending's
other trophies in each 1P mode's game over (--mode records the mode as current since 70b4c01, so
their mode arms run), Classic's intro stages, Adventure's and All-Star's match-end bonuses, the
team stock steal and sudden death (gmvs, gmvsmelee), the tournament setup left to the monkey
(gmtou_0), the VS rules menu, the rumble settings, the sound test, Camera Mode, the progressive
scan prompt, R held at boot at each debug level and Adventure's first route run through.

Each job checks its target functions on every call (LOCKSTEP_UNLIMITED) and scripts its way in
with MONKEY_HOLD from boot; after its script the monkey plays on. Each job gets its own copy of
the card.

    python tools/lockstep/campaign/mkjobs-recipes-more.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds template-saved.raw (a card with Melee's save). The scripts' fields were read from
short runs with CALLS (tools/run/src/main.rs): a --mode run's Start at 100 enters the mode (its
character select at 104), and from the main menu (303) A opens MENU's item at 502 (the rumble
settings at 501); the game overs' trophy fall starts at 35, the sound test's sounds play from 28
and the progressive scan prompt asks at 26.
"""
import os
import shutil
import sys

PRESS = 4


def steps(start, step, presses):
    """Presses one after another, `step` fields apart from `start`; "wait" skips a step."""
    out = []
    for i, b in enumerate(presses):
        if b != "wait":
            out.append((b, start + i * step))
    return out


def hold(presses):
    """MONKEY_HOLD for the presses: each (holds, field[, fields]) holds its comma-joined holds
    (one per port) that many fields (PRESS), the ports none names nothing, and nothing between."""
    holds, field = [], 0
    for p in sorted(presses, key=lambda p: p[1]):
        names, at = p[0], p[1]
        length = p[2] if len(p) > 2 else PRESS
        if at > field:
            holds.append(f"none@{field}-{at - 1}")
        holds += [f"{h}@{at}-{at + length - 1}" for h in names.split(",")]
        field = at + length
    end = field + 300
    holds.append(f"none@{field}-{end}")
    return ",".join(holds), end


# A --mode run: Start at the title enters the mode; its character select (104) as
# mkjobs-levers.py scripts it, port 1 or both ports moving up onto a portrait, picking, starting.
TITLE = [("start", 100)]
ONE_P_CSS = TITLE + [("sy=127", 160, 25), ("a", 190), ("start", 230)]
VS_CSS = TITLE + [("sy=127,p2+sy=127", 160, 25), ("a,p2+a", 190), ("start", 230)]
# From the main menu: Start past the movie, Start at the title, A on MENU's item (open at 502).
MENU_A = [("start", 100), ("start", 300), ("a", 500)]

# The game overs' trophy fall (gmregtyfall) and the ending's other trophies (gmregenddisp).
GOVER_FNS = ("gm_Scene_ToyFall_OnEnter,gm_801A6EE4,gm_801A659C,gm_801A68D8,fn_801A94BC,"
             "fn_801A7FB4,gm_801A9094,gm_801A8D54,gm_801A8114")
# Classic's stage intros (gm_1832).
CLASSIC_FNS = ("fn_80184AB8,fn_801852FC,fn_80186634,fn_80186080,fn_801851C0,fn_8018325C,"
               "fn_80185A0C,fn_801857C4,fn_80184138,fn_80186400")
# Adventure's stage table and its match-end bonuses (gm_17C0, gm_17E4); All-Star's (gm_18A1).
ADVENTURE_FNS = "gm_8017C838,gm_8017CBAC,fn_8017E8A4"
ALLSTAR_FNS = "fn_8018A364"
# The VS scene's stock steal, team outcome and match end (gmvs), the VS mode's exits (gmvsmelee).
VS_FNS = ("fn_8016B918,gm_GetTeamBattleOutcome,fn_8016D634,fn_8016E2BC,gm_8016EDDC,"
          "gmVsMelee_ExitResults,findSmallestLoser,gmVsMelee_WasAnyPlayerHuman,gmVsMelee_ExitVs,"
          "gmVsMelee_EnterVs")
# The tournament's setup and bracket screens (gmtou_0).
TOU_FNS = ("fn_80193FCC,fn_80192758,fn_80194F30,fn_80192BB0,fn_80195CCC,fn_801953C8,"
           "fn_80193B58,fn_80191B5C,fn_80192938,gm_80190EA4")
RULES_FNS = "fn_802309F0,mn_80230E38,fn_8022F538,mn_80230D18,mn_8022FEC8"
RUMBLE_FNS = ("mnVibration_HandleInput,mnVibration_Think,mnVibration_IntroProc,"
              "mnVibration_RefreshNameRows,mnVibration_OnAnimComplete,mnVibration_CreatePortPanels")
SOUND_FNS = ("fn_800269AC,lbAudioAx_80023B24,fn_800262A0,fn_800256BC,lbAudioAx_80027648,"
             "lbAudioAx_80024DC4,lbAudioAx_800233EC,fn_800253D8")
KINOKO_FNS = "grKinokoRoute_80207C88,grKinokoRoute_8020836C"

# The VS rules (MENU=2,3): on each of the first five rows left and right, then the Additional
# Rules (the seventh row) and the same on its rows.
ROW = ["left", "left", "right", "right", "right", "down"]
RULES = MENU_A + steps(600, 20, ROW * 6 + ["a", "wait"] + ROW * 6 + ["b", "b"])
# The rumble settings (MENU=4,0, open at 501): each port's row toggled, then out and back.
RUMBLE = MENU_A + steps(600, 20, (["right", "a", "left", "a", "down"] * 4) + ["up"] * 4
                        + ["b", "wait", "a"] + ["right", "a", "down"] * 4)
# Adventure's first route: the fighter runs right for a minute, jumping every 200 fields (a
# later hold on a port replaces an earlier one, so the jumps hold the stick too).
RUN_RIGHT = ONE_P_CSS + [p for k in range(18) for p in [("sx=127", 700 + 200 * k, 196),
                                                        ("sx=127+y", 896 + 200 * k)]]

# name, environment (None drops a base setting), card, --mode, presses, fields
JOBS = [
    # The trophy fall with every trophy (UNLOCK_ALL: the other trophies' display at its most)
    # and with the save's own, in each game over mode, the debug one included.
    (f"gover{mode}{tag}", {"LOCKSTEP_UNLIMITED": GOVER_FNS, **more}, "template-saved", mode,
     [], 6000)
    for mode in ("15", "16", "17", "1a")
    for tag, more in [("", {}), ("saved", {"UNLOCK_ALL": None})]
] + [
    # Classic from the stages whose intros are unverified (4, 7, 8, 9; 2, 5 and 7 in Japanese).
    (f"classic{n}{tag}", {"LOCKSTEP_UNLIMITED": CLASSIC_FNS, "CLASSIC_STAGE": n, "CPU_PLAYERS": 1,
                          **more}, "template-saved", "03", ONE_P_CSS, 8000)
    for n, tag, more in [(4, "", {}), (7, "", {}), (8, "", {}), (9, "", {}),
                         (2, "jp", {"GAME_LANGUAGE": "jp"}), (5, "jp", {"GAME_LANGUAGE": "jp"}),
                         (7, "jp", {"GAME_LANGUAGE": "jp"})]
] + [
    # Adventure from the stages mkjobs-levers.py's jobs left out, played by a CPU to each match's
    # end; All-Star played by a CPU as far as it gets.
    (f"adventure{n}", {"LOCKSTEP_UNLIMITED": ADVENTURE_FNS, "ADVENTURE_STAGE": n,
                       "CPU_PLAYERS": 1}, "template-saved", "04", ONE_P_CSS, 10000)
    for n in (1, 3, 5, 7, 8, 11, 12)
] + [
    (f"allstar{i}", {"LOCKSTEP_UNLIMITED": ALLSTAR_FNS, "CPU_PLAYERS": 1}, "template-saved", "05",
     ONE_P_CSS, 30000)
    for i in range(2)
] + [
    # Team matches in the debug VS mode with the rules varied (stock among them): a human out of
    # stocks pressing Start takes one from a teammate (fn_8016B918), at debug level 3 by its other
    # button test.
    (f"steal{i}{tag}", {"LOCKSTEP_UNLIMITED": VS_FNS, "MATCH_TEAMS": 1, "MATCH_RULES": 1,
                        "MATCH_TIME": 120, **more}, None, "e", [], 20000)
    for i, tag, more in [(0, "", {}), (1, "", {}), (2, "", {}), (0, "db3", {"DBLEVEL": 3})]
] + [
    # VS matches of 15 seconds with both ports idle: no KO, a tie, sudden death (fn_8016D634's
    # second match end) and the VS mode's exits.
    (f"vssd{i}", {"LOCKSTEP_UNLIMITED": VS_FNS, "MATCH_VS": 1, "MATCH_TIME": 15,
                  "SAVE_POKE": "stage_sel=1"}, "template-saved", "02",
     VS_CSS + [("none", 240, 2400)], 6000)
    for i in range(3)
] + [
    # The tournament's setup left to the monkey (its rows, match type, entrants, handicaps,
    # names), with one controller (PAD_UNPLUG) and with saved names for the bracket's rows.
    (f"tou{tag}", {"LOCKSTEP_UNLIMITED": TOU_FNS, "MENU": "2,1", **more}, "template-saved", None,
     MENU_A, 20000)
    for tag, more in [("free", {}), ("free2", {"SAVE_POKE": "handicap=1"}),
                      ("onepad", {"PAD_UNPLUG": "2,3,4"}), ("names", {"NAMES": 4})]
] + [
    ("rules", {"LOCKSTEP_UNLIMITED": RULES_FNS, "MENU": "2,3"}, "template-saved", None, RULES,
     4000),
    ("rulesjp", {"LOCKSTEP_UNLIMITED": RULES_FNS, "MENU": "2,3", "GAME_LANGUAGE": "jp"},
     "template-saved", None, RULES, 4000),
    ("rumble", {"LOCKSTEP_UNLIMITED": RUMBLE_FNS, "MENU": "4,0"}, "template-saved", None, RUMBLE,
     3000),
    ("rumbleonepad", {"LOCKSTEP_UNLIMITED": RUMBLE_FNS, "MENU": "4,0", "PAD_UNPLUG": "3,4"},
     "template-saved", None, RUMBLE, 3000),
] + [
    # The sound test's sounds and music, left to the monkey.
    (f"soundtest{i}", {"LOCKSTEP_UNLIMITED": SOUND_FNS}, "template-saved", "07", [], 15000)
    for i in range(2)
] + [
    # Camera Mode's snapshots of a match, drawn (GX_RENDER) so there is a picture to take.
    (f"camera{i}", {"LOCKSTEP_UNLIMITED": "hsd_803B3CD8,hsd_803B51C8", "GX_RENDER": "-"},
     "template-saved", "0a", [], 12000)
    for i in range(2)
] + [
    # The progressive scan prompt with deflicker off in the save, answered either way.
    (f"pscan{tag}", {"LOCKSTEP_UNLIMITED": "gmMainLib_8015FA34,gmMainLib_8015F600",
                     "SAVE_POKE": "0x1CC5:1:0"}, "template-saved", "27", presses, 1500)
    for tag, presses in [("yes", [("left", 60), ("a", 80)]), ("no", [("right", 60), ("a", 80)])]
] + [
    # R held through boot at each debug level.
    (f"launchr{level}", {"LOCKSTEP_UNLIMITED": "hsd_803931A4", "DBLEVEL": level},
     "template-saved", None, [("r", 0, 60)], 2500)
    for level in (0, 1, 2, 3, 4)
] + [
    # Adventure's first route run through by a human (its phase-2 stop and phase 3).
    ("kinokoroute", {"LOCKSTEP_UNLIMITED": KINOKO_FNS, "ADVENTURE_STAGE": 0}, "template-saved",
     "04", RUN_RIGHT, 8000),
]


def main():
    prefix, cards = sys.argv[1], sys.argv[2]
    for i, (name, more, card, mode, presses, fields) in enumerate(JOBS):
        job = f"{prefix}{name}"
        args = ""
        if card is not None:
            copy = f"{cards}/{job}.raw"
            shutil.copy(os.path.join(cards, f"{card}.raw"), copy)
            args += f" --card {copy}"
        if mode is not None:
            args += f" --mode {mode}"
        # The debug VS mode's matches, and MATCH_VS's, take their seed from --matches.
        if mode == "e" or "MATCH_VS" in more:
            args += f" --matches {6100 + i}"
        env = {"LOCKSTEP_SEED": 2100 + i, "UNLOCK_ALL": 1, "MONKEY_B": 2, "STALL_BEATS": 200,
               "LOCKSTEP_MUTATE": 2}
        if presses:
            env["MONKEY_HOLD"] = hold(presses)[0]
        env.update(more)
        envs = " ".join(f"{k}={v}" for k, v in env.items() if v is not None)
        print(f"{job}|{envs}{args} --monkey {6100 + i} --fields {fields}")


if __name__ == "__main__":
    main()
