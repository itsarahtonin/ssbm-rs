"""Writes jobs that check the HSD memory card layer (card.c) where it reads, repairs and rewrites
crafted saves (tools/lockstep/cards.py --saves) and where the card is pulled between two of its
commands (CARD_REMOVE_STEP). Each job boots on its own copy of its card, presses A at the title
screen at field 400 so the main menu loads and autosaves, and checks the layer's functions on
every call (LOCKSTEP_UNLIMITED); after its script the monkey plays on.

    python tools/lockstep/campaign/mkjobs-cardlayer.py PREFIX CARDS_DIR > jobs.txt

CARDS_DIR holds template-saved.raw and, in saves/, the cards of cards.py --saves.

A step (CARD_REMOVE_STEP=N[,G]) is an entry of hsd_803AAA48 about to run a command; with the
template and this script, steps 1-22 are the boot's load (1 the open request, 2-12 its block
scans, 13 the repair and the reads after it) and 23-48 the main menu's autosave (its open, then the
header check and the writes). The numbers of the crafted cards' steps were read with CARD_STEPS=1.
"""
import os
import shutil
import sys

PRESS = 4
UNLIMITED = ("hsd_803AAA48,hsd_803A949C,fn_803AD16C,fn_803ADF90,fn_803AE7F8,fn_803AF3F0,"
             "fn_803B0120")
TITLE = [("a", 400)]
# Options, Sound, and back: the second autosave (mnsound.c:119).
SOUND = TITLE + [("down", 480), ("down", 520), ("down", 560), ("a", 600), ("down", 660),
                 ("a", 700), ("b", 800)]
BOOT_BACK, SAVE_BACK = 200, 460

