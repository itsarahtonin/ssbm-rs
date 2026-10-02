"""How much of each function lockstep has verified, block by block.

    python tools/lockstep/coverage.py <decomp root> --coverage FILE... [--ledger FILE...]
        [--bar 0.9] [--dead FILE] [--stand-ins FILE] [--stale CHANGED=FILES]... [--csv OUT]
        [--done OUT] [--reviewed FILE] [--gaps FILE] [--known OUT] [--needed OUT]

ssbm-run's LOCKSTEP_COVERAGE bitmaps mark each instruction of the original that ran within a
check of its own function whose sides agreed. This splits every function of the decomp's
listings into basic blocks and counts a block verified when any of its instructions is marked.
Blocks that call __assert, OSPanic or HSD_Panic, which never return and never run in a
correct game, are left out of the count, and so are those feasible.py finds no input can
reach, such as a clamp at 999999 of a halfword. A function is verified at or above the bar, as long
as no ledger records a mismatch for it; --done lists those, for LOCKSTEP_DONE. Functions
reach.py finds can never run (--dead), and those the SDK layer stands in for (--stand-ins, from
ssbm-run's STAND_INS), are counted apart: no run can check them. Results of a binary built
before a port last changed say nothing of it: each --stale gives a list of ports, as changed.py
writes it, that take nothing from the results its glob matches. Functions whose mismatches
with changed inputs were reviewed and found to come from the inputs, not the port (--reviewed,
one per line with the reason), are no longer to review. The gap ledger (--gaps, as gaps.txt
describes it) explains blocks no run can reach; a function is complete when its countable blocks
are all verified or explained. --known writes the merged bitmap, stale results left out, for
ssbm-run's LOCKSTEP_KNOWN, and --needed the blocks each incomplete function still needs, for
LOCKSTEP_NEEDED: `0xSTART lo-hi lo-hi ...`, a block per range of its instructions' addresses.
"""

import argparse
import collections
import csv
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))
import feasible  # noqa: E402

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


