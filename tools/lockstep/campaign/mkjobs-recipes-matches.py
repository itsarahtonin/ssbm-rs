"""Writes jobs for the static analyses' stage and fighter recipes: long debug VS matches on the
stages whose hazards come round only after the per-function check cap had stopped checking them
(Venom's Arwings and Star Fox talk, Corneria's, Rainbow Cruise, Brinstar, both Kongo Jungles,
Green Greens, Flat Zone, Peach's Castle, Fourside, Yoshi's Island 64, Mute City, the Mushroom
Kingdoms, Icicle Mountain, Pokemon Stadium, Fountain of Dreams, Onett) and the Home-Run Contest;
and debug VS matches whose fighters MONKEY_HOLD scripts into the moves the analyses asked for
(Kirby's copies of Link, Young Link and Jigglypuff, Ness's PK Thunder into himself, Yoshi's egg
roll, Jigglypuff's Rollout, Marth's Dancing Blade, Samus's grapple, grabs, shields, the Ice
Climbers, Link holding an item, Donkey Kong's carry, Mr. Saturn, both hands). Each job checks its
target functions on every call (LOCKSTEP_UNLIMITED); after its script the monkey plays on.

    python tools/lockstep/campaign/mkjobs-recipes-matches.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds template-saved.raw (a card with Melee's save), for the Home-Run Contest's job.
A scripted debug VS job presses Start every 10 fields from 30, past booting and the opening
movie (a hold that leaves them out keeps the game booting); its first match starts by about 60.
The scripts' fields were read from short runs with CALLS on the functions each one opens.
--matches seeds come from choose(), a copy of tools/run/src/matches.rs's, so that the scripted
ports play the fighters the scripts are for. Names several functions share (the stages'
stageGObj procs) go to LOCKSTEP_UNLIMITED as hex addresses, and MATCH_PLAYERS leaves ports
empty: both need ssbm-run from this commit on. The Great Bay, Big Blue, Event 37, CPU kind,
stamina, 1P stage start and disc jobs are mkjobs-levers.py's and jobs-followup.txt's.
"""
import os
import re
import shutil
import sys

PRESS = 4
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..")


# --- choose(), as tools/run/src/matches.rs has it (MATCH_RULES unset) ---------------------------

class Rng:
    """monkey.rs's Rng (xorshift64*)."""

    def __init__(self, seed):
        self.x = max(seed, 1)

    def next(self):
        x = self.x
        x ^= x >> 12
        x ^= (x << 25) & (2**64 - 1)
        x ^= x >> 27
        self.x = x
        return (x * 0x2545F4914F6CDD1D) & (2**64 - 1)

    def chance(self, p):
        return self.next() % 100 < p

    def below(self, n):
        return self.next() % n


def choose(seed, stages, fighters, players=4, cpu_kinds=None):
    """The first match of --matches SEED: stage, item frequency (-1 none), and for each port
    playing (CKind, CPU, level)."""
    r = Rng(seed)
    stage = stages[r.below(len(stages))]
    items = -1 if r.chance(15) else r.below(5)
    out = []
    for i in range(4):
        if i >= players:
            continue
        f = fighters[r.below(len(fighters))]
        cpu = f >= 26 or r.chance(50)
        level = 1 + r.below(9) if cpu else 0
        if cpu and cpu_kinds:
            r.below(len(cpu_kinds))
        out.append((f, cpu, level))
    return stage, items, out


def seed(stages, fighters, want, players=4, cpu_kinds=None, start=1):
    """The first --matches seed from `start` whose first match `want(items, players)` takes."""
    for s in range(start, start + 200000):
        _, items, ps = choose(s, stages, fighters, players, cpu_kinds)
        if want(items, ps):
            return s
    raise SystemExit(f"no seed for {stages} {fighters}")


def p1_human_rest(cpu, items=None):
    """P1 human playing the list's first fighter, the others all CPUs (cpu) or all humans, and
    the items (None: any; "off": none; "on": some)."""
    def want(it, ps):
        if items == "off" and it != -1 or items == "on" and it == -1:
            return False
        return not ps[0][1] and all(p[1] == cpu for p in ps[1:])
    return want


def first_is(fighter, want):
    return lambda it, ps: ps[0][0] == fighter and want(it, ps)


# --- functions -----------------------------------------------------------------------------------

