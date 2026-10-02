"""Which code or data outside a set of units refers to the functions of those units: calls,
address loads and data words in the decomp's listings, the evidence that only certain entry
points lead into code the gap ledger explains.

    python tools/lockstep/callers.py C:/Projects/melee MetroTRK dolphin/mcc
"""
import os
import re
import sys

ROOT = os.path.join(sys.argv[1], "build", "GALE01", "asm")
prefixes = [p.replace("/", os.sep) for p in sys.argv[2:]]
defs = {}  # name -> unit
refs = []  # (from_unit, from_symbol, name)
FN = re.compile(r"^\.fn (\S+),")
OBJ = re.compile(r"^\.obj (\S+),")
for dirpath, _, files in os.walk(ROOT):
    for f in files:
        if not f.endswith(".s"):
            continue
        path = os.path.join(dirpath, f)
        unit = os.path.relpath(path, ROOT)[:-2]
        cur = None
        for line in open(path, encoding="utf-8", errors="replace"):
            m = FN.match(line) or OBJ.match(line)
            if m:
                cur = m.group(1).strip('"')
                if FN.match(line):
                    defs[cur] = unit
                continue
            if "*/\t" in line:
                ins = line.split("*/\t", 1)[1].split()
                if ins and ins[0] in ("bl", "b") and len(ins) > 1:
                    refs.append((unit, cur, ins[1]))
                for sym in re.findall(r"([A-Za-z_][\w@$.]*)@(?:ha|l|sda21)", line):
                    refs.append((unit, cur, sym))
            elif ".4byte" in line:
                for sym in re.findall(r"\.4byte ([A-Za-z_][\w@$.]*)", line):
                    refs.append((unit, cur, sym))
inside = {n for n, u in defs.items() if any(u.startswith(p) for p in prefixes)}
outside = {}
for unit, frm, name in refs:
    if name in inside and not any(unit.startswith(p) for p in prefixes):
        outside.setdefault(name, set()).add(f"{frm} ({unit})")
print(f"{len(inside)} functions in the units")
for name in sorted(outside):
    print(f"{name}: {', '.join(sorted(outside[name]))}")
