"""Which functions the game can reach at all, from the decomp's listings.

    python tools/lockstep/reach.py <decomp root> [--dead OUT]

A function is reachable from `__start`, from anything a data table points to (callbacks,
dispatch tables), and from what reachable code calls, branches to or takes the address of.
The rest can never run, whatever the input, so coverage counts only the reachable ones.
--dead lists the others, with the unit each is in.
"""

import argparse
import collections
import glob
import os
import re
import sys

INSN = re.compile(r"^/\* ([0-9A-F]{8}) [0-9A-F]{8}  (?:[0-9A-F]{2} ){4}\*/\t(\S+)\s*(.*)$")
SYMBOL = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)(?:@(?:ha|h|l|sda21))?\b")


def listings(root):
    """(unit, function or None for data, list of referenced names) for each item."""
    base = os.path.join(root, "build", "GALE01", "asm")
    for path in sorted(glob.glob(os.path.join(base, "**", "*.s"), recursive=True)):
        unit = os.path.relpath(path, base)[:-2].replace(os.sep, "/")
        fn, refs = None, []
        for line in open(path, encoding="utf-8", errors="replace"):
            if line.startswith(".fn "):
                fn, refs = line[4:].split(",")[0].strip(), []
                continue
            if line.startswith(".endfn"):
                yield unit, fn, refs
                fn = None
                continue
            if fn is not None:
                m = INSN.match(line.rstrip("\n"))
                if m:
                    refs.extend(n for n in SYMBOL.findall(m.group(3)) if not re.fullmatch(r"[rf]\d+|lr|ctr|0x\w+", n))
            elif line.lstrip().startswith((".4byte", ".rel")):
                operands = line.split(None, 1)[1] if len(line.split(None, 1)) > 1 else ""
                # A jump table's entries point into the function the table belongs to.
                yield unit, None, SYMBOL.findall(operands.split(",")[0] if ".rel" in line else operands)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("--dead")
    args = ap.parse_args()
    calls, unit_of, from_data = {}, {}, set()
    for unit, fn, refs in listings(args.root):
        if fn is None:
            from_data.update(refs)
        else:
            calls[fn] = refs
            unit_of[fn] = unit
    roots = {"__start"} | (from_data & calls.keys())
    live, todo = set(), list(roots)
    while todo:
        f = todo.pop()
        if f in live or f not in calls:
            continue
        live.add(f)
        todo.extend(r for r in calls[f] if r in calls and r not in live)
    dead = sorted(set(calls) - live)
    by_area = collections.Counter("/".join(unit_of[f].split("/")[:2]) for f in dead)
    print(f"{len(calls)} functions: {len(live)} reachable, {len(dead)} never")
    for area, n in by_area.most_common(25):
        print(f"  {area:28} {n}")
    if args.dead:
        with open(args.dead, "w", encoding="utf-8") as f:
            f.write("# Functions nothing reachable calls, branches to or points to\n")
            for name in dead:
                f.write(f"{name} {unit_of[name]}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