def symbols():
    """Each symbol's addresses (the generated symbol table): names several functions share go
    by address in LOCKSTEP_UNLIMITED."""
    path = os.path.join(ROOT, "crates", "types", "src", "gen", "symbols.rs")
    out = {}
    for m in re.finditer(r'\((0x[0-9a-f]+), 0x[0-9a-f]+, "([^"]+)", (?:true|false)\)',
                         open(path).read()):
        out.setdefault(m.group(2), []).append(int(m.group(1), 16))
    return out


SYMS = symbols()


def fns(*names):
    """LOCKSTEP_UNLIMITED's list: each function by name, or by its hex address where others
    share its name."""
    addresses = {a for addrs in SYMS.values() for a in addrs}
    out = []
    for n in names:
        if n.startswith("0x"):
            if int(n, 16) not in addresses:
                raise SystemExit(f"no function at {n}")
            out.append(n)
            continue
        addrs = SYMS.get(n)
        if addrs is None:
            raise SystemExit(f"no symbol {n}")
        if len(addrs) > 1:
            raise SystemExit(f"{n} names {len(addrs)} functions: give its address")
        out.append(n)
    return ",".join(out)


# The stage functions with blocks left (local/lockstep/needed.txt less tools/lockstep/gaps.txt).
VENOM = fns("grVenom_8020362C", "grVenom_80203B18", "grVenom_8020454C", "grVenom_80204CEC",
            "grVenom_80204F20", "grVenom_80205AD4", "grVenom_80205F30", "grVenom_80206874")
CORNERIA = fns("grCorneria_801DCE1C", "grCorneria_801DDAC4", "grCorneria_801DE8E4",
               "grCorneria_801DED50", "grCorneria_801DF8D0", "grCorneria_801E03C8",
               "grCorneria_801E1BF0", "grCorneria_801E2228", "grCorneria_801E2AF4",
               "grCorneria_801E2D14")
RCRUISE = fns("grRCruise_80200154", "grRCruise_80200578", "grRCruise_8020071C",
              "grRCruise_80200C04", "grRCruise_80201588")
ZEBES = fns("grZebes_801D925C", "grZebes_801D95B8", "grZebes_801D9798", "grZebes_801D99E0",
            "grZebes_801DA3F4", "grZebes_801DAA08", "fn_801DAC90", "grZebes_801DAE70",
            "grZebes_801DB088", "grZebes_801DBB60")
# Kongo Jungle 64's stage procs share their names with other stages'.
OLDKONGO = fns("0x8020F888", "0x802100FC", "grOldKongo_80210650")
KONGO = fns("grKongo_801D7BBC", "grKongo_801D577C", "grKongo_801D8078", "grKongo_801D828C",
            "grKongo_801D6074", "grKongo_801D637C", "grKongo_801D651C")
GREENS = fns("grGreens_80213C10", "grGreens_8021360C", "grGreens_80215358", "fn_80215B84",
             "grGreens_80215ED8")
FLATZONE = fns("grFlatzone_802171D4", "grFlatzone_802176BC")
CASTLE = fns("grCastle_801CEF04", "grCastle_801CF308", "grCastle_801D07BC", "grCastle_801D0834",
             "grCastle_801D08AC", "grCastle_801CE3AC")
FOURSIDE = fns("grFourside_801F3274", "grFourside_801F3CC8", "grFourside_801F3894")
OLDYOSHI = fns("grOldYoshi_8020F088")
MUTECITY = fns("grMuteCity_801F04B8", "grMuteCity_801F106C", "grMuteCity_801F1A34",
               "grMuteCity_801EFDF8", "grMuteCity_801F290C")
INISHIE1 = fns("grInishie1_801FAD84", "grInishie1_801FB0AC", "grInishie1_801FB3F0",
               "grInishie1_801FBAA0", "grInishie1_801FC664", "grInishie1_801FCB10")
INISHIE2 = fns("grInishie2_801FD224", "grInishie2_801FD4F0", "grInishie2_801FD824",
               "grInishie2_801FD9EC")
# Icicle Mountain's setupStageCallbacks and stageGObj procs share their names too.
ICEMT = fns("grIceMt_801F686C", "grIceMt_801F7080", "0x801F71E8", "0x801F72D4", "0x801F796C",
            "0x801F7A2C", "fn_801F9338", "grIceMt_801F9ACC", "grIceMt_801FA0BC",
            "grIceMt_801FA6D8")
