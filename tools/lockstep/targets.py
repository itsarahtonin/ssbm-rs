"""Lists, for ssbm-run's LOCKSTEP_TARGETS, what decides the branches to the code lockstep has
not reached in the functions below the bar.

    python tools/lockstep/targets.py <decomp root> REPORT_CSV OUT --coverage FILE... [--bar 0.9]

For each block not yet verified that a verified block branches to, the compare deciding that
branch is traced back to what set its register, within the block and its lone predecessors:

    call   a callee's result: `F call G`
    reg    an argument the function was called with: `F reg N = K` or `F reg N & MASK`
    load   a value the function loads: `F load PC SIZE = K` or `F load PC SIZE & MASK`

where K is the constant the compare tests against and MASK the bits a test such as `rlwinm.`
looks at. A listed function's mutated checks take one of its targets half the time: a callee
then returns a small number instead of running, on both sides alike; an argument becomes K or
a neighbor of it, or has the bits toggled; a word the original loaded at PC in the check
being mutated does too.
"""

import argparse
import csv
import glob
import os
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(__file__))
import coverage  # noqa: E402
import feasible  # noqa: E402

# Instructions that pass a value on with any small value or bit it holds intact.
PASSING = ("mr", "extsb", "extsh", "clrlwi")
LOAD_SIZES = {"lbz": 1, "lbzx": 1, "lbzu": 1, "lbzux": 1, "lhz": 2, "lhzx": 2, "lhzu": 2,
              "lhzux": 2, "lha": 2, "lhax": 2, "lhau": 2, "lhaux": 2, "lwz": 4, "lwzx": 4,
              "lwzu": 4, "lwzux": 4}
ARGS = {f"r{i}" for i in range(3, 11)}
CTR = (None, "dnz", "dz", "dnzt", "dnzf", "dzt", "dzf", "ns", "so", "un", "nu")


def mask(mb, me):
    """rlwinm's mask from bit mb to bit me, bit 0 the most significant."""
    m = 0
    for b in range(32):
        inside = mb <= b <= me if mb <= me else (b >= mb or b <= me)
        if inside:
            m |= 1 << (31 - b)
    return m


def source(cfg, b, upto, reg, depth=0):
    """What `reg` holds before instruction `upto` of block `b`: ("call", callee), ("reg", reg)
    for an argument, ("load", pc, size), or None. Follows copies and lone predecessors."""
    blocks, preds = cfg
    block = blocks[b]
    if depth > 8:
        return None
    for j in range(upto - 1, -1, -1):
        pc, m, ops = block[j]
        r = feasible.regs(ops)
        if coverage.is_call(m):
            if reg == "r3" and m == "bl":
                return ("call", r[0])
            if reg in feasible.VOLATILE:
                return None
            continue
        if not r or r[0] != reg or m.startswith(feasible.NOT_WRITING):
            continue
        if m.rstrip(".") in PASSING and len(r) >= 2:
            return source(cfg, b, j, r[1], depth + 1)
        if m in LOAD_SIZES:
            return ("load", pc, LOAD_SIZES[m])
        return None
    if b == 0:
        return ("reg", reg) if reg in ARGS else None
    if len(preds[b]) == 1:
        p = preds[b][0]
        return source(cfg, p, len(blocks[p]), reg, depth + 1)
    return None


