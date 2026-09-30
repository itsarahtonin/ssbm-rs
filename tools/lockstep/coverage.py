"""How much of each function lockstep has verified, block by block.

    python tools/lockstep/coverage.py <decomp root> --coverage FILE... [--ledger FILE...]
        [--bar 0.9] [--csv OUT] [--done OUT]

ssbm-run's LOCKSTEP_COVERAGE bitmaps mark each instruction of the original that ran within a
check of its own function whose sides agreed. This splits every function of the decomp's
listings into basic blocks and counts a block verified when any of its instructions is marked.
Blocks that call __assert, OSPanic or HSD_Panic, which never return and never run in a
correct game, are left out of the count. A function is verified at or above the bar, as long
as no ledger records a mismatch for it; --done lists those, for LOCKSTEP_DONE.
"""

import argparse
import collections
import csv
import glob
import os
import re
import sys

LO = 0x8000_0000
NORETURN = {"__assert", "OSPanic", "HSD_Panic"}
INSN = re.compile(r"^/\* ([0-9A-F]{8}) [0-9A-F]{8}  (?:[0-9A-F]{2} ){4}\*/\t(\S+)\s*(.*)$")


def is_call(m):
    return m in ("bl", "bla", "bctrl", "blrl", "bcctrl", "bclrl")


def ends_block(m):
    m = m.rstrip("+-")
    return (m.startswith("b") and not is_call(m)) or m in ("rfi", "sc") or m.startswith("tw")


def functions(root):
    """Yields (unit, name, [(address, mnemonic, operands)], {label addresses})."""
    base = os.path.join(root, "build", "GALE01", "asm")
    for path in sorted(glob.glob(os.path.join(base, "**", "*.s"), recursive=True)):
        unit = os.path.relpath(path, base)[:-2].replace(os.sep, "/")
        name, insns, labels = None, [], set()
        for line in open(path, encoding="utf-8", errors="replace"):
            if line.startswith(".fn "):
                name, insns, labels = line[4:].split(",")[0].strip(), [], set()
            elif line.startswith(".endfn"):
                if name and insns:
                    yield unit, name, insns, labels
                name = None
            elif name:
                if line.startswith(".L_"):
                    labels.add(int(line[3:11], 16))
                else:
                    m = INSN.match(line.rstrip("\n"))
                    if m:
                        insns.append((int(m.group(1), 16), m.group(2), m.group(3)))


def blocks(insns, labels):
    """The function's basic blocks, as lists of its instructions."""
    out, cur = [], []
    for i, (addr, m, ops) in enumerate(insns):
        if cur and addr in labels:
            out.append(cur)
            cur = []
        cur.append((addr, m, ops))
        if ends_block(m):
            out.append(cur)
            cur = []
    if cur:
        out.append(cur)
    return out


def load_bits(paths):
    bits = bytearray()
    for p in paths:
        data = open(p, "rb").read()
        if len(data) > len(bits):
            bits.extend(b"\0" * (len(data) - len(bits)))
        for i, b in enumerate(data):
            bits[i] |= b
    return bits


def covered(bits, addr):
    i = (addr - LO) // 4
    return i // 8 < len(bits) and bits[i // 8] >> (i % 8) & 1


def load_ledgers(paths):
    rows = collections.defaultdict(lambda: [0, 0, 0])
    for p in paths:
        for r in csv.DictReader(open(p, encoding="utf-8")):
            a = int(r["address"], 16)
            rows[a][0] += int(r["calls"])
            rows[a][1] += int(r["mismatches"])
            rows[a][2] += int(r["uninitialized"])
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--ledger", nargs="*", default=[])
    ap.add_argument("--bar", type=float, default=0.9)
    ap.add_argument("--csv")
    ap.add_argument("--done")
    args = ap.parse_args()
    bits = load_bits([p for g in args.coverage for p in glob.glob(g)])
    ledger = load_ledgers([p for g in args.ledger for p in glob.glob(g)])

    rows = []
    for unit, name, insns, labels in functions(args.root):
        bs = blocks(insns, labels)
        countable = [b for b in bs if not any(is_call(m) and ops.split(",")[0].strip() in NORETURN
                                              for _, m, ops in b)]
        done = sum(1 for b in countable if any(covered(bits, a) for a, _, _ in b))
        start = insns[0][0]
        calls, bad, uninit = ledger.get(start, (0, 0, 0))
        share = done / len(countable) if countable else 1.0
        rows.append((start, name, unit, len(bs), len(countable), done, share, calls, bad, uninit))

    verified = [r for r in rows if r[6] >= args.bar and r[8] == 0 and r[7] > 0]
    checked = [r for r in rows if r[7] > 0]
    mismatching = [r for r in rows if r[8] > 0]
    blocks_all = sum(r[4] for r in rows)
    blocks_done = sum(r[5] for r in rows)
    print(f"{len(rows)} functions, {blocks_all} blocks that can run "
          f"({sum(r[3] - r[4] for r in rows)} more only fail an assertion)")
    print(f"checked: {len(checked)} functions; blocks verified: {blocks_done} "
          f"({100 * blocks_done / max(blocks_all, 1):.1f}%)")
    print(f"verified at {args.bar:.0%} of their blocks, with no mismatch: {len(verified)}")
    print(f"with mismatches: {len(mismatching)}")
    by_area = collections.defaultdict(lambda: [0, 0])
    for r in rows:
        area = "/".join(r[2].split("/")[:2])
        by_area[area][0] += 1
        by_area[area][1] += r in verified
    print("verified by area:")
    for area, (n, v) in sorted(by_area.items(), key=lambda x: -(x[1][0] - x[1][1])):
        print(f"  {area:28} {v:6} of {n:6}")

    if args.csv:
        with open(args.csv, "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(["address", "name", "unit", "blocks", "countable", "verified_blocks",
                        "share", "calls", "mismatches", "uninitialized"])
            for r in rows:
                w.writerow([f"{r[0]:#010x}", r[1], r[2], r[3], r[4], r[5], f"{r[6]:.3f}",
                            r[7], r[8], r[9]])
    if args.done:
        with open(args.done, "w", encoding="utf-8") as f:
            f.write(f"# Functions verified at {args.bar:.0%} of their blocks with no mismatch\n")
            for r in verified:
                f.write(f"{r[0]:#010x} # {r[1]}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
