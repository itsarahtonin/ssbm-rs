"""Writes the levers page: what each way of verifying more blocks has yielded, next to the
coverage map.

    python tools/lockstep/campaign/levers.py BASE OUT_HTML

Each run in local/lockstep/levers.tsv (date, lever, run, known bitmap, results, corpus collection
log or empty, note) counts the instructions its results verified beyond what the known bitmap
held, and its cost as the original instructions it ran, in billions, from its log (and its
corpus collection's). The open blocks over time come from the history entries history.sh and
snapshot.sh save, and from report.csv as report-all.sh last wrote it; BASE names the history
entry the page counts gains from (the last finished round). levers-notes.tsv holds dated
findings.
"""
import csv
import datetime
import glob
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
L = os.path.join(ROOT, "local", "lockstep")
base_label, out = sys.argv[1], sys.argv[2]


def words(path):
    b = open(path, "rb").read()
    return [int.from_bytes(b[i:i + 8], "little") for i in range(0, len(b), 8)]


def beyond(results, known):
    try:
        w = words(os.path.join(L, results + ".bin"))
    except FileNotFoundError:
        return None
    k = words(os.path.join(L, known))
    return sum(bin(x & ~(k[i] if i < len(k) else 0)).count("1") for i, x in enumerate(w))


def cost(log):
    """Billions of original instructions the run's log reports, or None."""
    try:
        text = open(os.path.join(L, log + ".txt"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return None
    m = re.search(r"(\d+) fields, (\d+) M instructions", text)
    return int(m.group(2)) / 1000 if m else None


runs = []
for row in csv.reader(open(os.path.join(L, "levers.tsv"), encoding="utf-8"), delimiter="\t"):
    if not row or row[0].startswith("#"):
        continue
    date, lever, run, known, results, collect, note = (row + [""] * 7)[:7]
    new, spent = beyond(results, known), cost(results)
    if new is None or spent is None:
        continue
    collected = cost(collect) if collect else None
    runs.append({"date": date, "lever": lever, "run": run, "new": new,
                 "cost": round(spent + (collected or 0), 2),
                 "collect": round(collected, 2) if collected else None, "note": note})


def totals(rows):
    t = {"countable": 0, "verified": 0, "explained": 0}
    for r in rows:
        t["countable"] += int(r["countable"] or 0)
        t["verified"] += int(r["verified_blocks"] or 0)
        t["explained"] += int(r.get("explained") or 0)
    t["open"] = t["countable"] - t["verified"] - t["explained"]
    return t


timeline, base_rows = [], None
for path in sorted(glob.glob(os.path.join(L, "history", "*.csv"))):
    label = os.path.basename(path)[:-4].split("-", 1)[1].replace("_", " ")
    rows = list(csv.DictReader(open(path, encoding="utf-8")))
    timeline.append({"label": label, **totals(rows)})
    if label == base_label:
        base_rows = rows
latest = list(csv.DictReader(open(os.path.join(L, "report.csv"), encoding="utf-8")))
now = totals(latest)
if not timeline or timeline[-1]["open"] != now["open"]:
    timeline.append({"label": "now", **now})
base = totals(base_rows) if base_rows else now

# Where the blocks verified or explained since the base landed, by function.
before = {r["name"]: int(r["verified_blocks"] or 0) + int(r.get("explained") or 0)
          for r in base_rows or []}
gains = []
for r in latest:
    g = int(r["verified_blocks"] or 0) + int(r.get("explained") or 0) - before.get(r["name"], 0)
    if g > 0:
        gains.append({"name": r["name"], "unit": r["unit"], "gained": g,
                      "open": int(r["countable"]) - int(r["verified_blocks"])
                      - int(r.get("explained") or 0)})
gains.sort(key=lambda g: -g["gained"])

notes = [{"date": row[0], "text": row[1]}
         for row in csv.reader(open(os.path.join(L, "levers-notes.tsv"), encoding="utf-8"),
                               delimiter="\t")
         if row and not row[0].startswith("#") and len(row) > 1]

data = {"updated": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        "base": base_label, "baseTotals": base, "now": now, "timeline": timeline,
        "runs": runs, "gains": gains[:20], "gainedFunctions": len(gains), "notes": notes}
template = open(os.path.join(HERE, "levers.template.html"), encoding="utf-8").read()
open(out, "w", encoding="utf-8", newline="\n").write(
    template.replace("/*DATA*/null", json.dumps(data, separators=(",", ":"))))
print(f"{len(runs)} runs, {len(timeline)} timeline entries -> {out}")
