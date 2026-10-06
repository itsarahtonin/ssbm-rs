"""Sorts the open blocks (countable, unverified and not explained by the gap ledger) by what keeps
runs from them, and sums them per unit, to choose what to aim at next:

  directed   next to a verified block, behind a branch a mutation target names (load, cmp, call,
             reg, same): directed mutated checks (LOCKSTEP_DIRECTED) can set what decides it
  untraced   next to a verified block, behind a compare whose value targets.py can't trace
  switch     a switch case, or the block after a call, next to verified code
  deep       behind other unverified blocks only: reached once one before it is
  uncalled   in a function no run has called

    python tools/lockstep/triage.py C:/Projects/melee local/lockstep/report.csv \\
        --coverage local/lockstep/known.bin --gaps tools/lockstep/gaps.txt [--units N]

Its total matches the report's open blocks (countable less verified and explained), as
coverage.py counts them; frontier.py breaks the frontier down further by target kind.
"""
import argparse
import collections
import csv
import glob
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
import coverage  # noqa: E402
import feasible  # noqa: E402
import targets  # noqa: E402

KINDS = ("directed", "untraced", "switch", "deep", "uncalled")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("report")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--gaps")
    ap.add_argument("--units", type=int, default=30)
    args = ap.parse_args()
    rows = list(csv.DictReader(open(args.report, encoding="utf-8")))
    info = {r["name"]: r for r in rows}
    want = {r["name"] for r in rows
            if int(r["countable"]) - int(r["verified_blocks"]) - int(r.get("explained") or 0) > 0}
    gaps = coverage.load_gaps(args.gaps) if args.gaps else {}
    funcs = [f for f in coverage.functions(args.root) if f[1] in want]
    spans = {i[0][0]: (i[0][0], i[-1][0] + 4) for _, _, i, _ in funcs}
    bits = coverage.load_bits([p for g in args.coverage for p in glob.glob(g)], spans,
                              lambda path: set())
    total = collections.Counter()
    per_unit = collections.defaultdict(collections.Counter)
    per_fn = collections.Counter()
    unit_consts = {}
    for unit, name, insns, labels in funcs:
        start = insns[0][0]
        gap = set(gaps.get(name, ())) | gaps.get(f"{unit}:{name}", set())
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
        if unit not in unit_consts:
            unit_consts[unit] = targets.constants(args.root, unit)
        unreached = feasible.infeasible(insns, labels)
        hit = [any(coverage.covered(bits, a) for a, _, _ in blk) for blk in blocks]
        called = int(info[name]["calls"]) > 0
        for i, blk in enumerate(blocks):
            if hit[i] or blk[0][0] in unreached or targets.fails(blk):
                continue
            if "*" in gap or blk[0][0] - start in gap:
                continue
            near = [p for p in preds[i] if hit[p]]
            if not called:
                kind = "uncalled"
            elif not near and preds[i]:
                kind = "deep"
            elif not preds[i] or coverage.is_call(blocks[near[0]][-1][1]):
                kind = "switch"
            else:
                d = targets.decider(cfg, near[0], unit_consts[unit])
                kind = "untraced" if d is None or d[0] is None else "directed"
            total[kind] += 1
            per_unit[unit][kind] += 1
            per_fn[name] += 1
    print(f"open blocks: {sum(total.values())}")
    for kind in KINDS:
        print(f"  {kind:9} {total[kind]:5}")
    few = [n for n in per_fn.values() if n <= 2]
    print(f"functions with open blocks: {len(per_fn)}, {len(few)} of them with one or two ({sum(few)} blocks)")
    print(f"\nunits by open blocks ({' / '.join(KINDS)}):")
    for unit, c in sorted(per_unit.items(), key=lambda kv: -sum(kv[1].values()))[:args.units]:
        print(f"  {unit:44} {sum(c.values()):5}   " + " ".join(f"{c[k]:4}" for k in KINDS))


if __name__ == "__main__":
    main()
