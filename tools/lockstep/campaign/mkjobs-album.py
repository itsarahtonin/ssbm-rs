"""Writes jobs that script the snapshot album's (fn_802545C4, mnsnap.c) menus: each opens the
album from the Data menu on a card of snapshots and presses one sequence of buttons, then leaves
the rest of its fields to the monkey. The album costs about a million instructions a frame, so the
jobs check it on every call (LOCKSTEP_UNLIMITED). Each job gets its own copy of its card.

    python tools/lockstep/campaign/mkjobs-album.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds the cards `tools/lockstep/cards.py --snaps` makes (template-snaps.raw beside
them); the copies go there too, as PREFIX-NAME.raw.
"""
import os
import shutil
import sys

# From boot with MENU=5,0: A on Data, A on the album, A on slot A; the album takes input from
# about field 900, once its first page of thumbnails has loaded.
OPEN = [("a", 400), ("a", 500), ("a", 600)]
START, STEP, PRESS = 900, 60, 4

MENU = ["a"]  # opens the snapshot's menu on View
MOVE = MENU + ["down", "a"]
COPY = MENU + ["down"] * 2 + ["a"]
DELETE = MENU + ["down"] * 3 + ["a"]
OTHER = MENU + ["down"] * 4 + ["a"]

# name, card, presses (a button, button+button, or "wait"), extra environment
JOBS = [
    ("move", "template-snaps", MOVE + ["right", "right", "a", "wait"] + MOVE + ["b", "b"]),
    ("movepage", "template-snaps", MOVE + ["right"] * 5 + ["down", "a"]),
    ("moveback", "template-snaps", ["right"] * 5 + MOVE + ["left"] * 5 + ["up", "a", "b"]),
    ("menu1", "template-snaps", MENU + ["down", "a", "b", "b", "b"]),
    ("menu5", "template-snaps", MENU + ["down"] * 5 + ["up", "a", "b", "b"]),
    ("copy", "template-snaps", COPY + ["a", "wait", "wait", "a", "wait", "b", "b"]),
    ("copyb", "template-snaps", COPY + ["a", "wait", "wait", "b", "b", "b"]),
    ("other", "template-snaps", OTHER + ["a", "wait", "wait", "wait", "b", "b"]),
    ("delete", "template-snaps", DELETE + ["left", "a", "wait", "wait", "b"]),
    ("deleteno", "template-snaps", DELETE + ["a", "b", "b"]),
    ("deletelast", "snap5", ["right"] * 4 + DELETE + ["left", "a", "wait", "wait"]),
    ("deleteonly", "snap1", DELETE + ["left", "a", "wait", "wait", "wait"]),
    ("cursor", "snap5", ["right", "right", "down", "up", "left", "r", "l", "right", "down",
                         "right", "right", "right", "left", "up", "l", "r", "b"]),
    ("cursor24", "template-snaps", ["down"] * 7 + ["r"] * 4 + ["up"] * 3 + ["l"] * 6
                                   + ["left", "left", "right"] * 3),
    ("damaged", "snapdamaged", ["a", "wait", "a", "wait", "wait", "right", "left"]),
    ("damagedno", "snapdamaged", ["a", "wait", "right", "a", "wait", "b"]),
    ("damagedonly", "snap1damaged", ["a", "wait", "a", "wait", "wait", "wait"]),
    ("damagedpull", "snapdamaged", ["a", "wait", "a", "wait", "wait"],
     {"CARD_REMOVE": f"{START + 2 * STEP},{START + 5 * STEP}"}),
    # Slot B (--card-b): template-saved has room and no snapshots, template-snaps no room.
    ("bcopy", "template-snaps", COPY + ["a", "wait", "wait", "wait", "wait", "b"],
     {"card_b": "template-saved"}),
    ("bcopyfull", "template-snaps", COPY + ["a", "wait", "wait", "a", "wait", "b"],
     {"card_b": "template-snaps"}),
    ("bcopysame", "snap5", COPY + ["a", "wait", "wait", "a", "wait", "b"], {"card_b": "snap5"}),
    ("bcopyblank", "template-snaps", COPY + ["a", "wait", "wait", "a", "wait", "b"],
     {"card_b": "blank"}),
    ("bcopydamaged", "snapdamaged", ["right"] + COPY + ["a", "wait", "wait", "wait", "b"],
     {"card_b": "template-saved"}),
    ("bcopypull", "template-snaps", COPY + ["a", "wait", "wait", "wait", "wait", "a"],
     {"card_b": "template-saved", "CARD_REMOVE_B": f"{START + 7 * STEP},{START + 10 * STEP}"}),
    ("binsert", "template-snaps", COPY + ["a", "wait", "wait", "wait", "a", "wait", "wait"],
     {"card_b": "template-saved", "CARD_REMOVE_B": f"1,{START + 8 * STEP}"}),
    ("binsertother", "template-snaps", OTHER + ["a", "wait", "wait", "wait", "wait", "b"],
     {"card_b": "snap5", "CARD_REMOVE_B": f"1,{START + 8 * STEP}"}),
    ("bother", "template-snaps", OTHER + ["a", "wait", "wait", "wait", "right", "down", "b",
                                          "b"], {"card_b": "snap5"}),
    ("botherblank", "template-snaps", OTHER + ["a", "wait", "wait", "wait", "b", "b"],
     {"card_b": "blank"}),
    ("botherempty", "template-snaps", OTHER + ["a", "wait", "wait", "wait", "b", "b"],
     {"card_b": "template-saved"}),
    ("bselect", "template-snaps", MOVE + ["right", "a", "wait"] + DELETE + ["left", "a", "wait",
                                                                            "wait", "b", "b"],
     {"card_b": "snap5", "slot_b": True}),
    ("bselectcopy", "template-snaps", COPY + ["a", "wait", "wait", "a", "wait", "wait", "b"],
     {"card_b": "snap5", "slot_b": True}),
    ("bselectother", "snap5", OTHER + ["a", "wait", "wait", "wait", "right", "b", "b"],
     {"card_b": "template-snaps", "slot_b": True}),
    ("bselectpull", "template-snaps", ["right", "wait", "wait", "wait", "wait"],
     {"card_b": "snap5", "slot_b": True, "CARD_REMOVE_B": f"{START + 2 * STEP},{START + 4 * STEP}"}),
    ("bonly", None, ["wait", "right", "a", "b", "b"], {"card_b": "snap5"}),
    ("bonlyswitch", "template-snaps", ["wait", "b", "wait", "wait", "a", "right", "b"],
     {"card_b": "snap5", "CARD_REMOVE": f"{START + 2 * STEP},{START + 8 * STEP}"}),
    ("pull4", "template-snaps", ["right", "wait", "wait", "wait", "wait", "wait"],
     {"CARD_REMOVE": f"{START + 2 * STEP},{START + 4 * STEP}"}),
    ("pull2", "template-snaps", ["wait"], {"CARD_REMOVE": "560,700"}),
    ("pulldelete", "template-snaps", DELETE + ["left", "wait", "a", "wait", "wait", "wait"],
     {"CARD_REMOVE": f"{START + 6 * STEP},{START + 10 * STEP}"}),
    ("pullmove", "template-snaps", MOVE + ["right", "wait", "a", "wait", "wait"],
     {"CARD_REMOVE": f"{START + 4 * STEP},{START + 8 * STEP}"}),
]


