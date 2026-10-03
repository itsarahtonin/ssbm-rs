"""Writes the coverage map page: the latest report, and the history of every round's.

    python tools/lockstep/campaign/treemap.py LABEL OUT_HTML

report.csv (as report-all.sh last wrote it) is the map's latest state, named LABEL; the rounds
before it are history/NN-LABEL.csv, as history.sh saves them, which the page steps through.
reviewed.txt marks the mismatches with changed inputs found to come from the inputs.
"""
import csv
import datetime
import glob
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
# The campaign's state, and the review list beside the gap ledger.
L = os.path.join(ROOT, "local", "lockstep")
REVIEWED = os.path.join(ROOT, "tools", "lockstep", "reviewed.txt")
label, out = sys.argv[1], sys.argv[2]

reviewed = set()
for line in open(REVIEWED, encoding="utf-8"):
    w = line.split("#")[0].split()
    if w:
        reviewed.add(w[-1] if w[0].startswith("0x") else w[0])

latest = list(csv.DictReader(open(os.path.join(L, "report.csv"), encoding="utf-8")))
units, unit_index, rows = [], {}, []
for r in latest:
    u = r["unit"]
    if u not in unit_index:
        unit_index[u] = len(units)
        units.append(u)
    rows.append([r["name"], unit_index[u], int(r["countable"] or 0), int(r["verified_blocks"] or 0),
                 int(r["calls"] or 0), int(r["mismatches"] or 0), int(r["mutated"] or 0),
                 1 if r["name"] in reviewed else 0, int(r.get("explained") or 0)])
# Static functions of different units can share a name (stage files' callbacks, mostly), so a
# function is its name and unit.
order = {(r[0], units[r[1]]): i for i, r in enumerate(rows)}


def snapshot(name, report):
    """A round's state, three numbers a function in the latest's order: blocks verified,
    blocks explained, and flags (1 called, 2 mismatches, 4 mismatches with changed inputs not
    reviewed)."""
    v = [0] * (3 * len(rows))
    for r in report:
        i = order.get((r["name"], r["unit"]))
        if i is None:
            continue
        flags = (1 if int(r["calls"] or 0) > 0 else 0) | (2 if int(r["mismatches"] or 0) > 0 else 0) \
            | (4 if int(r["mutated"] or 0) > 0 and r["name"] not in reviewed else 0)
        v[3 * i:3 * i + 3] = [int(r["verified_blocks"] or 0), int(r.get("explained") or 0), flags]
    return {"label": name, "v": v}


snaps = []
for path in sorted(glob.glob(os.path.join(L, "history", "*.csv"))):
    name = os.path.basename(path)[:-4].split("-", 1)[1].replace("_", " ")
    if name != label:
        snaps.append(snapshot(name, csv.DictReader(open(path, encoding="utf-8"))))
snaps.append(snapshot(label, latest))

data = {"units": units, "fns": rows, "snaps": snaps, "bar": 0.9,
        "updated": datetime.datetime.now().strftime("%Y-%m-%d %H:%M")}
template = open(os.path.join(HERE, "treemap.template.html"), encoding="utf-8").read()
open(out, "w", encoding="utf-8", newline="\n").write(
    template.replace("/*DATA*/null", json.dumps(data, separators=(",", ":"))))
print(f"{len(rows)} functions, {len(units)} units, {len(snaps)} rounds -> {out}")