STADIUM = fns("grStadium_801D2344", "grStadium_801D384C")
IZUMI = fns("grIzumi_801CCBDC")
ONETT = fns("grOnett_801E3DA0", "grOnett_801E5030", "grOnett_801E43E0", "grOnett_801E41C8")
HOMERUN = fns("grHomeRun_8021D680", "grHomeRun_8021CB20", "grHomeRun_8021E500",
              "grHomeRun_8021C914")

KIRBY_LINK = fns("ftKb_SpecialNLk800FB500", "ftKb_SpecialNLk800FB5F4", "ftKb_SpecialNLk800FB880",
                 "ftKb_SpecialNLk800FBA00", "ftKb_LkSpecialNStart_Anim",
                 "ftKb_LkSpecialAirNStart_Anim", "ftKb_LkSpecialAirNEnd_Anim",
                 "ftKb_LkSpecialNStart_IASA", "ftKb_LkSpecialNLoop_IASA",
                 "ftKb_LkSpecialAirNStart_IASA", "ftKb_LkSpecialAirNLoop_IASA",
                 "ftKb_LkSpecialNLoop_Coll", "ftKb_LkSpecialNEnd_Coll",
                 "ftKb_LkSpecialAirNStart_Coll", "ftKb_LkSpecialAirNLoop_Coll",
                 "ftKb_LkSpecialAirNEnd_Coll", "ftKb_SpecialN_800EF0E4", "ftKb_SpecialN_800EF438")
KIRBY_PURIN = fns("fn_80100E0C", "ftKb_SpecialNPr_801010D4", "ftKb_PrSpecialNLoop_Anim",
                  "ftKb_PrSpecialN1_Anim", "ftKb_PrSpecialAirNLoop_Anim",
                  "ftKb_PrSpecialAirN_Anim", "ftKb_PrSpecialNFull_IASA",
                  "ftKb_PrSpecialAirNFull_IASA", "ftKb_PrSpecialNTurn_Phys",
                  "ftKb_PrSpecialNStart_Coll", "ftKb_PrSpecialNLoop_Coll",
                  "ftKb_PrSpecialNFull_Coll", "ftKb_PrSpecialN1_Coll",
                  "ftKb_PrSpecialAirNLoop_Coll", "ftKb_PrSpecialAirNFull_Coll",
                  "ftKb_PrSpecialAirN_Coll", "ftKb_SpecialN_800EF0E4", "ftKb_SpecialN_800EF438")
NESS = fns("ftNs_SpecialAirHi_CollisionModVel", "ftNs_SpecialHi_Enter", "ftNs_SpecialAirHi_Enter",
           "ftNs_SpecialHiHold_Anim", "ftNs_SpecialHi_Anim", "ftNs_SpecialAirHiHold_Anim",
           "ftNs_SpecialAirHi_Phys", "ftNs_SpecialHi_Coll", "ftNs_SpecialAirHi_Coll",
           "ftNs_SpecialAirHiRebound_Coll")
YOSHI = fns("fn_8012EDE8", "ftYs_SpecialS_8012F0DC", "ftYs_SpecialAirSLoop_1_Anim",
            "ftYs_SpecialAirSEnd_Anim", "ftYs_SpecialAirSLoop_2_Anim",
            "ftYs_SpecialAirSLanding_Anim", "ftYs_SpecialAirSLoop_0_IASA",
            "ftYs_SpecialAirSLoop_1_Phys", "ftYs_SpecialAirSLoop_0_Coll",
            "ftYs_SpecialAirSLoop_1_Coll", "ftYs_SpecialAirSLoop_2_Coll",
            "ftYs_SpecialAirSLoop_3_Coll")
PURIN = fns("ftPr_SpecialS_8013DA24", "ftPr_SpecialAirNChargeRelease_Anim",
            "ftPr_SpecialNStart_Coll", "ftPr_SpecialNFull_Coll", "ftPr_SpecialNRelease_Coll",
            "ftPr_SpecialAirNChargeLoop_Coll", "ftPr_SpecialAirNChargeFull_Coll",
            "ftPr_SpecialAirNChargeRelease_Coll", "ftPr_SpecialAirNStartTurn_Coll")
MARTH = fns("ftMs_SpecialS_80137940", "ftMs_SpecialS_801379D0", "ftMs_SpecialS_80137CBC",
            "ftMs_SpecialS_80137D60", "ftMs_SpecialS_80137FF8", "ftMs_SpecialS_8013809C")