def hold(presses, slot_b=False):
    holds, field = [], 0
    for button, at in OPEN[:2] + [("right", 560)] * slot_b + OPEN[2:]:
        holds += [f"none@{field}-{at - 1}", f"{button}@{at}-{at + PRESS - 1}"]
        field = at + PRESS
    at = START
    for button in presses:
        holds.append(f"none@{field}-{at - 1}")
        if button != "wait":
            holds.append(f"{button}@{at}-{at + PRESS - 1}")
            field = at + PRESS
        else:
            field = at
        at += STEP
    holds.append(f"none@{field}-{at + STEP}")
    return ",".join(holds), at + STEP


def main():
    prefix, cards = sys.argv[1], sys.argv[2]
    for i, (name, card, presses, *more) in enumerate(JOBS):
        more = dict(more[0]) if more else {}
        card_b, slot_b = more.pop("card_b", None), more.pop("slot_b", False)
        script, end = hold(presses, slot_b)
        job = f"{prefix}{name}"
        args = ""
        # Each slot's card a copy of its own; "blank" is a card file that doesn't exist yet,
        # blank flash the game reads as a broken card.
        for flag, src, suffix in (("--card", card, ""), ("--card-b", card_b, "-b")):
            if src is None:
                continue
            copy = os.path.join(cards, f"{job}{suffix}.raw")
            if src == "blank":
                if os.path.exists(copy):
                    os.remove(copy)
            else:
                shutil.copy(os.path.join(cards, f"{src}.raw"), copy)
            args += f" {flag} {copy}"
        env = {"LOCKSTEP_SEED": 700 + i, "DBLEVEL": 4, "UNLOCK_ALL": 1, "MONKEY_B": 2,
               "STALL_BEATS": 200, "LOCKSTEP_MUTATE": 2, "LOCKSTEP_UNLIMITED": "fn_802545C4",
               "MENU": "5,0", "MONKEY_HOLD": script, **more}
        envs = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"{job}|{envs}{args} --monkey {3700 + i} --mode 01 --fields {end + 1500}")


if __name__ == "__main__":
    main()
