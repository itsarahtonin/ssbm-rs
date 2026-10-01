"""Shows a function's basic blocks with whether lockstep verified each, to see what its checks
never reached.

    python tools/lockstep/gaps.py <decomp root> NAME... --coverage FILE...

Each block prints with its instructions, marked `+` where any of them is verified and `-`
where none is; `!` marks the blocks coverage.py leaves out, which only fail an assertion.
"""

import argparse
import glob
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
import coverage  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("names", nargs="+")
    ap.add_argument("--coverage", nargs="+", default=[])
    args = ap.parse_args()
    want = set(args.names)
    funcs = [f for f in coverage.functions(args.root) if f[1] in want]
    spans = {insns[0][0]: (insns[0][0], insns[-1][0] + 4) for _, _, insns, _ in funcs}
    bits = coverage.load_bits([p for g in args.coverage for p in glob.glob(g)], spans,
                              lambda path: set())
    for unit, name, insns, labels in funcs:
        blocks = coverage.blocks(insns, labels)
        print(f"{name} ({unit}): {len(blocks)} blocks")
        for b in blocks:
            assertion = any(coverage.is_call(m) and ops.split(",")[0].strip() in coverage.NORETURN
                            for _, m, ops in b)
            hit = any(coverage.covered(bits, a) for a, _, _ in b)
            mark = "!" if assertion else "+" if hit else "-"
            for i, (a, m, ops) in enumerate(b):
                print(f"  {mark if i == 0 else ' '} {a:08X}  {m:8} {ops}")
        print()


if __name__ == "__main__":
    main()