SAMUS = fns("it_802B7160", "it_802B743C", "it_802B75FC", "it_802B7C18", "fn_802B7E34",
            "fn_802B805C", "fn_802B895C", "fn_802B8D38", "it_802B99A0", "it_802BACC4",
            "ftSs_Init_CreateThrowGrappleBeam", "ftCo_AirCatch_Anim", "ftCo_CatchPull_Anim")
CAPTURE = fns("ftCo_8008EC90", "ftCo_800DC920", "ftCo_800DCE34", "fn_800DAADC", "fn_800DAEEC",
              "ftCo_CapturePulledLw_Phys", "fn_800DB230", "fn_800DBBF8",
              "ftCo_CaptureWaitLw_Phys", "fn_800DBED4", "ftCo_CatchPull_Anim")
SHIELD = fns("ftCo_80099010", "ftCo_GuardReflect_IASA")
CLIMBERS = fns("ftPp_SpecialS_80120FE0", "ftPp_SpecialHiThrow_0_Anim", "it_802C248C",
               "it_802C2EC4", "it_802C33B8", "it_802C1590", "itClimbersice_UnkMotion2_Phys",
               "itClimbersice_UnkMotion3_Coll")
LINK = fns("ftLk_SpecialN_Enter", "ftLk_SpecialAirN_Enter", "ftLk_SpecialNStart_Anim",
           "ftLk_SpecialNEnd_Anim", "ftLk_SpecialAirNStart_Anim", "ftLk_SpecialAirNEnd_Anim",
           "it_802AF32C", "itLinkBow_Logic100_PickedUp", "itLinkArrow_802A850C")
CARRY = fns("ftCo_8009BC58", "ftCo_8009C5A4", "ftCo_8009C640")
SATURN = fns("itDosei_802817A0", "itDosei_Logic7_PickedUp", "itDosei_UnkMotion4_Anim",
             "itDosei_Logic7_Dropped", "itDosei_Logic7_Thrown", "itDosei_80282DE4",
             "itDosei_Logic7_DmgReceived")
HANDS = fns("ftMh_MS_341_8014FF1C", "ftMh_MS_341_8014FFDC", "ftMh_Wait1_0_Anim",
            "ftMh_MS_341_80150894", "ftMh_MS_389_80150C8C", "ftMh_MS_389_80150D28",
            "ftMh_MS_389_80150DC4", "ftMh_MS_389_80151018", "ftMh_MS_370_80153D2C",
            "ftMh_MS_382_801552F8", "ftMh_TagSqueeze_Anim", "ftCh_Init_801560D8",
            "ftCh_Init_80156198", "ftCh_Wait1_0_Anim", "ftCh_Init_80156AD8", "fn_80159AA4",
            "ftCh_BackAirplane2_Anim", "fn_80157080", "ftCh_GrabUnk1_8015AC50",
            "ftCh_Init_8015868C", "ftCh_GrabUnk1_8015BA34", "ftCh_GrabUnk1_8015BC88",
            "ftBossLib_8015C6BC", "ftBossLib_8015C74C", "ftBossLib_8015C7EC",
            "ftBossLib_8015C88C", "ftBossLib_8015C92C", "ftBossLib_8015C9CC")

# --- scripts -------------------------------------------------------------------------------------
# (holds, field[, fields]): holds are comma-joined, one per port (p2+... for port 2). Fields count
# from boot; the scripted debug VS jobs' first match starts by 60, their fighters move from 200.

R, L_, U, D = "sx=127", "sx=-128", "sy=127", "sy=-128"

# Booting waits for a press before it goes on to --mode, and the opening movie for another: Start
# every 10 fields up to the script (the matches don't pause).
DEBUG_VS_START = [("none", 10, 20)] + [("start", f) for f in range(30, 200, 10)]

# Kirby (P1) walks right into the Link, Young Link or Jigglypuff ahead of him on Final
# Destination, inhales it (ftKb_SpecialN_Enter at 231) and B swallows it (the copy 7 fields
# later: ftKb_SpecialN_800F1BAC from ftKb_SpecialNDrink_Anim); then the copy on the ground, held
# to its loop and released; short in the air; held from a double jump's top into the landing
# (the Lk air start lands into the ground start); then run off the stage's right edge and held
# through the fall; after the respawn, a new inhale facing left, and its copy.
KIRBY_COPY = [(R, 200, 30), ("b", 230, 61), ("b", 330), ("down", 336),
              ("b", 420, 70),
              ("x", 520), ("b", 530, 16),
              ("x", 600), ("x", 612), ("b", 626, 75),
              (R, 760, 100), ("b", 860, 120),
              (L_, 1300, 30), ("b", 1330, 61), ("b", 1430), ("b", 1520, 70)]

