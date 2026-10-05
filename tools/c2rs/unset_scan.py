"""Lists the functions whose C can return, or use, a value it never sets, as clang's analyses
find them: what the original then gets is whatever a register or the stack held.

    python tools/c2rs/unset_scan.py <decomp root> [unit ...]

Prints one line per finding: unit, function, clang's warning. The review of these readings is
part of the function-equivalence review.
"""

import json
import os
import sys
from concurrent.futures import ProcessPoolExecutor

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "typegen"))
import extract  # noqa: E402

import clang.cindex as ci  # noqa: E402

CK = ci.CursorKind
WARNINGS = ["-Wreturn-type", "-Wno-error=return-type", "-Wuninitialized", "-Wsometimes-uninitialized",
            "-Wconditional-uninitialized"]
OPTIONS = {"-Wreturn-type", "-Wuninitialized", "-Wsometimes-uninitialized", "-Wconditional-uninitialized"}


def scan(args):
    root, unit, source = args
    os.chdir(root)
    index = ci.Index.create()
    tu, _ = extract.parse(index, source, ["-DMWERKS_GEKKO"] + WARNINGS)
    if any(d.severity >= ci.Diagnostic.Error for d in tu.diagnostics):
        tu, _ = extract.parse(index, source, WARNINGS)
    src = os.path.normpath(source)
    functions = []
    for c in tu.cursor.get_children():
        if c.kind == CK.FUNCTION_DECL and c.is_definition() and c.location.file and \
                os.path.normpath(str(c.location.file)) == src:
            functions.append((c.extent.start.line, c.extent.end.line, c.spelling))
    out = []
    for d in tu.diagnostics:
        if d.severity != ci.Diagnostic.Warning or d.option not in OPTIONS or d.location.file is None:
            continue
        if os.path.normpath(str(d.location.file)) != src:
            continue
        line = d.location.line
        fn = next((name for a, b, name in functions if a <= line <= b), "?")
        out.append((unit, fn, line, d.option, d.spelling))
    return out


def main():
    root = os.path.abspath(sys.argv[1])
    only = set(sys.argv[2:])
    os.chdir(root)
    units = [(u["name"].removeprefix("main/"), u["metadata"]["source_path"])
             for u in json.load(open("objdiff.json"))["units"]
             if (u.get("metadata") or {}).get("source_path", "").endswith(".c")]
    units = [(root, u, s) for u, s in units if not only or u in only]
    with ProcessPoolExecutor() as pool:
        for found in pool.map(scan, units, chunksize=8):
            for unit, fn, line, option, text in found:
                print(f"{unit}\t{fn}\t{line}\t{option}\t{text}")


if __name__ == "__main__":
    main()
