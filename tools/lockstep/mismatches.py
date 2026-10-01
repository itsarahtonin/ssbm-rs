"""Groups the mismatches ssbm-run logs report by what differed, to triage them by cause.

    python tools/lockstep/mismatches.py <decomp root> LOG... [--mutated] [--top N]

Each mismatch a log details (`NAME call N:` or, for mutated checks and probes, `NAME call N
with its inputs changed:`) gets a signature: the function, and for each difference its kind
with where it is, memory by the symbol it falls in. One cause tends to give one signature, in
many calls of a function or in many functions: registers an assertion's handler saved, say,
all differ in the same thread context.
"""

import argparse
import bisect
import collections
import glob
import os
import re

HEAD = re.compile(r"^  (\S+) call (\d+)( with its inputs changed)?:$")


def symbols(root):
    rows = []
    path = os.path.join(root, "config", "GALE01", "symbols.txt")
    for line in open(path, encoding="utf-8"):
        m = re.match(r"(\S+) = \.\w+:0x([0-9A-F]+);.*size:0x([0-9A-F]+)", line)
        if m:
            rows.append((int(m.group(2), 16), int(m.group(3), 16), m.group(1)))
    rows.sort()
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("logs", nargs="+")
    ap.add_argument("--mutated", action="store_true", help="only mismatches with inputs changed")
    ap.add_argument("--top", type=int, default=60)
    args = ap.parse_args()
    rows = symbols(args.root)
    starts = [r[0] for r in rows]

    def where(addr):
        i = bisect.bisect_right(starts, addr) - 1
        if i >= 0 and addr < rows[i][0] + max(rows[i][1], 1):
            return rows[i][2]
        return "heap or stack" if 0x8000_0000 <= addr < 0x8180_0000 else f"{addr:#x}"

    counts = collections.Counter()
    for pattern in args.logs:
        for path in glob.glob(pattern):
            lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
            i = 0
            while i < len(lines):
                m = HEAD.match(lines[i])
                i += 1
                if not m or (args.mutated and not m.group(3)):
                    continue
                parts = set()
                while i < len(lines) and lines[i].startswith("    "):
                    d = lines[i].strip()
                    i += 1
                    if d.startswith("Mem"):
                        addr = int(re.search(r"addr: (\w+)", d).group(1), 16)
                        parts.add("memory in " + where(addr))
                    elif d.startswith("Reg"):
                        parts.add("register " + re.search(r'name: "([^"]+)"', d).group(1))
                    elif "the port made" in d:
                        text = d.split("port: Some(\"", 1)[-1]
                        parts.add(re.sub(r"0x[0-9A-F]{4,}", "X", text.split(" (in ")[0])[:90])
                    elif d.startswith("Panic"):
                        parts.add("panic")
                    elif d.startswith("Call"):
                        # The first of the function's own calls that differs, with
                        # LOCKSTEP_TRACE_CALLS=1.
                        names = dict(re.findall(r'(original|port): (?:Some\(\("([^"]+)"|None)', d))
                        parts.add(f"calls {names.get('original') or 'nothing'} where the port "
                                  f"calls {names.get('port') or 'nothing'}")
                    else:
                        parts.add(d.split(" ")[0])
                sig = f"{m.group(1)}: " + "; ".join(sorted(parts))
                counts[sig] += 1
    for sig, n in counts.most_common(args.top):
        print(f"{n:6}  {sig}")


if __name__ == "__main__":
    main()
