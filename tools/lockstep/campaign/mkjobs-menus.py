"""Writes jobs that script menu screens' input at fixed fields (MONKEY_HOLD) from boot: the rumble
settings and VS records screens with names entered (NAMES), and the memory card screen's prompts
on crafted cards (tools/lockstep/cards.py), pulled cards and the Japanese text. Each job checks
its screen's frame function on every call (LOCKSTEP_UNLIMITED); after its script the monkey
plays on. Each job gets its own copy of its card.

    python tools/lockstep/campaign/mkjobs-menus.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds template-saved.raw and cards.py's full.raw and badicon.raw; "blank" is a card
file that doesn't exist yet, which the game reads as a broken card.
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


VIB = {"MENU": "4,0", "LOCKSTEP_UNLIMITED": "mnVibration_HandleInput"}
VIB_OPEN = [("a", 400)]
DIAG = {"MENU": "5,3", "LOCKSTEP_UNLIMITED": "mnDiagram_InputProc"}
DIAG_OPEN = [("a", 400), ("a", 500), ("a", 600)]  # Data, Records, VS records: up at 601
MC = {"LOCKSTEP_UNLIMITED": "gm_Scene_MemCard_OnFrame,gm_801AF250"}
JP = {"GAME_LANGUAGE": "jp"}

# name, environment, slot A card (None for none), --mode (None for the boot's), presses
JOBS = [
    ("vib10", {**VIB, "NAMES": 10}, "template-saved", "01", VIB_OPEN + steps(
        600, 30, ["right", "a", "a"] + ["down"] * 10 + ["up"] * 10 + ["left", "b"])),
    ("vib3", {**VIB, "NAMES": 3}, "template-saved", "01", VIB_OPEN + steps(
        600, 30, ["right", "a", "down", "down", "down", "up", "b"])),
    ("vib120", {**VIB, "NAMES": 120}, "template-saved", "01", VIB_OPEN + steps(
        600, 30, ["right"] + ["down"] * 40 + ["a"] + ["up"] * 40 + ["left", "b"])),
    ("diag12", {**DIAG, "NAMES": 12}, "template-saved", "01", DIAG_OPEN + steps(
        800, 30, ["x", "right", "a", "b", "left"] + ["right"] * 12 + ["left"] * 11
        + ["down"] * 12 + ["up"] * 11 + ["x"] + ["down"] * 25 + ["b"])),
    ("diag40", {**DIAG, "NAMES": 40, "RECORDS": 7}, "template-saved", "01", DIAG_OPEN + steps(
        800, 30, ["x", "right", "a", "b"] + ["right"] * 14 + ["down"] * 40 + ["up"] * 40
        + ["left"] * 14 + ["x", "b"])),
    # The memory card screen at boot, and again in its own mode (0x29) past the boot's prompt.
    ("mcreinsert", {**MC, "CARD_REMOVE": "1,250"}, "template-saved", None, [("a", 360)]),
    ("mcreinsertno", {**MC, "CARD_REMOVE": "1,250"}, "template-saved", None,
     [("right", 360), ("a", 420)]),
    ("mcreinsertnojp", {**MC, **JP, "CARD_REMOVE": "1,250"}, "template-saved", None,
     [("right", 360), ("a", 420)]),
    ("mcreinsert29jp", {**MC, **JP, "CARD_REMOVE": "1,900"}, "template-saved", "29",
     [("a", 300), ("a", 420), ("a", 1000)]),
    ("mcbadiconleft", MC, "badicon", None, [("left", 300), ("a", 360), ("a", 600)]),
    ("mcbadiconleftjp", {**MC, **JP}, "badicon", None, [("left", 300), ("a", 360), ("a", 600)]),
    ("mcbadicon", MC, "badicon", None, [("a", 300)]),
    ("mcbadiconjp", {**MC, **JP}, "badicon", None, [("a", 300)]),
    ("mcbadicon29", MC, "badicon", "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcbadicon29jp", {**MC, **JP}, "badicon", "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcblank29", MC, "blank", "29", [("left", 300), ("a", 360), ("a", 480), ("right", 600),
                                      ("a", 660), ("a", 780), ("right", 900), ("a", 960)]),
    ("mcblank29jp", {**MC, **JP}, "blank", "29", [
        ("left", 300), ("a", 360), ("a", 480), ("right", 600), ("a", 660), ("a", 780),
        ("right", 900), ("a", 960)]),
    ("mcblankformat29", MC, "blank", "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcblankformat29jp", {**MC, **JP}, "blank", "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcfull", MC, "full", None, [("a", 300)]),
    ("mcfull29", MC, "full", "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcfull29jp", {**MC, **JP}, "full", "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcfulljp", {**MC, **JP}, "full", None, [("a", 300)]),
    ("mcnone29jp", {**MC, **JP}, None, "29", [("a", 300), ("a", 420), ("a", 800)]),
    ("mcnone29jpdebug", {**MC, **JP, "DBLEVEL": 3}, None, "29", [
        ("a", 300), ("a", 420), ("l+r+a", 800), ("r", 860), ("l", 920)]),
] + [
    # A card pulled while a delete, create or format waits on it.
    (f"mcpulldelete{d}", {**MC, "CARD_REMOVE": f"{360 + d},{420 + d}"}, "badicon", None,
     [("left", 300), ("a", 360), ("a", 600)]) for d in (1, 2, 3, 5, 8)
] + [
    (f"mcpullformat{d}", {**MC, "CARD_REMOVE": f"{360 + d},{420 + d}"}, "blank", None,
     [("left", 300), ("a", 360), ("a", 480)]) for d in (1, 2, 3, 5, 8)
] + [
    (f"mcpullcreate{d}", {**MC, "CARD_REMOVE": f"{480 + d},{540 + d}"}, "blank", None,
     [("left", 300), ("a", 360), ("a", 480), ("a", 660)]) for d in (1, 2, 3, 5, 8)
]


def hold(presses):
    holds, field = [], 0
    for button, at in sorted(presses, key=lambda p: p[1]):
        holds += [f"none@{field}-{at - 1}", f"{button}@{at}-{at + PRESS - 1}"]
        field = at + PRESS
    end = field + 300
    holds.append(f"none@{field}-{end}")
    return ",".join(holds), end


def main():
    prefix, cards = sys.argv[1], sys.argv[2]
    for i, (name, more, card, mode, presses) in enumerate(JOBS):
        script, end = hold(presses)
        job = f"{prefix}{name}"
        args = ""
        if card is not None:
            copy = os.path.join(cards, f"{job}.raw")
            if card == "blank":
                if os.path.exists(copy):
                    os.remove(copy)
            else:
                shutil.copy(os.path.join(cards, f"{card}.raw"), copy)
            args += f" --card {copy}"
        if mode is not None:
            args += f" --mode {mode}"
        env = {"LOCKSTEP_SEED": 800 + i, "DBLEVEL": 4, "UNLOCK_ALL": 1, "MONKEY_B": 2,
               "STALL_BEATS": 200, "LOCKSTEP_MUTATE": 2, "MONKEY_HOLD": script, **more}
        envs = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"{job}|{envs}{args} --monkey {3800 + i} --fields {end + 1500}")


if __name__ == "__main__":
    main()
