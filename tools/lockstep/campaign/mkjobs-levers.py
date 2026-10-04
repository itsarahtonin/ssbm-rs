"""Writes jobs for the runner's levers that the static analyses of the unverified code asked for:
screens and modes reached through crafted save data (SAVE_POKE, RECORDS_1P), the debug menu's
test rows, tournaments of a set size (TOU_ENTRANTS), VS matches on unplugged ports (PAD_UNPLUG),
CPU kinds (MATCH_CPU_KIND) and stamina (MATCH_STAMINA), events played as a chosen fighter
(EVENT_FIGHTER), Classic and Adventure from a chosen stage (CLASSIC_STAGE, ADVENTURE_STAGE) and
disc errors (DVD_FATAL, DVD_COVER). Each job checks its target functions on every call
(LOCKSTEP_UNLIMITED) and scripts its way in with MONKEY_HOLD from boot (Start at the title screen
at field 100, which hands over to --mode, or A at 400 for the main menu's jobs, as
mkjobs-menus.py's); after its script the monkey plays on. Each job gets its own copy of the card.

    python tools/lockstep/campaign/mkjobs-levers.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds template-saved.raw (a card with Melee's save). The scripts' fields were read from
short runs with CALLS (tools/run/src/main.rs) on the functions each one opens.
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


# The title screen's Start, which hands over to --mode.
TITLE = [("start", 100)]

# The VS character select, from the title's Start: ports 1 and 2 move their cursors up onto the
# portraits, pick and start (gmVsMelee_ExitCss at 234). With a stage_sel other than 0 the stage
# select picks for itself (gm_80167FC4) and the match starts at once.
VS_CSS = TITLE + [("sy=127,p2+sy=127", 160, 25), ("a,p2+a", 190), ("start", 230)]
# The 1P modes' character select the same, port 1 alone (gmClassic_801B3E44 at 234).
ONE_P_CSS = TITLE + [("sy=127", 160, 25), ("a", 190), ("start", 230)]

# The debug menu (--mode 6) reads the stick (or X and Y) to move and R to change a value
# (un_80303AC4). Its top rows: Versus Mode, Result Test, ... Mode Team Test (the 9th).
DEBUG_TEAM_TEST = TITLE + steps(300, 20, ["x"] * 8 + ["a"])
# MODE: Kim (the 5th) and its AllStar Enemy row (the 8th): R picks the enemy, A shows the All-Star
# intro with it (gm_Scene_IntroAllstar_OnEnter at 863 for the script below).
def debug_allstar(rights):
    return DEBUG_TEAM_TEST + steps(520, 20, ["x"] * 4 + ["a", "wait", "wait"] + ["x"] * 7
                                   + ["r"] * rights + ["a"])


# MODE: Yoshiki (the 8th): Init, Format, Accessable, Create, Save, Load, Delete and SnapMount0
# (un_80301634 to un_8030191C), each its card tasks, then back out. SnapLoad0 on a card with no
# snapshot fails the game's assertion (lbsnap.c:410), as SnapDelete0 and SnapSwap0 would.
DEBUG_YOSHIKI = DEBUG_TEAM_TEST + steps(520, 20, ["x"] * 7 + ["a"]) + steps(
    720, 90, ["a"] + ["x", "a"] * 7 + ["b", "b", "b"])
# Result Test, the 2nd row, opens its own menu: the 1P to 4P kinds (R picks the fighter), then
# Start on its last row shows the results screen (un_80301C80 sets the mode's state 0xB).
DEBUG_RESULT = TITLE + steps(300, 20, ["x", "a", "wait", "wait"] + ["r"] * 2 + ["x"] + ["r"] * 5
                             + ["x"] + ["r"] * 9 + ["x"] + ["r"] * 13 + ["x"] * 13 + ["start"])
# The title's A, then Tournament Melee from the VS menu MENU=2,1 opens (fn_80192938, the bracket,
# at 836 and its first match at 1131 with the monkey on the setup screen). --mode 1b faults in
# the setup screen (an unmapped read at 0x2, as the f6 and w2 campaigns' mode1b jobs stopped).
TOURNAMENT_MENU = [("a", 400), ("a", 500)]

# The title's A, Records from the Data menu MENU=5,3 opens, then Records' third item, Misc
# (mnCount_Create at 681), and its rows.
MISC_RECORDS = [("a", 400), ("a", 500), ("down", 600), ("down", 640), ("a", 680)] + steps(
    780, 20, ["down"] * 40 + ["up"] * 40 + ["right"] * 5 + ["left"] * 5)

COUNT = {"LOCKSTEP_UNLIMITED": "mnCount_8025035C,mnCount_8025092C,mnCount_8025072C",
         "MENU": "5,3"}
ALLSTAR = {"LOCKSTEP_UNLIMITED": "fn_80186F6C,fn_801874FC,fn_801873F0"}
CARDTASKS = {"LOCKSTEP_UNLIMITED": "taskMount,taskCheck,taskOpen,taskUnk3,taskFormat,taskDelete,"
             "taskRename,taskCreate,taskRead,taskWrite,taskSetStatus,taskReadHeader,"
             "taskListSnapshots,taskFindFile,lbCardNew_DeleteSnap,executeNextTask"}
# The results screen's reads of which ports have controllers (HSD_PadMasterStatus' err).
RESULT_PADS = "fn_80177920,fn_80177DD0,fn_80178050,fn_801791E4,gm_80166A98"
# The results screens and the records they write (gm_801623A4 and the saturating adds).
RESULTS = {"LOCKSTEP_UNLIMITED": "gm_801623A4,fn_80162068,fn_80162170,fn_80161C90,gm_8016247C,"
           "gm_801623FC,gm_80162574," + RESULT_PADS}
TOURNAMENT = {"LOCKSTEP_UNLIMITED": "fn_80192938,gm_801B1810,gm_801B1834,gm_801B1A84,gm_801B1AD4",
              "CPU_PLAYERS": 1, "MENU": "2,1"}
# The CPU's item choices and its routines by kind (ftCo_800B2AFC's switch).
CPU_KIND_FNS = ("ftCo_800AD7FC,ftCo_800ADC28,ftCo_800ACD5C,ftCo_800ADE48,ftCo_800AF290,"
                "ftCo_800AECF0,ftCo_800B00F8,ftCo_800AFC40,ftCo_800B24B8,ftCo_800B126C,"
                "ftCo_800B1478,ftCo_800B17D0,ftCo_800AF78C,ftCo_800AFE3C,ftCo_800B1DA0,"
                "ftCo_800B1EF0,ftCo_800B21C8,ftCo_800B1AB8,ftCo_800B2AFC")
CPU = {"LOCKSTEP_UNLIMITED": CPU_KIND_FNS, "CPU_PLAYERS": 1, "MATCH_TIME": 60}
STAMINA = {"LOCKSTEP_UNLIMITED": "ftCo_Damage_Anim,ftCo_Damage_Coll,ftCo_DamageIce_Anim,"
           "ftCo_DownDamage_Anim,ftCo_DownWait_Anim,ftCo_800C8C84,ftCo_800C8D00,ftCo_8009F0F0,"
           "ftColl_80077C60,ftColl_80078A2C", "MATCH_TIME": 90}
EVENT_FNS = {"LOCKSTEP_UNLIMITED": "gm_801BC00C,fn_801BBFE8,gm_801BAB40"}
CLASSIC_MH = {"LOCKSTEP_UNLIMITED": "fn_8017C1A4,fn_8017C0C8,fn_8017C71C,fn_8017C7A0",
              "CPU_PLAYERS": 1}
ADVENTURE = {"LOCKSTEP_UNLIMITED": "gm_8017C838", "CPU_PLAYERS": 1}
SHRINE = ("grShrineRoute_OnInit,grShrineRoute_OnLoad,grShrineRoute_OnStart,"
          "grShrineRoute_802089E8,fn_80208A38,grShrineRoute_80208D14,grShrineRoute_80208F70,"
          "grShrineRoute_80209AF0,grShrineRoute_80209BEC,grShrineRoute_8020A104,"
          "grShrineRoute_8020A21C,grShrineRoute_8020A8A4,grShrineRoute_8020AA40,"
          "grShrineRoute_8020AB58,grShrineRoute_8020AC44,grShrineRoute_8020AD24,"
          "grShrineRoute_8020AE08,grShrineRoute_8020AF38,grShrineRoute_8020B020,"
          "grShrineRoute_8020B0AC,grShrineRoute_OnTouchLine")
DVD = {"LOCKSTEP_UNLIMITED": "CategorizeError,cbForStateGettingError,cbForStateError,stateReady,"
       "stateGettingError,cbForUnrecoveredError,cbForUnrecoveredErrorRetry,stateGoToRetry,"
       "cbForStateGoToRetry,stateCoverClosed,cbForStateCoverClosed,stateMotorStopped,"
       "cbForStateMotorStopped,stateCheckID,cbForStateCheckID1,cbForStateCheckID2,"
       "cbForStateCheckID3,stateTimeout,stateBusy,cbForStateBusy,DVDCancel"}

# name, environment, card ("template-saved", or None for none), --mode, presses, fields
JOBS = [
    # Records, Misc: the counts' rows and their scrolling (mnCount never ran).
    ("count", {**COUNT, "RECORDS": 7}, "template-saved", "01", MISC_RECORDS, 4000),
    ("countmax", {**COUNT, "RECORDS": 8, "SAVE_POKE": "stats=max,coins=9999"}, "template-saved",
     "01", MISC_RECORDS, 4000),
    ("count1p", {**COUNT, "RECORDS": 9, "RECORDS_1P": 9, "NAMES": 30}, "template-saved", "01",
     MISC_RECORDS, 4000),
    # The debug menu's All-Star intro with enemies from the list's start, Ice Climbers (two
    # entities) and its end.
] + [
    (f"dbgallstar{r}", ALLSTAR, "template-saved", "06", debug_allstar(r), 2500)
    for r in (0, 4, 14, 20, 33)
] + [
    # MODE: Yoshiki's card tasks, on a card and with it pulled between them.
    ("dbgyoshiki", CARDTASKS, "template-saved", "06", DEBUG_YOSHIKI, 3500),
    ("dbgyoshikipull", {**CARDTASKS, "CARD_REMOVE": "1300,1700"}, "template-saved", "06",
     DEBUG_YOSHIKI, 3500),
    # Result Test with controllers missing.
] + [
    (f"dbgresult{''.join(p.split(','))}", {"LOCKSTEP_UNLIMITED": RESULT_PADS, "PAD_UNPLUG": p},
     "template-saved", "06",
     DEBUG_RESULT, 3000) for p in ("2,3,4", "3,4", "4", "2")
] + [
    # Tournaments of each size, CPUs all, under rules the VS rules screen keeps.
    (f"tou{n}{tag}", {**TOURNAMENT, "TOU_ENTRANTS": n, "SAVE_POKE": poke}, "template-saved", "01",
     TOURNAMENT_MENU, 40000)
    for n, tag, poke in [
        (6, "time", "mode=0,time_limit=0,pause=0"),
        (12, "stock", "mode=1,stock_time_limit=0,stock_count=1,pause=0"),
        (24, "sd", "mode=0,time_limit=1,sd_penalty=2,pause=0"),
        (48, "stocksd", "mode=1,stock_time_limit=0,stock_count=1,sd_penalty=2,pause=0"),
        (6, "stocksd", "mode=1,stock_time_limit=2,stock_count=2,sd_penalty=2,pause=0"),
        (12, "timesd", "mode=0,time_limit=2,sd_penalty=2,pause=0"),
    ]
] + [
    # VS matches through the VS mode's results and records, from saves the VS rules and
    # options screens can't make, on ports with and without controllers.
    # pause=0: the monkey's Start would keep the game paused.
    (f"vs{name}", {**RESULTS, "SAVE_POKE": f"{poke},pause=0", **more}, "template-saved", "02",
     VS_CSS, 12000)
    for name, poke, more in [
        ("sel2", "stage_sel=2,time_limit=1", {"PAD_UNPLUG": "3,4"}),
        ("sel3", "stage_sel=3,time_limit=1", {"PAD_UNPLUG": "4"}),
        ("sel4", "stage_sel=4,time_limit=1", {}),
        ("handicap", "handicap=1,stage_sel=1,time_limit=1", {"PAD_UNPLUG": "3,4"}),
        ("coins", "mode=2,coins=9998,stage_sel=1,time_limit=1", {}),
        ("stats", "stats=max,stage_sel=1,time_limit=1,score_display=1", {"PAD_UNPLUG": "4"}),
        ("onestage", "stage_mask=0x4,stage_sel=1,time_limit=1", {}),
        ("nostage", "stage_mask=0,stage_sel=1,time_limit=1", {"PAD_UNPLUG": "3,4"}),
        # The VS mode's matches with the debug VS's CPUs, teams and stamina (MATCH_VS).
        ("cpumix", "stage_sel=1", {"MATCH_VS": 1, "MATCH_TIME": 40, "MATCH_TEAMS": 1}),
        ("stamina", "stage_sel=1", {"MATCH_VS": 1, "MATCH_TIME": 60, "MATCH_STAMINA": 40}),
    ]
] + [
    # CPU kinds in debug VS matches, items mostly high, every player a CPU.
    (f"cpukind{i}", {**CPU, "MATCH_CPU_KIND": kinds}, None, "e", [], 20000)
    for i, kinds in enumerate(["7,8,9,10", "11,12,13,14", "15,16,17,18", "19,20,21,22",
                               "23,24,25", "26,27,28,29", "7,10,13,18,23,27"])
] + [
    # Stamina matches.
    (f"stamina{hp}", {**STAMINA, "MATCH_STAMINA": hp, "CPU_PLAYERS": 1, "MATCH_RULES": 1}, None,
     "e", [], 15000) for hp in (20, 150, 999)
] + [
    # Events, with and without CPUs for the player.
    (f"event{n}{'cpu' if cpu else ''}", {**EVENT_FNS, "EVENT": n, **({"CPU_PLAYERS": 1} if cpu
                                                                    else {})},
     "template-saved", "2b", TITLE, 12000)
    for n in (8, 20, 21, 35, 43, 47, 49) for cpu in (False, True)
] + [
    # Events played as the fighter and costume of one of their opponents: the costume clash
    # (gm_801BC00C).
    (f"eventclash{n}", {**EVENT_FNS, "EVENT": n, "EVENT_FIGHTER": f, "CPU_PLAYERS": 1},
     "template-saved", "2b", TITLE, 8000) for n, f in [
        # Read from MATCH_LOG runs (EVENT_FIGHTER=0,0): event, an opponent's CKIND,COLOR.
        (8, "18,0"), (20, "14,0"), (43, "18,0"), (47, "24,0"), (5, "4,0"), (2, "6,1"),
        (6, "13,1"),
    ]
] + [
    # Classic from Master Hand's stage; Adventure from the stages gm_8017C838 loads extra
    # fighters for (Mushroom Kingdom's Yoshis, the Underground Maze's Link, Green Greens'
    # Kirbys, Pokemon Stadium, Icicle Mountain, Battlefield's wireframes).
    ("classicmh", {**CLASSIC_MH, "CLASSIC_STAGE": 10}, "template-saved", "03", ONE_P_CSS, 8000),
] + [
    (f"adventure{n}", {**ADVENTURE, "ADVENTURE_STAGE": n}, "template-saved", "04", ONE_P_CSS,
     8000) for n in (0, 4, 6, 9, 10)
] + [
    ("shrineroute", {"LOCKSTEP_UNLIMITED": SHRINE + ",gm_8017C838", "ADVENTURE_STAGE": 2,
                     "CPU_PLAYERS": 1}, "template-saved", "04", ONE_P_CSS, 10000),
    ("shrineroutehuman", {"LOCKSTEP_UNLIMITED": SHRINE, "ADVENTURE_STAGE": 2}, "template-saved",
     "04", ONE_P_CSS, 10000),
] + [
    # Disc errors while the game reads: the boot's loads, and the main menu's after the title.
    # (CategorizeError at 21 and 103; the cover's close reaches stateCheckID 70 fields later.)
    # A fatal error stops the game on its error screen.
    (f"dvd{name}", {**DVD, **more}, "template-saved", None, TITLE, fields)
    for name, more, fields in [
        ("fatal20", {"DVD_FATAL": 20}, 1000),
        ("fatal101", {"DVD_FATAL": 101}, 1000),
        ("cover20", {"DVD_COVER": "20,200"}, 3000),
        ("cover101", {"DVD_COVER": "101,400"}, 3000),
        ("cover102long", {"DVD_COVER": "102,1200"}, 3000),
    ]
]


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


def main():
    prefix, cards = sys.argv[1], sys.argv[2]
    for i, (name, more, card, mode, presses, fields) in enumerate(JOBS):
        job = f"{prefix}{name}"
        args = ""
        if card is not None:
            copy = os.path.join(cards, f"{job}.raw")
            shutil.copy(os.path.join(cards, f"{card}.raw"), copy)
            args += f" --card {copy}"
        if mode is not None:
            args += f" --mode {mode}"
        # The debug VS mode's matches, and MATCH_VS's, take their seed from --matches.
        if mode == "e" or "MATCH_VS" in more:
            args += f" --matches {4900 + i}"
        env = {"LOCKSTEP_SEED": 900 + i, "UNLOCK_ALL": 1, "MONKEY_B": 2, "STALL_BEATS": 200,
               "LOCKSTEP_MUTATE": 2}
        if presses:
            env["MONKEY_HOLD"] = hold(presses)[0]
        env.update(more)
        envs = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"{job}|{envs}{args} --monkey {4900 + i} --fields {fields}")


if __name__ == "__main__":
    main()
