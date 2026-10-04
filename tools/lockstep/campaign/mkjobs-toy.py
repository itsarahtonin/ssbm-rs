"""Writes jobs for the trophy screens' debug procs (melee/ty/toy, tydisplay), which a button held
as the scene starts installs at debug level 3 or more: Z in the gallery (_Toy_8030B530, the
viewer), L in the gallery (_Toy_8030E110), R in the gallery (_Toy_80310B48, the trophy count
editor) and Z in the collection (_tyDisplay_8031A94C). The scene starts on the title screen's
Start (field 100), with the button held from boot through it; then a script of holds, one after
another, each followed by a few idle fields, and the monkey after that.

    python tools/lockstep/campaign/mkjobs-toy.py PREFIX CARD > jobs.txt

CARD is a card with Melee's save (template-saved.raw); each job gets its own copy beside it.
"""
import os
import shutil
import sys

GAP = 12


def seq(start, steps):
    """HOLD:FIELDS steps one after another from `start`, GAP idle fields after each."""
    holds, at = [], start
    for step in steps:
        hold, n = step.rsplit(":", 1) if ":" in step else (step, "4")
        holds.append(f"{hold}@{at}-{at + int(n) - 1}")
        holds.append(f"none@{at + int(n)}-{at + int(n) + GAP - 1}")
        at += int(n) + GAP
    return holds, at


def opening(button):
    return [f"{button}@0-99", f"start+{button}@100-103", f"{button}@104-200", "none@201-239"]


def ports(buttons):
    """Each button pressed alone on ports 2, 3 and 4, port 1 idle."""
    return [f"p{p}+{b}" for b in buttons for p in (2, 3, 4)]


VIEWER = (["none:30", "b+right", "b+left"] + ["start"] * 6 + ["l", "l", "r"]
          + ["x+sy=127:80", "x+sy=-128:80", "sy=127:40", "sy=-128:40", "sx=127:130",
             "sx=-128:130", "sy=-128:30", "a+sx=90+sy=90:20", "a+sx=-90+sy=-90:20", "none:20"]
          + ports(["a", "b", "x", "y", "z", "l", "r", "start", "up", "down", "left", "right"])
          + ["p2+up:8", "p2+down:8", "p2+start", "none:20"])
EDITOR_L = (["l:4", "none:20", "l", "none:20", "r", "none:20", "r", "none:20", "r", "none:20",
             "l", "none:20"] + ["start", "none:20"] * 6
            + ["a", "none:20", "b", "none:30", "a+sx=100:20", "a+sy=-100:20", "x+sy=127:80",
               "x+sy=-128:80", "cx=127:130", "cx=-128:130", "cy=127:40", "cy=-128:40"]
            + ports(["l", "r", "a", "b", "x", "y", "start"]) + ["none:30"])
COUNT_UP = ["y:600", "x:30", "sy=-100:10", "sy=-100:10", "sy=100:10"] + ["sy=-100:10"] * 10
COUNT_DOWN = ["right:20", "x:900", "x:10"]
COLLECTION = (["none:10", "left:20", "right:20", "up:20", "down:20", "left+right:20",
               "left+up:20", "left+down:20", "a+sx=-100:20", "a+sy=-100:20", "a+cx=-100:20",
               "a+cy=-100:20", "a+sx=100+sy=100:20", "sx=-128:60", "sy=-128:60",
               "cx=127:150", "cx=-128:150", "cy=127:60", "cy=-128:60"]
              + ["l:4"] * 2 + ["r:4"] * 3 + ["x:200", "y:200", "start", "b:130"])

# name, --mode, held at the start, script, extra environment
JOBS = [
    ("viewer", "0b", "z", VIEWER),
    ("viewerports", "0b", "z", ports(["a", "b", "x", "y", "z", "l", "r", "start"]) * 2),
    ("editl", "0b", "l", EDITOR_L),
    ("countstart", "0b", "r", COUNT_UP + ["start"] + ["none:200"]),
    ("counta", "0b", "r", COUNT_DOWN + ["a"] + ["none:200"]),
    ("countb", "0b", "r", ports(["y", "x", "up", "down", "left", "right"]) + ["b"]),
    ("countports", "0b", "r", ports(["a", "start"])),
    ("collection", "0d", "z", COLLECTION),
]


def main():
    prefix, card = sys.argv[1], sys.argv[2]
    for i, (name, mode, button, script) in enumerate(JOBS):
        holds, end = seq(240, script)
        job = f"{prefix}{name}"
        copy = os.path.join(os.path.dirname(card), f"{job}.raw")
        shutil.copy(card, copy)
        env = {"LOCKSTEP_SEED": 900 + i, "DBLEVEL": 4, "UNLOCK_ALL": 1, "MONKEY_B": 2,
               "STALL_BEATS": 200, "LOCKSTEP_MUTATE": 2,
               "LOCKSTEP_UNLIMITED": "_Toy_8030B530,_Toy_8030E110,_Toy_80310B48,_tyDisplay_8031A94C",
               "MONKEY_HOLD": ",".join(opening(button) + holds + [f"none@{end}-{end + 60}"])}
        envs = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"{job}|{envs} --card {copy} --monkey {3900 + i} --mode {mode} --fields {end + 2000}")


if __name__ == "__main__":
    main()
