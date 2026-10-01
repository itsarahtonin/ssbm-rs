"""Lists functions for ssbm-run's probes: those coverage leaves below the bar whose parameters a
probe can give, a game object and numbers.

    python tools/lockstep/probes.py <decomp root> local/typegen/types.json REPORT_CSV OUT
        [--bar 0.9]

A probe calls the function, under lockstep, on a live object of the kind its unit works on: a
fighter of its fighter kind (any fighter for common code), an item, or the stage's ground
objects; or with no object, for functions that take only numbers. Each line of OUT is
`address form class kind params # name`: the form says what the first parameter is (g a game
object, u the object's user data, such as its Fighter, n no object), the class is the object's
GObj class or -1 for any, the kind a fighter kind or -1 for any, and params one letter per
further parameter, i for an integer and f for a float.
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
# The GObj classes whose user data is each of these.
USER_DATA = {"Fighter": CLASS_FIGHTER, "Item": CLASS_ITEM, "Ground": CLASS_GROUND}


def unit_class(unit):
    """The GObj class a unit's functions work on (-1 if it isn't known), and the fighter kind
    (-1 for any)."""
    m = re.match(r"melee/ft/kinds/(ft\w+)/", unit)
    if m:
        return CLASS_FIGHTER, FIGHTER_KINDS.get(m.group(1), -1)
    if unit.startswith("melee/ft/"):
        return CLASS_FIGHTER, -1
    if unit.startswith("melee/it/"):
        return CLASS_ITEM, -1
    if unit.startswith("melee/gr/"):
        return CLASS_GROUND, -1
    return -1, -1


def record_name(t, records):
    if t.get("k") != "ptr":
        return None
    to = t.get("to") or {}
    return records.get(to.get("id"), {}).get("name") if to.get("k") == "rec" else None


def letters(params):
    """One letter per numeric parameter, or None if one is not a number."""
    out = []
    for p in params:
        if p.get("k") in ("int", "enum") and p.get("size", 4) <= 4:
            out.append("i")
        elif p.get("k") == "float":
            out.append("f")
        else:
            return None
    return "".join(out) or "-"


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
    sigs = {f["addr"]: f["type"] for f in types["functions"] if f.get("addr") is not None}
    lines, skipped = [], {}
    for r in csv.DictReader(open(args.report, encoding="utf-8")):
        if float(r["share"]) >= args.bar and int(r["mismatches"]) == 0:
            continue
        addr = int(r["address"], 16)
        cls, kind = unit_class(r["unit"])
        ft = sigs.get(addr)
        if ft is None or ft.get("variadic"):
            skipped["no prototype"] = skipped.get("no prototype", 0) + 1
            continue
        params = ft.get("params") or []
        first = record_name(params[0], records) if params else None
        if first == "HSD_GObj":
            form, rest = "g", letters(params[1:])
        elif first in USER_DATA:
            form, cls, rest = "u", USER_DATA[first], letters(params[1:])
            kind = kind if cls == CLASS_FIGHTER else -1
        else:
            form, cls, kind, rest = "n", -1, -1, letters(params)
        if rest is None:
            skipped["other parameters"] = skipped.get("other parameters", 0) + 1
            continue
        lines.append(f"{addr:#010x} {form} {cls} {kind} {rest} # {r['name']}")
    with open(args.out, "w", encoding="utf-8", newline="\n") as w:
        w.write("# Functions to probe: address, object form, object class, fighter kind, parameters\n")
        w.write("\n".join(lines) + "\n")
    print(f"{len(lines)} functions to probe; left out: " +
          ", ".join(f"{n} {why}" for why, n in sorted(skipped.items(), key=lambda x: -x[1])))


if __name__ == "__main__":
    main()