# Ness (P1): PK Thunder from a double jump, steered clockwise round a full turn (the thunder
# turns 6 degrees a field toward a stick 45 degrees or more off its way: PlNs.dat's article 3)
# back into him, the stick moved a quarter turn every 15 fields from 4 or 12 fields after the
# thunder appears: the self-hit's launch (ftNs_SpecialAirHi_Coll and ftNs_SpecialHi_Coll from
# 303, ftNs_SpecialAirHi_CollisionModVel at 340); four times, then once from the ground.
NESS_UPB = []
for i, a in enumerate((4, 12, 4, 12)):
    t = 200 + 260 * i
    s0 = t + 46 + a
    NESS_UPB += [("x", t), ("x", t + 14), (U + "+b", t + 26), ("sx=90+sy=-90", s0, 15),
                 ("sx=-90+sy=-90", s0 + 15, 15), ("sx=-90+sy=90", s0 + 30, 15),
                 ("sx=90+sy=90", s0 + 45, 15)]
NESS_UPB += [(U + "+b", 1300), ("sx=90+sy=-90", 1324, 15), ("sx=-90+sy=-90", 1339, 15),
             ("sx=-90+sy=90", 1354, 15), ("sx=90+sy=90", 1369, 15)]

# Yoshi (P1): egg roll right toward the stage's wall or edge, turned back, jumped from, off the
# ledge, and from the air (ftYs_SpecialS_Enter at 201, the roll's loops from 253).
YOSHI_ROLL = [(R + "+b", 200), (R, 204, 80), (L_, 284, 60), (R, 344, 30), ("x", 374),
              (R, 378, 60), (R + "+b", 500), (R, 504, 150),
              ("x", 700), (L_ + "+b", 710), (L_, 714, 120), ("x", 850), ("x", 862),
              (R + "+b", 872), (R, 876, 90)]

# Jigglypuff (P1): Rollout charged in the air from a double jump, rolling out over the edge where
# it times out before it lands (ftPr_SpecialAirNChargeRelease_Anim from 296); then on the
# ground, into the others.
PURIN_ROLL = [("x", 200), ("x", 214), ("b", 224, 70), (R, 294, 4), ("none", 298, 200),
              ("b", 600, 80), (R, 680, 60),
              (R, 800, 40), ("x", 840), ("x", 852), ("b", 860, 40), (R, 900, 80)]

# Marth (P1): Dancing Blade from a full hop (its hits 22 fields apart, started 6 to 46 fields
# into the jump) and from a double jump (16 or 14 apart, the fourth 14 to 26 after the third),
# so that its hits land (ftMs_SpecialS_801379D0 for the second hit at 276, 80137D60 for the
# third at 294 to 1014); alternately right and left, the third hit up and the fourth down. The
# fourth hit's landing and the ground hits' runs off a ledge (80137940, 80137CBC, 80137FF8)
# didn't come in the test runs: the Rainbow Cruise job's moving platforms may give those.
MARTH_DB = []
for i, (jumps, g, dl, h) in enumerate([(1, 22, 6, 22), (1, 22, 16, 22), (1, 22, 36, 22),
                                       (2, 16, 14, 14), (2, 16, 14, 18), (2, 16, 14, 22),
                                       (2, 16, 14, 26), (2, 14, 14, 20)]):
    t = 230 + 240 * i
    MARTH_DB += [("x", t)] + ([("x", t + 12)] if jumps == 2 else []) + [
        ((R if i % 2 == 0 else L_) + "+b", t + dl), ("b", t + dl + g),
        (U + "+b", t + dl + 2 * g), (D + "+b", t + dl + 2 * g + h)]

# Samus (P1): grapple standing, from a dash, and Z in the air toward the stage's walls (the
# grapple's it_802B743C from 208, ftCo_AirCatch_Anim from 370).
SAMUS_GRAB = [("z", 200), (R, 260, 20), (R + "+z", 280), ("x", 360), ("z", 368),
              (R, 420, 70), ("x", 490), ("z", 496), (L_, 560, 80), ("x", 640), ("z", 646),
              (L_ + "+z", 720), (D, 760, 20), ("x", 800), ("x", 812), ("z", 818)]

