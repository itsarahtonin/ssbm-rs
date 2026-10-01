"""Functions whose generated port changed between two commits, for rechecking what lockstep
verified on the older one.

    python tools/lockstep/changed.py OLD [NEW] > changed.txt

Compares each registered port of crates/game/src/tu at the two commits (NEW defaults to the
working tree), with the inline copies it calls counted as part of it, and prints the address
and name of each that differs, in LOCKSTEP_DONE's format.
"""

import re
import subprocess
import sys

FN = re.compile(r"^(pub )?fn (\w+)<", re.M)
REGISTER = re.compile(r"register_port\(\s*(0x[0-9a-f]+),\s*(.*?)Returns::", re.S)


def files(rev):
    out = subprocess.run(["git", "ls-tree", "-r", "--name-only", rev, "crates/game/src/tu"],
                         capture_output=True, text=True, check=True).stdout.split()
    return [f for f in out if f.endswith(".rs") and not f.endswith("mod.rs")]


def read(rev, path):
    if rev is None:
        try:
            return open(path, encoding="utf-8").read()
        except FileNotFoundError:
            return ""
    return subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True,
                          encoding="utf-8").stdout


def bodies(text):
    """name -> body text of each function in a module."""
    marks = [(m.start(), m.group(2)) for m in FN.finditer(text)]
    out = {}
    for i, (at, name) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else len(text)
        out[name] = text[at:end]
    return out


def closure(name, fns, seen=None):
    """A function's body with those of the inline copies and machine code it calls."""
    seen = seen if seen is not None else set()
    if name in seen or name not in fns:
        return ""
    seen.add(name)
    body = fns[name]
    parts = [body]
    for callee in sorted(set(re.findall(r"\b((?:inl|asm)_\w+)\(", body))):
        parts.append(closure(callee, fns, seen))
    return "".join(parts)


def main():
    old = sys.argv[1]
    new = sys.argv[2] if len(sys.argv) > 2 else None
    paths = sorted(set(files(old)) | set(files(new or "HEAD")))
    n = 0
    for path in paths:
        a, b = read(old, path), read(new, path)
        if a == b:
            continue
        fa, fb = bodies(a), bodies(b)
        for m in REGISTER.finditer(b):
            # The adapter calls the port it registers.
            addr, adapter = m.group(1), m.group(2)
            name = next((c for c in re.findall(r"\b([A-Za-z_]\w*)\(ctx", adapter) if c in fb), None)
            if name is not None and closure(name, fa) != closure(name, fb):
                print(f"{addr} # {name}")
                n += 1
    print(f"{n} ports changed", file=sys.stderr)


if __name__ == "__main__":
    main()
