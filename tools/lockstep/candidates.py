"""Checks gap ledger candidates before they join gaps.txt: each line in its format with a known
KIND and evidence, naming a function the decomp has (as UNIT:NAME where other units share the
name), at the start of one of its blocks that the coverage given leaves unverified, and not a
block gaps.txt or an earlier line already explains. Prints the lines that fail, with why.

    python tools/lockstep/candidates.py <decomp root> CANDIDATES... --coverage FILE... [--gaps FILE]

--accepted OUT writes the lines that pass.
"""

import argparse
import collections
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))
import coverage  # noqa: E402

KINDS = {"dead", "dev", "fail", "hw", "oracle"}
LINE = re.compile(r"^(\S+)\+(0x[0-9A-Fa-f]+|\*)\s+(\S+)\s*#\s*(\S.*)$")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("candidates", nargs="+")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--gaps", default=os.path.join(os.path.dirname(__file__), "gaps.txt"))
    ap.add_argument("--accepted")
    args = ap.parse_args()

    by_name = collections.defaultdict(list)
    for unit, name, insns, labels in coverage.functions(args.root):
        by_name[name].append((unit, insns, labels))
    spans = {}
    for name, defs in by_name.items():
        for _, insns, _ in defs:
            spans[insns[0][0]] = (insns[0][0], insns[-1][0] + 4)
    bits = coverage.load_bits([p for g in args.coverage for p in glob.glob(g)], spans,
                              lambda path: set())
    explained = coverage.load_gaps(args.gaps)

    def resolve(name):
        """The function's (unit, insns, labels), or why the name doesn't pick one."""
        unit = None
        if ":" in name:
            unit, name = name.split(":", 1)
        defs = by_name.get(name, [])
        if unit is not None:
            defs = [d for d in defs if d[0] == unit]
        if not defs:
            return None, "no such function"
        if len(defs) > 1:
            return None, f"other units share the name: {', '.join(d[0] for d in defs)}"
        if unit is None and len(by_name[name]) > 1:
            return None, "needs its unit"
        return defs[0], None

    accepted, seen, bad = [], set(), 0
    for path in args.candidates:
        for n, raw in enumerate(open(path, encoding="utf-8"), 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            where = f"{os.path.basename(path)}:{n}"

            def fail(why):
                nonlocal bad
                bad += 1
                print(f"{where}: {why}: {line[:160]}")

            m = LINE.match(line)
            if not m:
                fail("not FUNCTION+0xOFF KIND # evidence")
                continue
            name, off, kind, _ = m.groups()
            if kind not in KINDS:
                fail(f"unknown kind {kind}")
                continue
            found, why = resolve(name)
            if why:
                fail(why)
                continue
            unit, insns, labels = found
            start = insns[0][0]
            key = (unit, name.split(":")[-1], off)
            if key in seen:
                fail("duplicate")
                continue
            seen.add(key)
            if "*" in explained.get(name, ()):
                fail("gaps.txt explains the whole function")
                continue
            if off == "*":
                hit = [b[0][0] for b in coverage.blocks(insns, labels)
                       if any(coverage.covered(bits, a) for a, _, _ in b)]
                if hit:
                    fail(f"{len(hit)} of its blocks are verified (first at {hit[0]:08X})")
                    continue
            else:
                o = int(off, 16)
                if o in explained.get(name, ()):
                    fail("gaps.txt already explains it")
                    continue
                block = next((b for b in coverage.blocks(insns, labels) if b[0][0] == start + o),
                             None)
                if block is None:
                    fail(f"{start + o:08X} starts no block of {name}")
                    continue
                if any(coverage.covered(bits, a) for a, _, _ in block):
                    fail(f"the block at {start + o:08X} is verified")
                    continue
            accepted.append(line)
    print(f"{len(accepted)} lines pass, {bad} fail")
    if args.accepted:
        with open(args.accepted, "w", encoding="utf-8", newline="\n") as out:
            out.writelines(line + "\n" for line in accepted)


if __name__ == "__main__":
    main()