# P1 walks right into the port beside it and grabs it (at 227) while the other three smash left
# (ftCo_800DCE34 at 250, a grab hit by a third player); again with smashes right, then jabs.
OTHERS = "p2+{0},p3+{0},p4+{0}"
GRAB_THIRD = [(R, 200, 10), ("z", 215), (OTHERS.format("cx=-127"), 235, 3),
              (R, 320, 10), ("z", 335), (OTHERS.format("cx=127"), 355, 3),
              (R, 440, 10), ("z", 455)] + [(OTHERS.format("a"), 465 + 8 * i) for i in range(8)]

# P2 holds its shield until it breaks (ftCo_80099010 at 528, then Furafura) while P1 hits it.
SHIELD_BREAK = [("p2+r", 200, 700)] + [("a", 220 + 12 * i) for i in range(40)]

# Ice Climbers (P1): Squall Hammer into P2 (ftPp_SpecialS_Enter at 201), back, from a jump; then
# Belay (ftPp_SpecialS_80120FE0 is its string's script) on the ground and in the air.
CLIMBERS_S = [(R + "+b", 200), ("b", 210), ("b", 220), ("b", 230), (L_ + "+b", 330),
              ("b", 340), ("b", 350), ("x", 430), (R + "+b", 440), ("b", 450),
              (U + "+b", 560), ("x", 700), (U + "+b", 710)]
# All four Ice Climbers shield together until their shields break (eight ftCo_80099010 calls at
# 527 to 533).
CLIMBERS_SHIELD = [(",".join(f"p{p}+r" for p in range(1, 5)), 200, 800)]

# Link or Young Link (P1) pulls a bomb (down-B) and, holding it, uses the bow on the ground and in
# the air (ftLk_SpecialN_Enter at 261, ftLk_SpecialNEnd_Anim from 312, the air bow at 421).
LINK_ITEM = [(D + "+b", 200), ("b", 260, 50), (D + "+b", 360), ("x", 410), ("b", 420, 30),
             (D + "+b", 500), ("b", 550, 4)]


# name, environment, card, --mode, presses, fields, --matches seed (None for the ones that pick
# their own below)
def stage(kind, unlimited, time, fields, **more):
    return {"MATCH_STAGES": kind, "MATCH_TIME": time, "LOCKSTEP_UNLIMITED": unlimited,
            "LOCKSTEP_STALE": 100000, **more}


def fighters(stages, cks, unlimited, time=300, **more):
    return {"MATCH_STAGES": stages, "MATCH_FIGHTERS": cks, "MATCH_TIME": time,
            "LOCKSTEP_UNLIMITED": unlimited, **more}


def lst(s):
    return [int(x) for x in str(s).split(",")]


JOBS = []


def add(name, env, presses, fields, matches=None, card=None, mode="e"):
    JOBS.append((name, env, card, mode, presses, fields, matches))


# Long stage matches: the monkey and the seed's CPUs play.
for n, s in ((1, 101), (2, 202)):
    add(f"venom{n}", stage(22, VENOM, 900, 45000), [], 45000, s)
    add(f"corneria{n}", stage(7, CORNERIA, 900, 40000, MATCH_PLAYERS=n + 1), [], 40000, s)
    add(f"rcruise{n}", stage(11, RCRUISE, 600, 40000), [], 40000, s)
    add(f"zebes{n}", stage(6, ZEBES, 600, 40000), [], 40000, s)
    add(f"oldkongo{n}", stage(30, OLDKONGO, 600, 40000), [], 40000, s)
    add(f"castle{n}", stage(4, CASTLE, 600, 40000), [], 40000, s)
    add(f"fourside{n}", stage(18, FOURSIDE, 600, 40000), [], 40000, s)
    add(f"mutecity{n}", stage(10, MUTECITY, 600, 40000), [], 40000, s)
    add(f"oldyoshi{n}", stage(29, OLDYOSHI, 300, 20000), [], 20000, s)
    add(f"greens{n}", stage(17, GREENS, 300, 20000), [], 20000, s)
    add(f"pstadium{n}", stage(3, STADIUM, 300, 20000,
                             **({"GAME_LANGUAGE": "jp"} if n == 2 else {})), [], 20000, s)
# Kongo Jungle's barrel events: the Box event wants items on.
for n in (1, 2):
    add(f"kongo{n}", stage(5, KONGO, 600, 40000), [], 40000,
        seed([5], list(range(26)), lambda it, ps: it >= 2, start=300 * n))