# name, card (template-saved or a cards.py --saves card), presses (None: none at all, the title
# screen until the end), more environment. The blocks named are the ones each job is for.
JOBS = [
    # hsd_803AAA48 +0x814 +0x820 +0x834 +0x858 +0x868 +0x898 +0x8c8 +0xbcc +0xbd8 +0xbec +0xc10
    # +0xc20 +0xc60 +0xc90: the mirror's rebuild (READ_SECTOR, WRITE_SECTOR)
    ("v1", "v1", TITLE, {}),
    # fn_803AD16C +0x9c4 +0x9f4 +0xa28 +0xa3c, hsd_803A949C +0xe48: the third copy cleared
    ("v2", "v2", TITLE, {}),
    # hsd_803AAA48 +0x1090 +0x1094 +0x10a8 (v3a), +0x109c +0x10b0 (v3b): WRITE_HEADER's banners
    ("v3a", "v3a", TITLE, {}),
    ("v3b", "v3b", TITLE, {}),
    # hsd_803AAA48 +0x53c +0x1120 +0x11ac +0x11d0 +0x1204, hsd_803A949C +0x8c8: a header of
    # three sectors, read, checked and written
    ("v4", "v4", SOUND, {}),
    # fn_803AD16C +0x4ac +0x4c8 +0x518 +0x5a4 +0x5bc +0x648, fn_803ADF90 +0x16c +0x188 +0x1b4,
    # fn_803B0120 +0x1c0 +0x1d8 +0x1dc +0x228: a 9-block flags-3 file
    ("v5", "v5", TITLE, {}),
    # and its stale copy: fn_803AD16C +0x3a4, fn_803ADF90 +0x258, fn_803B0120 +0x31c +0x4a0 (a);
    # fn_803ADF90 +0x268 +0x270, fn_803B0120 +0x32c +0x334 (b); fn_803ADF90 +0x294,
    # fn_803B0120 +0x358 (c); fn_803B0120 +0x348 (d)
    ("v5a", "v5a", TITLE, {}),
    ("v5b", "v5b", TITLE, {}),
    ("v5c", "v5c", TITLE, {}),
    ("v5d", "v5d", TITLE, {}),
    # fn_803AE7F8 +0x484 +0x490 +0x498 +0x4a8 +0x4f0 +0x58c: a 2-block flags-0 file written
    ("v6", "v6", TITLE, {}),
    # fn_803AF3F0 +0x6dc +0x744 +0x80c +0x81c: more stale copies than blocks, freed
    ("v7", "v7", TITLE, {}),
    # fn_803AD16C +0x744 (v8a), +0xab8 (v8b), +0xab8 +0xb5c (v8c), +0xb94 +0xc30 +0xc40 (v8d),
    # +0xb94 +0xc30 +0xc38 (v8e); fn_803ADF90 +0x2e8 (v8d, v8e)
    ("v8a", "v8a", TITLE, {}),
    ("v8b", "v8b", TITLE, {}),
    ("v8c", "v8c", TITLE, {}),
    ("v8d", "v8d", TITLE, {}),
    ("v8e", "v8e", TITLE, {}),
    # fn_803ADF90 +0x2e8: a flags-3 file 0 with a stale copy. No main menu: its autosave's check
    # of file 0 compares with the game's NULL buffer for it and faults the game itself.
    ("v9", "v9", None, {}),
    # hsd_803A949C +0xe48: a new save created at the boot's prompt
    ("nosave", "nosave", TITLE, {}),
] + [
    # hsd_803AAA48 +0x1364 (steps 1, 23), +0x1548 (2-12, 24-34), +0x734 (13-21), +0x5a4 (35-36),
    # +0x35c (37-47), +0x9d0 (39-40): each step of the boot's load and the autosave
    (f"pull{n}", "template-saved", TITLE,
     {"CARD_REMOVE_STEP": f"{n},{BOOT_BACK if n <= 22 else SAVE_BACK}"}) for n in range(1, 49)
] + [
    # hsd_803AAA48 +0x454: the second and third header sectors' reads
    (f"v4pull{n}", "v4", TITLE, {"CARD_REMOVE_STEP": f"{n},{BOOT_BACK}"}) for n in (2, 3)
] + [
    # hsd_803AAA48 +0x860 (13), +0xc18 (14): the mirror's rebuild
    (f"v1pull{n}", "v1", TITLE, {"CARD_REMOVE_STEP": f"{n},{BOOT_BACK}"}) for n in (13, 14)
] + [
    # hsd_803AAA48 +0x1078 (38), +0xdd0 (39): the autosave's header rewrite and status
    (f"v3apull{n}", "v3a", TITLE, {"CARD_REMOVE_STEP": f"{n},{SAVE_BACK}"}) for n in (37, 38, 39)
] + [
    # hsd_803AAA48 +0xd70 (1), +0x1078 (2), +0x9d0 (3-12), +0xdd0 (13): each step of the create
    (f"nosavepull{n}", "nosave", TITLE, {"CARD_REMOVE_STEP": f"{n},{SAVE_BACK}"})
    for n in range(1, 15)
]


def hold(presses, fields):
    if presses is None:
        return f"none@0-{fields}", fields
    holds, field = [], 0
    for button, at in sorted(presses, key=lambda p: p[1]):
        holds += [f"none@{field}-{at - 1}", f"{button}@{at}-{at + PRESS - 1}"]
        field = at + PRESS
    end = field + 300
    holds.append(f"none@{field}-{end}")
    return ",".join(holds), end + 300


def main():
    prefix, cards = sys.argv[1], sys.argv[2]
    for i, (name, card, presses, more) in enumerate(JOBS):
        script, fields = hold(presses, 500)
        job = f"{prefix}{name}"
        copy = os.path.join(cards, f"{job}.raw")
        source = card if card == "template-saved" else os.path.join("saves", card)
        shutil.copy(os.path.join(cards, f"{source}.raw"), copy)
        env = {"LOCKSTEP_SEED": 900 + i, "MONKEY_B": 2, "STALL_BEATS": 200, "LOCKSTEP_MUTATE": 2,
               "LOCKSTEP_UNLIMITED": UNLIMITED, "MONKEY_HOLD": script, **more}
        envs = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"{job}|{envs} --card {copy} --mode 01 --monkey {3900 + i} --fields {fields}")


if __name__ == "__main__":
    main()