def decider(cfg, b):
    """What decides the conditional branch ending block `b`: (source, "=", K) for a compare
    with a constant, (source, "&", MASK) for a bit test, or None."""
    block = cfg[0][b]
    bm = feasible.BRANCH.match(block[-1][1])
    if not bm or bm.group(1) in CTR:
        return None
    for j in range(len(block) - 2, -1, -1):
        _, m, ops = block[j]
        r = feasible.regs(ops)
        if m in ("cmpwi", "cmplwi"):
            x, k = (r[1], r[2]) if len(r) == 3 else (r[0], r[1])
            k = feasible.imm(k)
            return (source(cfg, b, j, x), "=", k & 0xFFFFFFFF) if k is not None else None
        if m.startswith(("cmp", "fcmp")) or coverage.is_call(m):
            return None
        if m.endswith("."):
            base = m[:-1]
            if base in ("mr", "or") and len(r) >= 2:
                return (source(cfg, b, j, r[1]), "=", 0)
            if base == "extsb" or base == "extsh":
                return (source(cfg, b, j, r[1]), "=", 0)
            if base == "andi" and len(r) == 3 and feasible.imm(r[2]) is not None:
                return (source(cfg, b, j, r[1]), "&", feasible.imm(r[2]))
            if base == "clrlwi" and len(r) == 3 and feasible.imm(r[2]) is not None:
                return (source(cfg, b, j, r[1]), "&", (1 << (32 - feasible.imm(r[2]))) - 1)
            if base == "extrwi" and len(r) == 4:
                n, at = feasible.imm(r[2]), feasible.imm(r[3])
                if n is not None and at is not None:
                    return (source(cfg, b, j, r[1]), "&", ((1 << n) - 1) << (32 - at - n))
            if base == "rlwinm" and len(r) == 5 and feasible.imm(r[2]) == 0:
                mb, me = feasible.imm(r[3]), feasible.imm(r[4])
                if mb is not None and me is not None:
                    return (source(cfg, b, j, r[1]), "&", mask(mb, me))
            return None
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("report")
    ap.add_argument("out")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--bar", type=float, default=0.9)
    args = ap.parse_args()
    under = {r["name"] for r in csv.DictReader(open(args.report, encoding="utf-8"))
             if int(r["countable"]) > 0 and float(r["share"]) < args.bar}
    funcs = list(coverage.functions(args.root))
    # Each name's functions, by unit: a static callee is the caller's unit's own.
    addrs = defaultdict(dict)
    for unit, name, insns, _ in funcs:
        addrs[name][unit] = insns[0][0]
    todo = [f for f in funcs if f[1] in under]
    spans = {i[0][0]: (i[0][0], i[-1][0] + 4) for _, _, i, _ in todo}
    bits = coverage.load_bits([p for g in args.coverage for p in glob.glob(g)], spans,
                              lambda path: set())
    lines = []
    kinds = defaultdict(int)
    for unit, name, insns, labels in todo:
        blocks = coverage.blocks(insns, labels)
        index = {blk[0][0]: i for i, blk in enumerate(blocks)}
        preds = [[] for _ in blocks]
        for i, blk in enumerate(blocks):
            _, m, ops = blk[-1]
            t = feasible.target(ops)
            if t is not None and t in index and not coverage.is_call(m):
                preds[index[t]].append(i)
            if m not in ("b", "blr", "bctr", "rfi") and i + 1 < len(blocks):
                preds[i + 1].append(i)
        cfg = (blocks, preds)
        unreached = feasible.infeasible(insns, labels)
        hit = [any(coverage.covered(bits, a) for a, _, _ in blk) for blk in blocks]
        seen = set()
        for i, blk in enumerate(blocks):
            if not hit[i]:
                continue
            t = feasible.target(blk[-1][2])
            ways = [index.get(t)] if t is not None else []
            ways.append(i + 1 if i + 1 < len(blocks) else None)
            if all(w is None or hit[w] or blocks[w][0][0] in unreached for w in ways):
                continue
            d = decider(cfg, i)
            if d is None or d[0] is None:
                continue
            src, op, k = d
            if src[0] == "call":
                where = addrs.get(src[1], {})
                at = where.get(unit) or (next(iter(where.values())) if len(where) == 1 else None)
                if at is None or src[1] == name:
                    continue
                line = f"call {at:#010x}"
                note = f"{name} {src[1]}"
            elif src[0] == "reg":
                line = f"reg {src[1][1:]} {op} {k:#x}"
                note = name
            else:
                size = src[2]
                if op == "&":
                    k &= (1 << (8 * size)) - 1
                    if not k:
                        continue
                line = f"load {src[1]:#010x} {size} {op} {k:#x}"
                note = name
            if line not in seen:
                seen.add(line)
                kinds[src[0]] += 1
                lines.append(f"{insns[0][0]:#010x} {line} # {note}\n")
    with open(args.out, "w") as f:
        f.writelines(lines)
    print(f"{len(lines)} targets in {len({ln.split()[0] for ln in lines})} functions: "
          + ", ".join(f"{n} {k}" for k, n in sorted(kinds.items())), file=sys.stderr)


if __name__ == "__main__":
    main()