# Green Greens' blocks stacked five high: players that stand still (CPU kind 5 has no routine).
add("greensstill", stage(17, GREENS, 600, 40000, CPU_PLAYERS=1, MATCH_CPU_KIND=5), [], 40000, 303)
add("flatzone", stage(27, FLATZONE, 300, 20000), [], 20000, 304)
add("inishie1", stage(19, INISHIE1, 600, 40000), [], 40000, 305)
add("inishie2", stage(20, INISHIE2, 300, 20000), [], 20000, 306)
add("onett", stage(9, ONETT, 300, 20000), [], 20000, 307)
add("izumi", stage(2, IZUMI, 300, 20000), [], 20000, 308)
add("icemt", stage(25, ICEMT, 300, 20000), [], 20000, 309)

# Star Fox's talk on Venom and Corneria: Fox or Falco (P1, human, the others CPUs) taps D-pad
# down for one field every 50 (ftFx_AppealS_CheckInput takes it only standing in Wait: the tap
# at 520 started ftFx_AppealS_Enter at 522 on Venom), up to 1200, then stands; the talk's own
# taps after the first are ignored. On Corneria the CPU comes to hit him mid-talk.
TALK = [("down", f, 1) for f in range(220, 1200, 50)] + [("none", 1200, 2000)]
for name, kind, ck in (("venomfox", 22, 2), ("venomfalco", 22, 20), ("cornfox", 7, 2),
                      ("cornfalco", 7, 20)):
    unl = VENOM if kind == 22 else CORNERIA
    players = 4 if kind == 22 else 2
    add(f"talk{name}",
        stage(kind, unl, 300, 12000, MATCH_FIGHTERS=ck, MATCH_PLAYERS=players),
        TALK, 12000,
        seed([kind], [ck], p1_human_rest(True), players=players))

# Home-Run Contest: Start past booting and the opening movie, then its character select (the
# mode's first screen): the stick up onto the portraits, A, Start; A every 25 fields after that
# in case a screen still waits (jabs once the contest has started); then from 400 walking right
# into the sandbag, jabs and forward smashes (the contest started at 174 with Falco; the sandbag
# took hits from 278). The monkey goes on through the next contests.
ONE_P_CSS = [("none", 10, 20), ("start", 30), ("start", 40), (U, 100, 25), ("a", 130),
             ("start", 170)] + [("a", f) for f in range(200, 400, 25)]
HR_HITS = [(R, 400, 10)] + [("a", 412 + 10 * i) for i in range(15)] + [
    ("cx=127", 570 + 20 * i, 6) for i in range(12)]
add("homerun", {"LOCKSTEP_UNLIMITED": HOMERUN}, ONE_P_CSS + HR_HITS, 8000, None,
    "template-saved", "20")

# Kirby's copies: --matches 1210 has P1 Kirby and the other three ports the second fighter, all
# human and items off; the holds leave them standing while Kirby works on them.
for name, ck, unl in (("link", 6, KIRBY_LINK), ("clink", 21, KIRBY_LINK),
                      ("purin", 15, KIRBY_PURIN)):
    add(f"kirby{name}", fighters(32, f"4,{ck}", unl), KIRBY_COPY, 8000, 1210)
    add(f"kirby{name}bf", fighters(31, f"4,{ck}", unl), KIRBY_COPY, 8000, 1210)

# Ness's PK Thunder into himself: MATCH_FIGHTERS=11 --matches 20 (every port human) on Final
# Destination, Battlefield and Brinstar, two ports (the other Ness stands, out of the way).
for st in (32, 31, 6):
    add(f"nessupb{st}", fighters(st, 11, NESS, MATCH_PLAYERS=2), NESS_UPB, 8000, 20)

# The others: P1 the fighter and P2 the same, both human (the holds leave P2 standing as the
# script's target), items off. The seed has all four ports human, so that it gives the same
# two with or without MATCH_PLAYERS.
SCRIPTED = [
    ("yoshiroll", [32, 7, 19], 17, YOSHI, YOSHI_ROLL),
    ("purinroll", [32, 31], 15, PURIN, PURIN_ROLL),
    ("marthdb", [32, 31, 11], 9, MARTH, MARTH_DB),
    ("samusgrab", [7, 6, 3], 16, SAMUS, SAMUS_GRAB),
    ("climberss", [32, 31], 14, CLIMBERS, CLIMBERS_S),
    ("linkitem", [32, 31], 6, LINK, LINK_ITEM),
    ("clinkitem", [32, 31], 21, LINK, LINK_ITEM),
]
for name, sts, ck, unl, script in SCRIPTED:
    for st in sts:
        add(f"{name}{st}", fighters(st, ck, unl, MATCH_PLAYERS=2), script, 8000,
            seed([st], [ck], p1_human_rest(False, "off")))

