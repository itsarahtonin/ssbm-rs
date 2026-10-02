"""Sorts the frontier, the unreached blocks next to verified ones in the functions runs call, by
what keeps mutated checks from them: a target that names what decides the branch (load, call,
reg, same, switch), a compare whose value targets.py cannot trace, or no static way in at all.

    python tools/lockstep/frontier.py C:/Projects/melee local/lockstep/report.csv \\
        --coverage local/lockstep/known.bin

Blocks that only fail an assertion, or that no input reaches, are left out, as coverage.py
leaves them out of the count.
"""
import argparse
import csv
import glob
from collections import Counter, defaultdict

import coverage
import feasible
import targets


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("report")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--examples", type=int, default=4)
    args = ap.parse_args()
    rows = list(csv.DictReader(open(args.report, encoding="utf-8")))
    want = {r["name"] for r in rows if int(r["calls"]) > 0
            and int(r["countable"]) - int(r["verified_blocks"]) - int(r.get("explained") or 0) > 0}
    funcs = [f for f in coverage.functions(args.root) if f[1] in want]
    spans = {i[0][0]: (i[0][0], i[-1][0] + 4) for _, _, i, _ in funcs}
    bits = coverage.load_bits([p for g in args.coverage for p in glob.glob(g)], spans,
                              lambda path: set())
    why = Counter()
    examples = defaultdict(list)
    unit_consts = {}
    for unit, name, insns, labels in funcs:
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
        for i, blk in enumerate(blocks):
            if hit[i] or blk[0][0] in unreached or targets.fails(blk):
                continue
            near = [p for p in preds[i] if hit[p]]
            if preds[i] and not near:
                continue
            if not preds[i]:
                kind = "switch case" if any(
                    b[-1][1] == "bctr" and hit[j] for j, b in enumerate(blocks)) else "no way in"
            elif coverage.is_call(blocks[near[0]][-1][1]):
                kind = "after a call"
            else:
                d = targets.decider(cfg, near[0], unit_consts[unit])
                if d is None:
                    kind = "untraced compare"
                elif d[0] is None:
                    kind = "untraced value"
                else:
                    kind = f"target {d[0][0]}"
            why[kind] += 1
            examples[kind].append(name)
    for kind, n in why.most_common():
        names = list(dict.fromkeys(examples[kind]))[:args.examples]
        print(f"{n:6} {kind:18} e.g. {', '.join(names)}")


if __name__ == "__main__":
    main()
