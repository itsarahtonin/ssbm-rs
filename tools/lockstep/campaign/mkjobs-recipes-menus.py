"""Writes jobs for the menu and game-mode recipes the static analyses of the unverified code left
(local/lockstep/recipes.md and its per-unit notes): Name Entry and the new-name keyboard, the
name delete's nametag bookkeeping (gmMainLib_8015DBF4), the Records screens' rankings and stat
sheets, the Event Match list at each cleared-event count, the character selects (Training, two
ports on one costume, port 2's 1P select, Japanese, 1P records), Multi-Man Melee, the title's
attract demo, the Classic game over and continue, a VS match cancelled from the pause, the
tournament's match-up screens, the lottery, the item and random stage switches, the launch
buttons held at boot at each debug level and the debug menu's records and ending tests. What
mkjobs-levers.py already covers (the counts screen, All-Star intro, Yoshiki and Result Test,
tournament sizes, VS saves, CPU kinds, stamina, events, Classic and Adventure stages, disc
errors) is left out.

Each job checks its target functions on every call (LOCKSTEP_UNLIMITED) and scripts its way in
with MONKEY_HOLD from boot; after its script the monkey plays on. Each job gets its own copy of
the card.

    python tools/lockstep/campaign/mkjobs-recipes-menus.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds template-saved.raw (a card with Melee's save). The scripts' fields were read from
short runs with CALLS (tools/run/src/main.rs) on the functions each one opens: from the main menu
(303), Name Entry opens at field 502 (its keyboard at 602), the VS records at 601, the Event list
at 502, the item switch at 701, the random stage switch at 861, the lottery and the character
selects at 503 (the Multi-Man ones 3 fields after its menu's A), their matches 132 fields after,
the Classic continue screen at 1641 and the title's attract demo, left alone, at 6155.
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


def pokes(*specs):
    return ",".join(specs)


def shift(presses, by):
    return [(p[0], p[1] + by) + tuple(p[2:]) for p in presses]


# Start past the opening movie (the title at 103), Start at the title (the main menu at 303, on
# the item MENU puts under the cursor) and A, which opens it at 502. The modes start from the
# menus, not --mode: --mode leaves the state machine's current mode at the title's
# (gm_GetCurrentGameMode() is 0 in the mode it starts), so the name keyboard's exit, for one,
# takes the character select's path and faults.
MENU_A = [("start", 100), ("start", 300), ("a", 500)]
# The character selects (open at 503) as mkjobs-levers.py scripts them from 104: both ports, or
# port 1 alone, move up onto a portrait, pick and start (the match at 635; UNLOCK_ALL keeps the
# cursor off locked fighters).
VS_CSS = MENU_A + shift([("sy=127,p2+sy=127", 160, 25), ("a,p2+a", 190), ("start", 230)], 400)
ONE_P_CSS = MENU_A + shift([("sy=127", 160, 25), ("a", 190), ("start", 230)], 400)
# MENU kinds and items: VS Melee, Training, Classic, All-Star, Target Test, Home-Run Contest,
# Multi-Man Melee (its own menu, kind 33: A, then right to the mode, 10-Man first), the lottery.
VS_MENU, TRAINING_MENU, CLASSIC_MENU, ALLSTAR_MENU = "2,0", "1,4", "6,0", "6,2"
TARGET_MENU, HOMERUN_MENU, MULTIMAN_MENU, LOTTERY_MENU = "9,0", "9,1", "9,2", "3,1"


def multiman_css(downs, script):
    """The Multi-Man menu's mode `downs` to the right, then the character select's script (as from
    the select's opening at 104)."""
    at = 600 + 20 * downs + 20
    return MENU_A + steps(600, 20, ["right"] * downs) + [("a", at)] + shift(script, at - 100)

# Name Entry (mnname): the cursor starts on New (24); Sort is 25 and Delete 26 below it. Delete
# puts the cursor on the names' grid (6 rows by 4 columns shown, L and R or the edges scroll the
# columns), A there asks, left or right picks Yes and A deletes (gmMainLib_8015DBF4).
NAME_FNS = ("mnName_8023749C,CompareNameStrings,mnName_SortNames,mnName_80237D94,"
            "mnName_ConfirmNameDeleteInput,mnName_MainInput,mnName_80238754,mnName_802388D4,"
            "mnName_80238AE0,fn_80239574,mnName_80239878,mnName_80239A24,mnName_8023A290,"
            "mnName_8023A59C,mnName_8023A9B4,gmMainLib_8015DBF4")
NAME = {"LOCKSTEP_UNLIMITED": NAME_FNS, "MENU": "2,4"}
DELETE_FIRST = ["down", "down", "a", "a", "right", "a"]
NAME_GRID = MENU_A + steps(620, 20, ["l"] * 3 + ["r"] * 7 + ["down", "a", "a"]
                           + ["down", "a"] + ["right"] * 7 + ["left"] * 8 + ["down"] * 7
                           + ["up"] * 7 + ["a", "right", "left", "b"] + ["right"] * 7
                           + ["a", "right", "a"] + ["down", "a", "right", "a"]
                           + ["left"] * 2 + ["a", "left", "a", "b", "b", "b"])
NAME_SORT = MENU_A + steps(600, 20, ["down", "a", "a", "a"] + ["down", "a", "a", "right", "a"]
                           + ["up", "a", "a"])
# A name's 8 bytes (namedata, Shift-JIS) at 0x2FF8 + 0x1A4 * i + 0x198 from the save (i < 19).
NAMEDATA = [0x3190 + 0x1A4 * i for i in range(19)]
# Every nametag gmMainLib_8015DBF4 renumbers: the VS slots and event data, each GmVsMode table's
# six players (0x590 + 0x140 * mode + 0x68 + 0x24 * player + 0xA) and the rules' three.
NAMETAGS = ([0x520, 0x526, 0x52C, 0x534, 0x586]
            + [0x590 + 0x140 * k + 0x72 + 0x24 * i for k in range(13) for i in range(6)]
            + [0x1860, 0x1861, 0x1863])


def nametags(value):
    return ",".join(f"{off:#x}:1:{value}" for off in NAMETAGS)


# Printf formats as names: Name Entry draws each name as HSD_SisLib_803A6B98's format.
MSL_FNS = ("__pformatter,float2str,round_decimal,longlong2str,long2str,parse_format,__num2dec,"
           "__StringWrite")
FORMATS = [0x256C6C6400000000, 0x2568687800000000, 0x25232E3373000000, 0x252E343066000000]
NAME_SHOW = MENU_A + steps(800, 30, ["l", "r", "down", "a", "a", "b"])

# The new-name keyboard (mnnamenew): 50 keys in ten columns of five from the right (A, 45, under
# the cursor first), the side column (mode keys 0x33-0x35, then backspace 0x36, random 0x37 and
# OK 0x38) right of column 0. In English (mode 2) keys 2, 7, 12 and 17 are spaces.
NEW_FNS = ("mnNameNew_KeySetup,PickAutoName,NameContainsOnlySpaces,mnNameNew_MainInput,"
           "mnNameNew_GlyphVariantSetup")
NEW = {"LOCKSTEP_UNLIMITED": NEW_FNS + "," + NAME_FNS, "MENU": "2,4"}
# Backspace on an empty name, the random key 50 times, backspaces, Start twice (OK).
NEW_RANDOM = MENU_A + steps(600, 20, ["a"] + ["right"] * 10 + ["down", "a", "down"]) + steps(
    900, 16, ["a"] * 50) + steps(1720, 20, ["up"] + ["a"] * 5 + ["down", "a", "start", "start"])
# Three spaces and a J (refused: not only spaces), backspace, a fourth space (refused: only
# spaces), then the mode keys and L and R, backspaces and B on the empty name.
NEW_SPACES = MENU_A + steps(600, 20, ["a"] + ["right"] * 9 + ["down", "down", "a", "a", "a",
                                      "up", "up", "a", "start", "b", "down", "down", "down",
                                      "a", "start", "start", "b", "b", "l", "r", "r", "l"]
                            + ["right"] * 10 + ["up", "a", "up", "a", "down", "down", "a"]
                            + ["b"] * 6)
NEW_JP = MENU_A + steps(600, 20, ["a"] + ["right"] * 10 + ["up", "up", "a"] + ["left"] * 8
                        + ["down"] * 3 + ["a", "a", "a", "a", "start", "b", "b", "b", "b", "l",
                                          "a", "a", "start", "start"])

# Data, Records, VS records (up at 601): L shows the rankings (mnDiagram3), R the stat sheet
# (mnDiagram2); from those L and R go round the three. X toggles names, down scrolls the rows
# (to 0x15 by fighter, 0x18 by name: the three rows by name only).
DIAG_FNS = ("mnDiagram2_GetAggregatedFighterRank,mnDiagram2_HandleInput,mnDiagram2_GetRankedName,"
            "mnDiagram2_CreateStatRow,mnDiagram2_Create,mnDiagram3_HandleInput,"
            "mnDiagram3_PopulateRankings,mnDiagram3_Init")
DIAG = {"LOCKSTEP_UNLIMITED": DIAG_FNS, "MENU": "5,3"}
DIAG_OPEN = MENU_A + [("a", 600)]
DIAG_RANKS = DIAG_OPEN + steps(800, 20, ["l"] + ["down"] * 12 + ["up"] * 12 + ["x"]
                               + ["down"] * 16 + ["x", "x"] + ["down"] * 16 + ["up"] * 3
                               + ["l", "x"] + ["down"] * 16 + ["right"] * 8 + ["left"] * 8
                               + ["x"] + ["right"] * 5 + ["down"] * 16 + ["x", "r"]
                               + ["down"] * 14 + ["r", "l", "x", "down", "l", "l", "b"])

# 1P, Event Match (the list at 502): X pages by nine, down and up past the shown rows. The list's
# length goes by the cleared events (save x1A68, bit n for event n) at debug levels to 2.
EVENT = {"LOCKSTEP_UNLIMITED": "mnEvent_8024CE74,mnEvent_8024D864,mnEvent_8024E1B4,"
         "mnEvent_8024E524", "MENU": "1,1"}
EVENT_LIST = MENU_A + steps(700, 20, ["x"] * 4 + ["down"] * 30 + ["up"] * 30 + ["y", "y"]
                            + ["down"] * 12 + ["a", "b", "b"])
# The fighters' x7C bit 0x04 (gm_80162EC8, for 50 or more cleared).
FIGHTER_FLAGS = ",".join(f"{0x1FA8 + 0xAC * i:#x}:1:0x4" for i in range(25))

CSS_FNS = ("mnCharSel_8025C020,mnCharSel_CursorThink,mnCharSel_802640A0,fn_802633B0,"
           "mnCharSel_8025D1C4,mnCharSel_8025BD30,fn_8025F0E0,mnCharSel_8025DB34")
CSS = {"LOCKSTEP_UNLIMITED": CSS_FNS}
# Both ports on one fighter, port 2 cycling its costume onto port 1's.
TWO_ON_ONE = MENU_A + shift([("sy=127,p2+sy=127", 160, 25), ("a,p2+a", 190)] + steps(
    220, 30, ["p2+x", "p2+y", "p2+x", "x", "p2+y", "y", "p2+b", "p2+a", "p2+x"]), 400)
# Port 2 through the menus, then port 2 picks.
PORT2_1P = [("p2+start", 100), ("p2+start", 300), ("p2+a", 500)] + shift(
    [("p2+sy=127", 160, 25), ("p2+a", 190), ("p2+x", 220), ("p2+start", 260)], 400)
# The cursor swept across the portraits without picking (the 1P records panel follows it).
SWEEP = [("sy=127", 160, 20), ("sx=127", 190, 60), ("sx=-128", 260, 120), ("sy=127", 390, 8),
         ("sx=127", 400, 120), ("sy=-128", 530, 12), ("sx=-128", 550, 120), ("a", 680),
         ("b", 720), ("sx=127", 740, 40)]
ONE_P = [("sy=127", 160, 25), ("a", 190), ("start", 230)]

MULTIMAN_FNS = ("gm_80182174,fn_80181E18,fn_80182B5C,gm_80182578,gm_801B65D4,gm_801B688C,"
                "gm_801B7AA0,gm_801B8024,gm_801B6F44,gm_801B74F0")
MULTIMAN = {"LOCKSTEP_UNLIMITED": MULTIMAN_FNS}

TITLE_FNS = "fn_80182F40,fn_801A1498,gm_Scene_Title_OnEnter,gmTitle_801A12C4"

GOVER_FNS = ("fn_8019EFC4,fn_8019F2D4,fn_8019F810,fn_8019F9C4,fn_801A0B60,"
             "gm_Scene_ComingSoon_OnEnter,gm_Scene_ComingSoon_OnExit")
# Classic, the fighter run off the stage's left edge (holding left) until its stocks are gone
# (the continue screen 980 fields after the select's opening).
CLASSIC_SD = ONE_P_CSS + [("sx=-128", 1000, 600)]

RESULT_FNS = ("fn_80174468,fn_8017507C,fn_801756E0,fn_80175880,fn_80175DC8,fn_80176A6C,"
              "fn_80176BF0,fn_80176D3C,fn_80176F60,fn_801771C0,gm_Scene_Results_OnEnter")
# Pause (Start), then L+R+A+Start held: the match ends there (the results 3 fields later).
def cancelled(at):
    return VS_CSS + [("start", at), ("l+r+a+start", at + 60, 10)]


TOU_FNS = ("fn_8019BA08,fn_8019BF8C,fn_8019C048,fn_8019C744,fn_8019CDBC,fn_8019D1BC,"
           "gm_Scene_TouAlt_OnFrame,gm_8019E634,fn_80192758,fn_80193FCC,fn_80192BB0,"
           "fn_80194F30,fn_80195CCC,fn_80191B5C,fn_80192938,fn_80193B58,fn_801953C8")
TOU = {"LOCKSTEP_UNLIMITED": TOU_FNS, "MENU": "2,1"}

TY_FNS = ("_tyFigupon_803155C8,_tyFigupon_80316420,_tyFigupon_80316C24,tyFigupon_Scene_OnEnter,"
          "_tyFigupon_8031753C,_tyFigupon_80317A60,_tyFigupon_80315C44,_tyFigupon_8031638C")
LOTTERY = MENU_A + steps(800, 30, ["up"] * 6 + ["down"] * 3 + ["x", "y", "z", "a"]) + steps(
    1400, 60, ["a"] * 5) + steps(1800, 30, ["z", "down", "a"]) + steps(2400, 60, ["a"] * 5
                                                                      + ["b", "b"])

ITEMSW_FNS = ("mnItemSw_8023453C,mnItemSw_80233B68,mnItemSw_802351A0,mnItemSw_80234104,"
              "fn_80233E10,fn_80234C24")
STAGESW_FNS = "mnStageSw_80236CBC,fn_80236998,mnStageSw_80236548,fn_80235F80,mnStageSw_80235C58"
# VS, Rules (MENU=2,3): Item Switch is its sixth row, Additional Rules its seventh, Random Stage
# Select the sixth there.
ITEMSW = MENU_A + steps(600, 20, ["down"] * 5 + ["a"]) + steps(
    760, 20, ["a", "right", "a", "down", "a", "left", "a"] + ["down"] * 14 + ["a"] + ["up"] * 16
    + ["left", "left", "right", "right", "right"] + ["up"] * 2 + ["a", "down", "a", "b"])
STAGESW = MENU_A + steps(600, 20, ["down"] * 6 + ["a", "wait"] + ["down"] * 5 + ["a"]) + steps(
    920, 20, ["a", "right", "a", "down", "a", "right", "a"] + ["down"] * 6 + ["right"] * 8
    + ["a"] + ["up"] * 8 + ["left"] * 8 + ["a", "b"])

# The debug menu (--mode 6), as mkjobs-levers.py's: x moves down a row, R raises a value, A
# opens, Start starts. Its 9th row is <TEST MODE> (MODE: Hanyu first).
DEBUG_TEST_MODE = [("start", 100)] + steps(300, 20, ["x"] * 8 + ["a"])
# MODE: Otoguro, its Set HomeRun Record (6th row: Chara and Count, un_803009E0 and un_80300A88
# with mode 2) and Set Target Clear (7th, mode 3).
DEBUG_RECORDS = DEBUG_TEST_MODE + steps(520, 20, ["x", "a", "wait", "wait"] + ["x"] * 5 + ["a"]
                                        + ["r"] * 4 + ["x"] + ["r"] * 4 + ["l"] * 2 + ["b"]
                                        + ["x", "a"] + ["r"] * 4 + ["x"] + ["r"] * 5 + ["l"] * 3
                                        + ["x", "r", "b", "b"])


# MODE: Nagasima, RegularEnding Test (4th row) or Real (5th): Chara, Color and the kind of ending,
# then Start (fn_8030110C / fn_803011EC with 6).
def debug_ending(row, chara, color, kind):
    return DEBUG_TEST_MODE + steps(520, 20, ["x"] * 3 + ["a", "wait", "wait"] + ["x"] * (row - 1)
                                   + ["a", "wait"] + ["r"] * chara + ["x"] + ["r"] * color
                                   + ["x"] + ["r"] * kind + ["start"])


# MODE: Kim's Card Check - scene (2nd row) and - mode (3rd).
def debug_kim(row):
    return DEBUG_TEST_MODE + steps(520, 20, ["x"] * 4 + ["a", "wait", "wait"] + ["x"] * (row - 1)
                                   + ["a"])


DEBUG_FNS = ("un_803009E0,un_80300A88,fn_8030110C,fn_803011EC,un_80301490,un_80301454,"
             + GOVER_FNS)

# name, environment (None drops a base setting), card, --mode, presses, fields
JOBS = [
    # Name Entry's grid: 24 names (four columns, no scrolling), 27, 30 (five: the wrapped
    # column), the full 120 (New refused), each scrolled, sorted and deleting.
    (f"name{n}", {**NAME, "NAMES": n}, "template-saved", None,
     NAME_GRID + ([("a", 600)] if n == 120 else []), 3500)
    for n in (24, 27, 30, 120)
] + [
    # SortNames and CompareNameStrings on names that share prefixes and spaces.
    ("namecmp", {**NAME, "NAMES": 2, "SAVE_POKE": f"{NAMEDATA[1]:#x}:8:0x8260826081400000"},
     "template-saved", None, NAME_SORT, 2500),
    ("namecmp3", {**NAME, "NAMES": 3, "SAVE_POKE": pokes(
        f"{NAMEDATA[0]:#x}:8:0x8260826081400000", f"{NAMEDATA[1]:#x}:8:0x8260826000000000",
        f"{NAMEDATA[2]:#x}:8:0x8260814082600000")}, "template-saved", None, NAME_SORT, 2500),
    # gmMainLib_8015DBF4 with every nametag on the deleted name (0), above it (2), and 1 deleted
    # under twice (above, then on it).
    ("nametag0", {**NAME, "NAMES": 3, "SAVE_POKE": nametags(0)}, "template-saved", None,
     MENU_A + steps(600, 20, DELETE_FIRST + ["b", "b"]), 2000),
    ("nametag2", {**NAME, "NAMES": 3, "SAVE_POKE": nametags(2)}, "template-saved", None,
     MENU_A + steps(600, 20, DELETE_FIRST + ["b", "b"]), 2000),
    ("nametag1", {**NAME, "NAMES": 3, "SAVE_POKE": nametags(1)}, "template-saved", None,
     MENU_A + steps(600, 20, DELETE_FIRST + ["a", "right", "a", "b", "b"]), 2000),
    # Printf formats as names (MSL's integer, string and float conversions). %n faults the game
    # itself (an unmapped write at 0 from __pformatter as Name Entry opens), and %.500f overruns
    # the text's buffer (a jump to 0x30303030): neither is a job.
    ("nameprintf", {**NAME, "LOCKSTEP_UNLIMITED": NAME_FNS + "," + MSL_FNS, "NAMES": 6,
                    "SAVE_POKE": pokes(*(f"{NAMEDATA[i]:#x}:8:{v:#x}"
                                         for i, v in enumerate(FORMATS)))},
     "template-saved", None, NAME_SHOW, 2000),
    # The new-name keyboard.
    ("namerandom", {**NEW, "NAMES": 3}, "template-saved", None, NEW_RANDOM, 3500),
    ("namerandomfull", {**NEW, "NAMES": 119}, "template-saved", None, NEW_RANDOM, 3500),
    ("namespaces", {**NEW, "NAMES": 3}, "template-saved", None, NEW_SPACES, 3000),
    ("namekana", {**NEW, "NAMES": 3}, "template-saved", None, NEW_JP, 3000),
    ("namekanajp", {**NEW, "NAMES": 3, "GAME_LANGUAGE": "jp"}, "template-saved", None, NEW_JP,
     3000),
] + [
    # The records' rankings and stat sheets by fighter and by name.
    (f"diagranks{tag}", {**DIAG, **more}, "template-saved", None, DIAG_RANKS, 4000)
    for tag, more in [
        ("1", {"NAMES": 1, "RECORDS": 21}),
        ("6", {"NAMES": 6, "RECORDS": 22}),
        ("30", {"NAMES": 30, "RECORDS": 23}),
        ("120", {"NAMES": 120, "RECORDS": 24}),
        ("max", {"NAMES": 12, "RECORDS": 25, "SAVE_POKE": "stats=max"}),
        ("none", {"RECORDS": 26}),
        ("jp", {"NAMES": 8, "RECORDS": 27, "GAME_LANGUAGE": "jp"}),
    ]
] + [
    # The Event list by cleared events: 6, 10, 16, 22, 27 (with every fighter, and without),
    # 50 and all 51 (with and without the fighters' flags).
    (f"event{n}{tag}", {**EVENT, "SAVE_POKE": f"0x1A68:8:{(1 << n) - 1:#x}" + extra, **more},
     "template-saved", None, EVENT_LIST, 3000)
    for n, tag, extra, more in [
        (6, "", "", {}), (10, "", "", {}), (16, "", "", {}), (22, "", "", {}),
        (27, "", "", {}), (27, "locked", "", {"UNLOCK_ALL": None}),
        (50, "", "", {}), (51, "", "", {}), (51, "flags", "," + FIGHTER_FLAGS, {}),
    ]
] + [
    # Character selects: Training, both ports on one costume (Training, VS), port 2's 1P select,
    # the Japanese ones, and the 1P records panels swept over (Classic, All-Star, multi-man,
    # Target Test, Home-Run Contest).
    ("csstraining", {**CSS, "MENU": TRAINING_MENU}, "template-saved", None, TWO_ON_ONE, 3000),
    ("csstrainingjp", {**CSS, "MENU": TRAINING_MENU, "GAME_LANGUAGE": "jp"}, "template-saved",
     None, TWO_ON_ONE, 3000),
    ("cssvssame", {**CSS, "MENU": VS_MENU}, "template-saved", None, TWO_ON_ONE, 3000),
    ("css1pport2", {**CSS, "MENU": CLASSIC_MENU, "RECORDS_1P": 31}, "template-saved", None,
     PORT2_1P, 3000),
    ("css1pport2jp", {**CSS, "MENU": CLASSIC_MENU, "RECORDS_1P": 32, "GAME_LANGUAGE": "jp"},
     "template-saved", None, PORT2_1P, 3000),
] + [
    (f"cssrecords{tag}", {**CSS, "MENU": menu, "RECORDS_1P": seed, **more}, "template-saved",
     None, multiman_css(downs, SWEEP) if menu == MULTIMAN_MENU else MENU_A + shift(SWEEP, 400),
     2500)
    for tag, menu, downs, seed, more in [
        ("classic", CLASSIC_MENU, 0, 33, {}),
        ("classicjp", CLASSIC_MENU, 0, 34, {"GAME_LANGUAGE": "jp"}),
        ("allstar", ALLSTAR_MENU, 0, 35, {}), ("10man", MULTIMAN_MENU, 0, 36, {}),
        ("15min", MULTIMAN_MENU, 3, 37, {}),
        ("cruel", MULTIMAN_MENU, 5, 38, {"GAME_LANGUAGE": "jp"}),
        ("target", TARGET_MENU, 0, 39, {}), ("homerun", HOMERUN_MENU, 0, 40, {}),
    ]
] + [
    # Multi-Man Melee played by the monkey from its start (gm_80182174), and 3-Minute left idle
    # (survived, or not).
    (f"multiman{tag}", {**MULTIMAN, "MENU": MULTIMAN_MENU}, "template-saved", None,
     multiman_css(downs, ONE_P), fields)
    for tag, downs, fields in [("10man", 0, 12000), ("100man", 1, 30000), ("3min", 2, 15000),
                               ("15min", 3, 60000), ("endless", 4, 20000), ("cruel", 5, 12000)]
] + [
    ("multiman3minidle", {**MULTIMAN, "MENU": MULTIMAN_MENU}, "template-saved", None,
     multiman_css(2, ONE_P + [("none", 240, 11500)]), 13000),
    # The title left alone: the opening movie, the title (978), its attract demo (fn_80182F40
    # from 6155 for 1200 frames) and the title again (10128); the demo cut short by Start or A;
    # and A at the title instead of Start.
    ("attract", {"LOCKSTEP_UNLIMITED": TITLE_FNS}, "template-saved", None,
     [("none", 0, 10500), ("start", 10600)], 11500),
    ("attractstart", {"LOCKSTEP_UNLIMITED": TITLE_FNS}, "template-saved", None,
     [("none", 0, 6900), ("start", 6900)], 8000),
    ("attractajp", {"LOCKSTEP_UNLIMITED": TITLE_FNS, "GAME_LANGUAGE": "jp"}, "template-saved",
     None, [("none", 0, 6500), ("a", 6500)], 8000),
    ("titlea", {"LOCKSTEP_UNLIMITED": TITLE_FNS}, "template-saved", None,
     [("start", 100), ("a", 300), ("a", 500), ("start", 1300)], 2500),
] + [
    # Classic lost (run off the stage), then the continue screen with no coins and with 9999,
    # left to count down or answered.
    (f"gameover{tag}", {"LOCKSTEP_UNLIMITED": GOVER_FNS, "MENU": CLASSIC_MENU,
                        "SAVE_POKE": f"coins={coins}"},
     "template-saved", None, CLASSIC_SD + extra, 6500)
    for tag, coins, extra in [
        ("coins0", 0, []), ("coins0a", 0, steps(1800, 60, ["a"] * 10)),
        ("coins9999", 9999, []), ("coins9999a", 9999, steps(1800, 60, ["a"] * 10)),
        ("coins9999b", 9999, steps(1800, 60, ["right", "a", "b", "a"] * 3)),
    ]
] + [
    # VS matches ended from the pause with L+R+A+Start, by time, stock and coins.
    (f"cancel{tag}", {"LOCKSTEP_UNLIMITED": RESULT_FNS, "MENU": VS_MENU,
                      "SAVE_POKE": f"{poke},pause=1"},
     "template-saved", None, cancelled(at), 6500)
    for tag, poke, at in [
        ("time", "stage_sel=1", 1100), ("stock", "stage_sel=1,mode=1", 1900),
        ("coin", "stage_sel=1,mode=2", 2800),
        ("ff", "stage_sel=1,friendly_fire=1", 1300),
    ]
] + [
    # Tournaments with human players, for the monkey on the match-up screens (L+R's random
    # fighter, the setup's cursor resting).
    (f"tou{n}{tag}", {**TOU, "TOU_ENTRANTS": n, "SAVE_POKE": poke}, "template-saved", None,
     MENU_A, 40000)
    for n, tag, poke in [
        (6, "", "time_limit=1,pause=0"),
        (16, "handicap", "handicap=1,time_limit=1,pause=0"),
        (32, "", "mode=1,stock_count=1,pause=0"),
    ]
] + [
    # The lottery with no coins, too few, enough and plenty.
    (f"lottery{coins}{tag}", {"LOCKSTEP_UNLIMITED": TY_FNS, "MENU": LOTTERY_MENU,
                              "SAVE_POKE": f"coins={coins}",
                              **more}, "template-saved", None, LOTTERY, 5000)
    for coins, tag, more in [(0, "", {}), (9, "", {}), (10, "", {}), (35, "", {}),
                             (500, "", {}), (9999, "", {}), (9999, "jp", {"GAME_LANGUAGE": "jp"})]
] + [
    # The item switch and the random stage select, on unlocked, locked, empty and full sets.
    (f"itemsw{tag}", {"LOCKSTEP_UNLIMITED": ITEMSW_FNS, "MENU": "2,3", **more},
     "template-saved", None, ITEMSW, 3000)
    for tag, more in [
        ("", {}), ("locked", {"UNLOCK_ALL": None}),
        ("none", {"SAVE_POKE": "item_mask=0,item_freq=0"}),
        ("all", {"SAVE_POKE": "item_mask=0xffffffffffffffff,item_freq=4"}),
        ("jp", {"GAME_LANGUAGE": "jp", "SAVE_POKE": "item_freq=3"}),
    ]
] + [
    (f"stagesw{tag}", {"LOCKSTEP_UNLIMITED": STAGESW_FNS, "MENU": "2,3", **more},
     "template-saved", None, STAGESW, 3000)
    for tag, more in [
        # Without UNLOCK_ALL the Additional Rules' sixth row doesn't open it: the stages locked
        # here instead.
        ("", {}), ("locked", {"SAVE_POKE": "stages=0"}),
        ("none", {"SAVE_POKE": "stage_mask=0"}),
        ("all", {"SAVE_POKE": "stage_mask=0xffffffff"}),
        ("lockednone", {"SAVE_POKE": "stages=0,stage_mask=0"}),
    ]
] + [
    # X or Y held through boot at each debug level (gmMain_8015FDA4's launch buttons).
    (f"launch{b}{level}", {"LOCKSTEP_UNLIMITED": "gmMain_8015FDA4", "DBLEVEL": level},
     "template-saved", None, [(b, 0, 60)], 2500)
    for level in (0, 2, 3, 4) for b in ("x", "y")
] + [
    # The debug menu's record setters and its regular ending test (each ending kind, then the
    # real one) and card checks.
    ("dbgrecords", {"LOCKSTEP_UNLIMITED": DEBUG_FNS}, "template-saved", "06", DEBUG_RECORDS,
     3000),
] + [
    (f"dbgending{kind}", {"LOCKSTEP_UNLIMITED": DEBUG_FNS}, "template-saved", "06",
     debug_ending(4, chara, chara % 4, kind), 4500)
    for kind, chara in [(0, 0), (1, 5), (2, 12), (3, 20)]
] + [
    ("dbgendingreal", {"LOCKSTEP_UNLIMITED": DEBUG_FNS}, "template-saved", "06",
     debug_ending(5, 3, 1, 0), 4500),
    ("dbgcardscene", {"LOCKSTEP_UNLIMITED": DEBUG_FNS}, "template-saved", "06", debug_kim(2),
     3000),
    ("dbgcardmode", {"LOCKSTEP_UNLIMITED": DEBUG_FNS}, "template-saved", "06", debug_kim(3),
     3000),
]


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
        env = {"LOCKSTEP_SEED": 1100 + i, "UNLOCK_ALL": 1, "MONKEY_B": 2, "STALL_BEATS": 200,
               "LOCKSTEP_MUTATE": 2}
        if presses:
            env["MONKEY_HOLD"] = hold(presses)[0]
        env.update(more)
        envs = " ".join(f"{k}={v}" for k, v in env.items() if v is not None)
        print(f"{job}|{envs}{args} --monkey {5100 + i} --fields {fields}")


if __name__ == "__main__":
    main()
