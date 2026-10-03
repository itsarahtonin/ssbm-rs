"""Picks jobs to collect corpora from, for the needed functions no corpus has reached yet: the
functions a needed list names that no fuzz pass so far checked (the ledgers of the results
directories given), each with the jobs of any earlier round that called it, chosen greedily so
that few jobs reach them all. Writes the jobs' lines, as their rounds ran them, to OUT.

    python tools/lockstep/campaign/corpus-sources.py NEEDED OUT MAX FUZZED_DIR...

    e.g. corpus-sources.py local/lockstep/needed-f6x.txt local/lockstep/jobs-f6x-sources.txt \\
         30 cov-cz3 cov-cz7 cov-cz8
"""
import collections
import csv
import glob
import os
import sys

L = "local/lockstep"
needed_path, out_path, most, fuzzed = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4:]

needed = {line.split()[0].lower() for line in open(needed_path, encoding="utf-8") if line.strip()}
reached = set()
for d in fuzzed:
    for ledger in glob.glob(f"{L}/{d}/*.csv"):
        for r in csv.DictReader(open(ledger, encoding="utf-8")):
            if int(r["calls"] or 0) > 0:
                reached.add(r["address"].lower())
left = needed - reached

# Every round's job lines, by name; and per function left, the jobs that called it.
lines = {}
for path in sorted(glob.glob(f"{L}/jobs-*.txt")):
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        if line and not line.startswith("#") and "|" in line:
            lines.setdefault(line.split("|", 1)[0], line)
callers = collections.defaultdict(set)
for ledger in glob.glob(f"{L}/cov-*/*.csv"):
    job = os.path.basename(ledger)[:-4]
    # Boss fighters fail an assertion in the original game; card fault jobs need their card.
    if job not in lines or "boss" in job or "--card" in lines[job] and "cf" in job:
        continue
    for r in csv.DictReader(open(ledger, encoding="utf-8")):
        a = r["address"].lower()
        if a in left and int(r["calls"] or 0) > 0:
            callers[job].add(a)

chosen, covered = [], set()
while len(chosen) < most:
    job, gain = max(((j, fs - covered) for j, fs in callers.items() if j not in chosen),
                    key=lambda x: (len(x[1]), x[0]), default=(None, set()))
    if not gain:
        break
    chosen.append(job)
    covered |= gain
with open(out_path, "w", encoding="utf-8") as out:
    out.write(f"# {len(left)} needed functions no fuzz pass reached; these {len(chosen)} jobs "
              f"called {len(covered)} of them\n")
    for job in chosen:
        out.write(lines[job] + "\n")
print(f"{len(left)} needed functions unreached by fuzzing; {len(chosen)} jobs reach "
      f"{len(covered)} of them -> {out_path}")
