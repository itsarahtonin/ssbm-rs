"""Compares two runs' GX command streams, as GX_STREAM logs them: which frames differ, and with
their GX_DETAIL logs of one frame, which commands, grouped by register.

    python tools/gx/compare.py A.txt B.txt [A.txt.N B.txt.N]

Run the same replay twice, once with original code only and once with every port (`--port
all`): a frame whose commands differ points at a port, or at a read of stack the original never
wrote that lockstep sets aside, reaching the GPU.
"""
import collections
import difflib
import sys


def frames(path):
    out = []
    for line in open(path, encoding="utf-8"):
        w = line.split()
        out.append((int(w[1]), int(w[3]), int(w[5]), w[7]))
    return out


def main():
    a, b = frames(sys.argv[1]), frames(sys.argv[2])
    n = min(len(a), len(b))
    differ = [i for i in range(n) if a[i] != b[i]]
    shape = [i for i in differ if a[i][1:3] != b[i][1:3]]
    print(f"{n} frames compared ({len(a)} and {len(b)} logged): {len(differ)} differ, "
          f"{len(shape)} of them in their number of commands or draws")
    if differ:
        print(f"first differing frame: {differ[0]}")
    if shape:
        f = shape[0]
        print(f"first frame differing in shape: {f}: {a[f][1]} commands, {a[f][2]} draws "
              f"vs {b[f][1]}, {b[f][2]}")
    if len(sys.argv) < 5:
        return
    da = open(sys.argv[3], encoding="utf-8").read().splitlines()
    db = open(sys.argv[4], encoding="utf-8").read().splitlines()
    kinds = collections.Counter()
    first = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, da, db, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        for line in da[i1:i2]:
            kinds[" ".join(line.split()[:2])] += 1
        if len(first) < 5:
            first.append((i1, da[i1:i2][:3], db[j1:j2][:3]))
    print("commands that differ, by kind:")
    for kind, count in kinds.most_common():
        print(f"  {count:6} {kind}")
    for at, x, y in first:
        print(f"at command {at}: {x} vs {y}")


if __name__ == "__main__":
    main()
