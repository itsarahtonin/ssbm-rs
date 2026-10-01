"""Lists functions for ssbm-run's probes: those coverage leaves below the bar whose parameters a
probe can give, a game object and numbers.

    python tools/lockstep/probes.py <decomp root> local/typegen/types.json REPORT_CSV OUT
        [--bar 0.9]

A probe calls the function, under lockstep, on a live object of the kind its unit works on: a
fighter of its fighter kind (any fighter for common code), an item, or the stage's ground
objects. Each line of OUT is `address class kind params # name`: the class is the object's
GObj class, the kind a fighter kind or -1 for any, and params one letter per parameter after
the object, i for an integer and f for a float.
"""

import argparse
import csv
import json
import re

# Fighter kinds by the directory of their code.
FIGHTER_KINDS = {
    "ftMario": 0, "ftFox": 1, "ftCaptain": 2, "ftDonkey": 3, "ftKirby": 4, "ftKoopa": 5,
    "ftLink": 6, "ftSeak": 7, "ftNess": 8, "ftPeach": 9, "ftPopo": 10, "ftNana": 11,
    "ftPikachu": 12, "ftSamus": 13, "ftYoshi": 14, "ftPurin": 15, "ftMewtwo": 16, "ftLuigi": 17,
    "ftMars": 18, "ftZelda": 19, "ftCLink": 20, "ftDrMario": 21, "ftFalco": 22, "ftPichu": 23,
    "ftGameWatch": 24, "ftGanon": 25, "ftEmblem": 26, "ftMasterHand": 27, "ftCrazyHand": 28,
    "ftZakoBoy": 29, "ftZakoGirl": 30, "ftGigaKoopa": 31, "ftSandbag": 32,
}
CLASS_FIGHTER, CLASS_ITEM, CLASS_GROUND = 4, 6, 13


def object_class(unit):
    """The GObj class a unit's functions work on, and the fighter kind, if any."""
    m = re.match(r"melee/ft/kinds/(ft\w+)/", unit)
    if m:
        return CLASS_FIGHTER, FIGHTER_KINDS.get(m.group(1), -1)
    if unit.startswith("melee/ft/"):
        return CLASS_FIGHTER, -1
    if unit.startswith("melee/it/"):
        return CLASS_ITEM, -1
    if unit.startswith("melee/gr/"):
        return CLASS_GROUND, -1
    return None, None


def is_gobj(t, records):
    if t.get("k") != "ptr":
        return False
    to = t.get("to") or {}
    return to.get("k") == "rec" and records.get(to.get("id"), {}).get("name") == "HSD_GObj"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("types")
    ap.add_argument("report")
    ap.add_argument("out")
    ap.add_argument("--bar", type=float, default=0.9)
    args = ap.parse_args()
    types = json.load(open(args.types, encoding="utf-8"))
    records = types["records"]
    sigs = {}
    for f in types["functions"]:
        if f.get("addr") is not None:
            sigs[f["addr"]] = f["type"]
    lines, skipped = [], {}
    for r in csv.DictReader(open(args.report, encoding="utf-8")):
        if float(r["share"]) >= args.bar and int(r["mismatches"]) == 0:
            continue
        addr = int(r["address"], 16)
        cls, kind = object_class(r["unit"])
        ft = sigs.get(addr)
        why = None
        if cls is None:
            why = "no object class"
        elif ft is None or ft.get("variadic"):
            why = "no prototype"
        else:
            params = ft.get("params") or []
            if not params or not is_gobj(params[0], records):
                why = "first parameter not an object"
            else:
                letters = []
                for p in params[1:]:
                    if p.get("k") in ("int", "enum") and p.get("size", 4) <= 4:
                        letters.append("i")
                    elif p.get("k") == "float":
                        letters.append("f")
                    else:
                        why = "other parameters"
                        break
                if why is None:
                    lines.append(f"{addr:#010x} {cls} {kind} {''.join(letters) or '-'} # {r['name']}")
        if why:
            skipped[why] = skipped.get(why, 0) + 1
    with open(args.out, "w", encoding="utf-8", newline="\n") as w:
        w.write("# Functions to probe: address, object class, fighter kind, parameters\n")
        w.write("\n".join(lines) + "\n")
    print(f"{len(lines)} functions to probe; left out: " +
          ", ".join(f"{n} {why}" for why, n in sorted(skipped.items(), key=lambda x: -x[1])))


if __name__ == "__main__":
    main()