# Grabs hit by a third player and shields broken: every port human (the holds move ports 1 to
# 3), the same fighter in each.
for ck in (2, 9, 16):
    add(f"grabthird{ck}", fighters(32, ck, CAPTURE), GRAB_THIRD, 6000,
        seed([32], [ck], lambda it, ps: it == -1 and not any(p[1] for p in ps)))
for ck in (8, 14):
    add(f"shieldbreak{ck}", fighters(32, ck, SHIELD), SHIELD_BREAK, 6000,
        seed([32], [ck], lambda it, ps: it == -1 and not any(p[1] for p in ps)))
add("climbersshield", fighters(32, 14, SHIELD + "," + CLIMBERS), CLIMBERS_SHIELD, 6000,
    seed([32], [14], lambda it, ps: it == -1 and not any(p[1] for p in ps)))

# Donkey Kong CPUs carrying (Cargo) each other, every port a CPU.
for n in (1, 2):
    add(f"dkcarry{n}", fighters(32 if n == 1 else 31, 1, CARRY, 600, CPU_PLAYERS=1,
                                LOCKSTEP_STALE=100000), [], 30000, 400 + n)
# Mr. Saturn among many items: matches with items at their highest.
for n in (1, 2):
    add(f"saturn{n}", fighters("32,31,8", ",".join(map(str, range(26))), SATURN, 600,
                               LOCKSTEP_STALE=100000), [], 30000,
        seed([32, 31, 8], list(range(26)), lambda it, ps: it == 4, start=500 * n))

# Master Hand and Crazy Hand both in the match with a playable fighter, in long time and long
# stamina matches.
HAND_FIGHTERS = [26, 30, 2]
def both(it, ps):
    kinds = [p[0] for p in ps]
    return kinds.count(26) == 1 and kinds.count(30) == 1 and any(not p[1] for p in ps)
add("hands", fighters(32, "26,30,2", HANDS, 600, LOCKSTEP_STALE=100000), [], 30000,
    seed([32], HAND_FIGHTERS, both))
add("handsstamina", fighters(32, "26,30,2", HANDS, 900, MATCH_STAMINA=400,
                             LOCKSTEP_STALE=100000), [], 40000,
    seed([32], HAND_FIGHTERS, both, start=1000))


def hold(presses):
    """MONKEY_HOLD for the presses: each (holds, field[, fields]) holds its comma-joined holds
    (one per port) that many fields (PRESS), overlapping holds each on its port, and nothing
    between them; the monkey takes over 300 fields after the last."""
    holds, field = [], None
    for p in sorted(presses, key=lambda p: p[1]):
        names, at = p[0], p[1]
        length = p[2] if len(p) > 2 else PRESS
        if field is not None and at > field:
            holds.append(f"none@{field}-{at - 1}")
        holds += [f"{h}@{at}-{at + length - 1}" for h in names.split(",")]
        field = max(field or 0, at + length)
    end = field + 300
    holds.append(f"none@{field}-{end}")
    return ",".join(holds), end


def main():
    prefix, cards = sys.argv[1], sys.argv[2]
    for i, (name, more, card, mode, presses, fields, matches) in enumerate(JOBS):
        job = f"{prefix}{name}"
        args = ""
        if card is not None:
            copy = os.path.join(cards, f"{job}.raw")
            shutil.copy(os.path.join(cards, f"{card}.raw"), copy)
            args += f" --card {copy}"
        args += f" --mode {mode}"
        if mode == "e":
            args += f" --matches {matches if matches is not None else 5900 + i}"
        env = {"LOCKSTEP_SEED": 1900 + i, "UNLOCK_ALL": 1, "MONKEY_B": 2, "STALL_BEATS": 200,
               "LOCKSTEP_MUTATE": 2}
        if presses:
            if mode == "e":
                presses = DEBUG_VS_START + presses
            env["MONKEY_HOLD"] = hold(presses)[0]
        env.update(more)
        envs = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"{job}|{envs}{args} --monkey {5900 + i} --fields {fields}")


if __name__ == "__main__":
    main()
