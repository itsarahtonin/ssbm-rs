"""Writes a focused job list for the functions the latest report leaves below the bar: for each
one some runs reached, one of the three jobs of waves 4 to 6 that called it most, a different one
each round, again with new seeds and more mutated checks per call. Jobs come from
jobs-wave*.txt; ledgers from cov-w4 on. BAR (default 0.9) is the share of its blocks, verified
or explained, a function needs.

    python tools/lockstep/campaign/mkjobs-focus.py NAME SEED MUTATE OUT

NAME prefixes the jobs and their card files (cards/NAME<kind>.raw, as wave6.sh names them).
"""
import collections
import csv
import glob
import os
import re
import sys

L = "local/lockstep"
name, seed, mutate, out_path = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]

bar = float(os.environ.get("BAR", "0.9"))
rows = list(csv.DictReader(open(f"{L}/report.csv", encoding="utf-8")))
below = {r["address"].lower() for r in rows
         if not (int(r["verified_blocks"] or 0) + int(r.get("explained") or 0)
                 >= bar * int(r["countable"] or 0)
                 and int(r["mismatches"] or 0) == 0 and int(r["mutated"] or 0) == 0)}

jobs = {}
for path in sorted(glob.glob(f"{L}/jobs-wave*.txt")):
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        if line and not line.startswith("#") and "|" in line:
            n, args = line.split("|", 1)
            jobs[n] = args

callers = collections.defaultdict(dict)
for d in ["cov-w4", "cov-w5", "cov-w6"] + sorted(os.path.basename(p) for p in glob.glob(f"{L}/cov-f*")):
    for ledger in glob.glob(f"{L}/{d}/*.csv"):
        job = os.path.basename(ledger)[:-4]
        if job not in jobs or "boss" in job:
            # Boss fighters in a debug VS match fail an assertion in the original game too.
            continue
        for r in csv.DictReader(open(ledger, encoding="utf-8")):
            a = r["address"].lower()
            if a in below:
                calls = int(r["calls"] or 0)
                if calls > callers[a].get(job, 0):
                    callers[a][job] = calls

# Each round takes the next of a function's three jobs that called it most, so rounds see it
# in other situations than the last one did.
turn = int(re.sub(r"\D", "", name) or 0)
best = {}
for a, jobs_of in callers.items():
    ranked = sorted(jobs_of.items(), key=lambda x: (-x[1], x[0]))[:3]
    job, calls = ranked[turn % len(ranked)]
    best[a] = (calls, job)

chosen = sorted({job for _, job in best.values()})
out = [f"# Focused round {name}: the jobs that called {len(best)} of the {len(below)} functions "
       f"below the bar most, with LOCKSTEP_SEED={seed}, run seeds +{seed} and LOCKSTEP_MUTATE={mutate}"]
for job in chosen:
    args = jobs[job]
    args = re.sub(r"(--monkey|--matches) (\d+)", lambda m: f"{m.group(1)} {int(m.group(2)) + seed}", args)
    args = re.sub(r"LOCKSTEP_MUTATE=\d+", f"LOCKSTEP_MUTATE={mutate}", args)
    args = re.sub(r"LOCKSTEP_SEED=\d+ ", "", args)
    # Waves 5 and 6 share job and card names but for their tag, which keeps them apart here.
    args = re.sub(rf"{L}/cards/(w\d|pm)([a-z]+)", rf"{L}/cards/{name}\1\2", args)
    short = job
    out.append(f"{name}{short}|LOCKSTEP_SEED={seed} {args}")
open(out_path, "w", encoding="utf-8", newline="\n").write("\n".join(out) + "\n")
print(f"{len(chosen)} jobs for {len(best)} of {len(below)} functions below the bar")
