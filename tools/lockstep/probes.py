"""Lists functions for ssbm-run's probes: those coverage leaves below the bar whose parameters a
probe can give: game objects, their parts, numbers and scratch memory.

    python tools/lockstep/probes.py <decomp root> local/typegen/types.json REPORT_CSV OUT
        [--bar 0.9] [--dead FILE]

A probe calls the function, under lockstep, with live game objects of the kind its unit works
on (a fighter of its fighter kind, any fighter for common code, an item, the stage's ground
objects) and small numbers. Each line of OUT is `address class kind params # name`: the class
is the GObj class of its first object parameter (-1 for any), the kind a fighter kind (-1 for
any), and params one letter per parameter:

    g  a game object (HSD_GObj)
    F  a fighter (its Fighter), I  an item (its Item), R  a ground (its Ground)
    j  a joint (HSD_JObj), c  a camera (HSD_CObj), l  a light (HSD_LObj): a game object's own
    p  scratch memory, for any other pointer or a struct passed by value
    x  a function that returns at once, for a function pointer
    i  an integer, b/B  an unsigned/signed byte, h/H  an unsigned/signed halfword,
    w  a 64-bit integer, f  a float

or `-` for none. --dead adds the functions reach.py found nothing calls (`name unit` lines), which
only probes can check.
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
# Code a call out of context says nothing about: it jumps elsewhere for good, or saves and loads
# the CPU's registers, whose contents ports don't keep as the original's are.
NOT_PROBED = re.compile(r"longjmp|setjmp|Context|SwitchThread|SelectThread|Exception|Interrupt"
                        r"Handler|__OSDispatchInterrupt|OSResetSystem|__start")
# Letters for the records a probe can give.
RECORDS = {"HSD_GObj": "g", "Fighter": "F", "Item": "I", "Ground": "R", "HSD_JObj": "j",
           "HSD_CObj": "c", "HSD_LObj": "l"}


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


def letter(p, records):
    k = p.get("k")
    if k in ("int", "enum"):
        size = p.get("size", 4)
        if size == 1:
            return "B" if p.get("signed") else "b"
        if size == 2:
            return "H" if p.get("signed") else "h"
        return "i" if size <= 4 else "w"
    if k == "float":
        return "f"
    if k == "rec":
        return "p"
    if k == "ptr":
        to = p.get("to") or {}
        if to.get("k") == "fn":
            return "x"
        name = records.get(to.get("id"), {}).get("name") if to.get("k") == "rec" else None
        return RECORDS.get(name, "p")
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("types")
    ap.add_argument("report")
    ap.add_argument("out")
    ap.add_argument("--bar", type=float, default=0.9)
    ap.add_argument("--dead")
    args = ap.parse_args()
    types = json.load(open(args.types, encoding="utf-8"))
    records = types["records"]
    sigs = {f["addr"]: f["type"] for f in types["functions"] if f.get("addr") is not None}
    targets = [r for r in csv.DictReader(open(args.report, encoding="utf-8"))
               if float(r["share"]) < args.bar or int(r["mismatches"]) > 0]
    if args.dead:
        addrs = {(f["name"], f.get("tu")): f["addr"] for f in types["functions"]
                 if f.get("addr") is not None and f.get("defined")}
        for line in open(args.dead, encoding="utf-8"):
            w = line.split("#")[0].split()
            if len(w) == 2 and (w[0], w[1]) in addrs:
                targets.append({"address": hex(addrs[w[0], w[1]]), "name": w[0], "unit": w[1]})
    lines, skipped = [], {}
    for r in targets:
        addr = int(r["address"], 16)
        if NOT_PROBED.search(r["name"]):
            skipped["control flow or registers"] = skipped.get("control flow or registers", 0) + 1
            continue
        ft = sigs.get(addr)
        if ft is None or ft.get("variadic"):
            skipped["no prototype"] = skipped.get("no prototype", 0) + 1
            continue
        params = [letter(p, records) for p in ft.get("params") or []]
        if None in params:
            skipped["other parameter types"] = skipped.get("other parameter types", 0) + 1
            continue
        cls, kind = unit_class(r["unit"])
        first = next((x for x in params if x in "gFIR"), None)
        if first in ("F", "I", "R"):
            cls = {"F": CLASS_FIGHTER, "I": CLASS_ITEM, "R": CLASS_GROUND}[first]
            kind = kind if first == "F" else -1
        lines.append(f"{addr:#010x} {cls} {kind} {''.join(params) or '-'} # {r['name']}")
    with open(args.out, "w", encoding="utf-8", newline="\n") as w:
        w.write("# Functions to probe: address, object class, fighter kind, parameters\n")
        w.write("\n".join(lines) + "\n")
    print(f"{len(lines)} functions to probe; left out: " +
          ", ".join(f"{n} {why}" for why, n in sorted(skipped.items(), key=lambda x: -x[1])))


if __name__ == "__main__":
    main()