def load_bits(paths, spans, stale):
    """The union of the bitmaps, each without the instructions of the ports stale for it."""
    merged, masks = 0, {}
    for p in paths:
        data = int.from_bytes(open(p, "rb").read(), "little")
        starts = frozenset(stale(p))
        if starts:
            if starts not in masks:
                mask = 0
                for start in starts:
                    lo, hi = spans.get(start, (start, start))
                    if hi > lo:
                        mask |= ((1 << ((hi - lo) // 4)) - 1) << ((lo - LO) // 4)
                masks[starts] = mask
            data &= ~masks[starts]
        merged |= data
    return bytearray(merged.to_bytes((merged.bit_length() + 7) // 8, "little"))


def covered(bits, addr):
    i = (addr - LO) // 4
    return i // 8 < len(bits) and bits[i // 8] >> (i % 8) & 1


def load_gaps(path):
    """function -> offsets of the blocks the gap ledger explains, or "*" for all of them"""
    gaps = collections.defaultdict(set)
    for line in open(path, encoding="utf-8") if path else ():
        w = line.split("#")[0].split()
        if w:
            name, off = w[0].rsplit("+", 1)
            gaps[name].add(off if off == "*" else int(off, 16))
    return gaps


def load_ledgers(paths, stale):
    """address -> [calls, mismatches, uninitialized, mismatches with inputs no real call gave]"""
    rows = collections.defaultdict(lambda: [0, 0, 0, 0])
    for p in paths:
        skip = stale(p)
        for r in csv.DictReader(open(p, encoding="utf-8")):
            a = int(r["address"], 16)
            if a in skip:
                continue
            rows[a][0] += int(r["calls"])
            rows[a][1] += int(r["mismatches"])
            rows[a][2] += int(r["uninitialized"])
            rows[a][3] += int(r.get("mutated") or 0)
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--ledger", nargs="*", default=[])
    ap.add_argument("--bar", type=float, default=0.9)
    ap.add_argument("--dead")
    ap.add_argument("--stand-ins")
    ap.add_argument("--stale", action="append", default=[])
    ap.add_argument("--csv")
    ap.add_argument("--done")
    ap.add_argument("--reviewed")
    ap.add_argument("--gaps")
    ap.add_argument("--known")
    ap.add_argument("--needed")
    args = ap.parse_args()

    def names(path):
        if not path:
            return set()
        lines = (l.split("#")[0].split() for l in open(path, encoding="utf-8"))
        return {w[-1] if w[0].startswith("0x") else w[0] for w in lines if w}
    dead, stand_ins, reviewed = names(args.dead), names(args.stand_ins), names(args.reviewed)
    stale_lists = []
    for s in args.stale:
        changed, pattern = s.split("=", 1)
        starts = {int(l.split()[0], 16) for l in open(changed, encoding="utf-8")
                  if l.startswith("0x")}
        stale_lists.append((starts, {os.path.normcase(os.path.abspath(p))
                                     for p in glob.glob(pattern)}))

    def stale(path):
        path = os.path.normcase(os.path.abspath(path))
        return set().union(*(starts for starts, files in stale_lists if path in files))
    gaps = load_gaps(args.gaps)
    funcs = list(functions(args.root))
    spans = {insns[0][0]: (insns[0][0], insns[-1][0] + 4) for _, _, insns, _ in funcs}
    bits = load_bits([p for g in args.coverage for p in glob.glob(g)], spans, stale)
    if args.known:
        # The instructions still verified, for LOCKSTEP_KNOWN: stale results left out.
        open(args.known, "wb").write(bytes(bits) + bytes(1) * (-len(bits) % 8))
    ledger = load_ledgers([p for g in args.ledger for p in glob.glob(g)], stale)

    rows = []
    needed = {}
    unreachable, contradicted = 0, []
    for unit, name, insns, labels in funcs:
        bs = blocks(insns, labels)
        unreached = feasible.infeasible(insns, labels)
        for b in bs:
            if b[0][0] in unreached and any(covered(bits, a) for a, _, _ in b):
                contradicted.append(f"{name}+{b[0][0] - insns[0][0]:#x}")
        unreached -= {b[0][0] for b in bs if any(covered(bits, a) for a, _, _ in b)}
        unreachable += len(unreached)
        countable = [b for b in bs if b[0][0] not in unreached and
                     not any(is_call(m) and ops.split(",")[0].strip() in NORETURN
                             for _, m, ops in b)]
        done = sum(1 for b in countable if any(covered(bits, a) for a, _, _ in b))
        start = insns[0][0]
        gap = gaps.get(name, ())
        explained = sum(1 for b in countable if ("*" in gap or b[0][0] - start in gap)
                        and not any(covered(bits, a) for a, _, _ in b))
        calls, bad, uninit, mutated = ledger.get(start, (0, 0, 0, 0))
        missing = [(b[0][0], b[-1][0] + 4) for b in countable
                   if not any(covered(bits, a) for a, _, _ in b)
                   and not ("*" in gap or b[0][0] - start in gap)]
        if missing:
            needed[start] = missing
        share = done / len(countable) if countable else 1.0
        rows.append((start, name, unit, len(bs), len(countable), done, share, calls, bad, uninit,
                     mutated, explained))

    apart = [r for r in rows if r[1] in dead or r[1] in stand_ins]
    rows = [r for r in rows if r not in apart]
    verified = [r for r in rows if r[6] >= args.bar and r[8] == 0 and r[7] > 0]
    checked = [r for r in rows if r[7] > 0]
    mismatching = [r for r in rows if r[8] > 0]
    blocks_all = sum(r[4] for r in rows)
    blocks_done = sum(r[5] for r in rows)
    never = [r for r in apart if r[1] in dead]
    probed = sum(1 for r in never if r[6] >= args.bar and r[8] == 0 and r[7] > 0)
    print(f"{len(rows) + len(apart)} functions: {len(apart)} no run can reach "
          f"({len(never)} never called, {probed} of them verified by probes; "
          f"{sum(r[1] in stand_ins for r in apart)} that the SDK layer stands in for), "
          f"{len(rows)} to check")
    print(f"{blocks_all} blocks of these that can run "
          f"({sum(r[3] - r[4] for r in rows)} more only fail an assertion or no input reaches: "
          f"{unreachable} found unreachable in the machine code)")
    if contradicted:
        print(f"blocks found unreachable that runs verified, so counted: {len(contradicted)} "
              f"({', '.join(contradicted[:8])})")
    print(f"checked: {len(checked)} functions; blocks verified: {blocks_done} "
          f"({100 * blocks_done / max(blocks_all, 1):.1f}%)")
    print(f"verified at {args.bar:.0%} of their blocks, with no mismatch: {len(verified)}")
    complete = [r for r in rows if r[5] + r[11] >= r[4] and r[8] == 0 and (r[7] > 0 or r[5] == 0)]
    print(f"blocks explained by the gap ledger: {sum(r[11] for r in rows)}; complete, every block "
          f"verified or explained, with no mismatch: {len(complete)}")
    whole = [r for r in apart if r[5] + r[11] >= r[4] and r[8] == 0]
    print(f"of the functions no run can reach, every block verified by probes or explained: "
          f"{len(whole)} of {len(apart)}")
    print(f"with mismatches: {len(mismatching)}")
    print(f"with mismatches only from changed inputs (mutations, probes), to review: "
          f"{sum(1 for r in rows if r[10] > 0 and r[8] == 0 and r[1] not in reviewed)}"
          f" (and {sum(1 for r in rows if r[10] > 0 and r[8] == 0 and r[1] in reviewed)} "
          f"reviewed as coming from the inputs)")
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
                        "share", "calls", "mismatches", "uninitialized", "mutated",
                        "explained"])
            for r in rows:
                w.writerow([f"{r[0]:#010x}", r[1], r[2], r[3], r[4], r[5], f"{r[6]:.3f}",
                            r[7], r[8], r[9], r[10], r[11]])
    if args.needed:
        with open(args.needed, "w", encoding="utf-8") as f:
            for start, missing in sorted(needed.items()):
                f.write(f"{start:#010x} " + " ".join(f"{lo:#x}-{hi:#x}" for lo, hi in missing) + "\n")
    if args.done:
        with open(args.done, "w", encoding="utf-8") as f:
            f.write(f"# Functions verified at {args.bar:.0%} of their blocks with no mismatch\n")
            for r in verified:
                f.write(f"{r[0]:#010x} # {r[1]}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
