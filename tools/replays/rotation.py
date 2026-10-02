"""Runs the next slice of the replay corpus and records the results in a ledger.

The slice is the replays the ledger has never seen, then those checked longest ago, in index
order. Each result goes into ledger.csv as: replay, commit, time, PASS or FAIL, summary.

    python tools/replays/rotation.py [--n 40] [--bin PATH] [--jobs 8] [-- extra ssbm-run args]
"""

import argparse
import csv
import datetime
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
# The machine's own paths, as tools/lockstep/campaign/env.sh describes them.
DISC = os.environ["SSBM_DISC"]
DATA = os.path.join(ROOT, "local", "replays")
LEDGER = os.path.join(DATA, "ledger.csv")


def local_path(path):
    """`path` from the index, under REPLAYS if it names the replays under REPLAYS_FROM."""
    path = path.replace("\\", "/")
    src, dst = os.environ.get("REPLAYS_FROM"), os.environ.get("REPLAYS")
    return path.replace(src, dst) if src and dst else path


def commit():
    head = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout.strip()
    dirty = subprocess.run(["git", "diff", "--quiet", "HEAD"], cwd=ROOT).returncode != 0
    return head + ("-dirty" if dirty else "")


def ledger():
    """replay name -> time of its latest entry."""
    seen = {}
    if os.path.exists(LEDGER):
        for row in csv.reader(open(LEDGER, newline="", encoding="utf-8")):
            if row and row[0] != "replay":
                seen[row[0]] = max(seen.get(row[0], ""), row[2])
    return seen


def run(bin_path, extra, path):
    name = os.path.splitext(os.path.basename(path.replace("\\", "/")))[0]
    out = os.path.join(DATA, "results-rotation", name + ".txt")
    known = os.path.join(DATA, "known", name + ".txt")
    cmd = [bin_path, DISC, "--replay", local_path(path), "--fields", "100000", "--known", known,
           "--write-known", os.path.join(DATA, "known-new", name + ".txt")] + extra
    with open(out, "w", encoding="utf-8") as f:
        code = subprocess.run(cmd, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT).returncode
    summary = ""
    for line in open(out, encoding="utf-8", errors="replace"):
        if "replay check" in line:
            summary = line.strip()
    return name, "PASS" if code == 0 else "FAIL", summary


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=40)
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--bin", default=os.path.join(ROOT, "target", "release", "ssbm-run"))
    ap.add_argument("extra", nargs="*")
    args = ap.parse_args()
    extra = args.extra or ["--port", "all"]
    for d in ("results-rotation", "known", "known-new"):
        os.makedirs(os.path.join(DATA, d), exist_ok=True)
    index = [row["path"] for row in csv.DictReader(open(os.path.join(DATA, "index.csv"), encoding="utf-8"))]
    seen = ledger()

    def key(path):
        name = os.path.splitext(os.path.basename(path.replace("\\", "/")))[0]
        return (name in seen, seen.get(name, ""))
    batch = sorted(index, key=key)[:args.n]
    rev = commit()
    new_file = not os.path.exists(LEDGER)
    fails = 0
    with ThreadPoolExecutor(args.jobs) as pool, open(LEDGER, "a", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        if new_file:
            w.writerow(["replay", "commit", "time", "result", "summary"])
        for name, result, summary in pool.map(lambda p: run(args.bin, extra, p), batch):
            now = datetime.datetime.now().isoformat(timespec="seconds")
            w.writerow([name, rev, now, result, summary])
            f.flush()
            fails += result != "PASS"
            print(f"{result} {name}: {summary}", flush=True)
    print(f"{len(batch) - fails} of {len(batch)} passed at {rev}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
