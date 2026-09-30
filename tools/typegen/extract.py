#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Extracts types, globals and function prototypes from the Melee decomp with libclang.

Usage: extract.py <decomp root> <out.json>

Layouts come from clang with the decomp's lint flags (32-bit big-endian PowerPC, layout asserts
as static asserts), plus -fsigned-char to match MWCC. Symbols come from the decomp's config.
"""

import bisect
import json
import os
import re
import sys
from concurrent.futures import ProcessPoolExecutor

import clang.cindex as ci

TK = ci.TypeKind
CK = ci.CursorKind

FLAGS = [
    "-xc", "-std=c99", "-nostdinc", "-fno-builtin", "--target=ppc32-none-eabi", "-DLINT",
    "-fno-short-enums", "-fsigned-char", "-DVERSION_GALE01", "-DBUILD_VERSION=0",
    "-Isrc", "-isystemsrc/MSL", "-isystemlibs/dolphin/include", "-isystemlibs/dolphin/src",
    "-isystembuild/GALE01/include", "-isystemlibs/dolphin/src/dolphin", "-fdeclspec",
]

SIGNED = {TK.SCHAR, TK.CHAR_S, TK.SHORT, TK.INT, TK.LONG, TK.LONGLONG, TK.WCHAR}
UNSIGNED = {TK.UCHAR, TK.CHAR_U, TK.USHORT, TK.UINT, TK.ULONG, TK.ULONGLONG, TK.BOOL, TK.CHAR16, TK.CHAR32}
FLOATS = {TK.FLOAT, TK.DOUBLE, TK.LONGDOUBLE}


def load_symbols(path):
    syms = []
    pat = re.compile(r"^(\S+) = (\.?\w+):0x([0-9A-Fa-f]+); // type:(\w+)(?: size:0x([0-9A-Fa-f]+))?.*?scope:(\w+)")
    for line in open(path):
        m = pat.match(line)
        if m:
            syms.append({"name": m.group(1), "section": m.group(2), "addr": int(m.group(3), 16),
                         "type": m.group(4), "size": int(m.group(5) or "0", 16), "scope": m.group(6)})
    return syms


def load_splits(path):
    ranges, cur = [], None
    for line in open(path):
        m = re.match(r"^(\S.*\.(?:c|cpp|s)):\s*$", line)
        if m:
            cur = m.group(1)
            continue
        m = re.match(r"\s+(\.?\w+)\s+start:0x([0-9A-F]+) end:0x([0-9A-F]+)", line)
        if m and cur:
            ranges.append((int(m.group(2), 16), int(m.group(3), 16), cur, m.group(1)))
    ranges.sort()
    return ranges


class Collector:
    def __init__(self, tu):
        self.tu = tu
        self.records, self.enums, self.typedefs = {}, {}, {}
        self.functions, self.globals = [], []

    def tref(self, t):
        t = t.get_canonical()
        k = t.kind
        if k == TK.POINTER:
            return {"k": "ptr", "to": self.tref(t.get_pointee())}
        if k == TK.CONSTANTARRAY:
            return {"k": "arr", "of": self.tref(t.element_type), "n": t.element_count}
        if k == TK.INCOMPLETEARRAY:
            return {"k": "arr", "of": self.tref(t.element_type), "n": 0}
        if k == TK.RECORD:
            return {"k": "rec", "id": self.record(t.get_declaration())}
        if k == TK.ENUM:
            return {"k": "enum", "id": self.enum(t.get_declaration()), "size": t.get_size()}
        if k in (TK.FUNCTIONPROTO, TK.FUNCTIONNOPROTO):
            params = [self.tref(a) for a in t.argument_types()] if k == TK.FUNCTIONPROTO else None
            variadic = t.is_function_variadic() if k == TK.FUNCTIONPROTO else False
            return {"k": "fn", "ret": self.tref(t.get_result()), "params": params, "variadic": variadic}
        if k == TK.VOID:
            return {"k": "void"}
        if k in FLOATS:
            return {"k": "float", "size": t.get_size()}
        if k in SIGNED:
            return {"k": "int", "size": t.get_size(), "signed": True}
        if k in UNSIGNED:
            return {"k": "int", "size": t.get_size(), "signed": False}
        return {"k": "unknown", "spelling": t.spelling}

    def record(self, decl):
        decl = decl.get_definition() or decl
        key = decl.get_usr() or f"{self.tu}:{decl.location.line}:{decl.location.column}"
        if key in self.records:
            return key
        entry = {"kind": "union" if decl.kind == CK.UNION_DECL else "struct",
                 "name": decl.spelling, "anonymous": decl.is_anonymous() or "unnamed" in decl.spelling,
                 "file": str(decl.location.file) if decl.location.file else None}
        self.records[key] = entry
        if not decl.is_definition():
            entry["complete"] = False
            return key
        t = decl.type
        entry.update(complete=True, size=t.get_size(), align=t.get_align(), fields=[])
        for f in decl.get_children():
            if f.kind != CK.FIELD_DECL:
                continue
            field = {"name": f.spelling, "offset_bits": t.get_offset(f.spelling) if f.spelling else None,
                     "type": self.tref(f.type)}
            if f.spelling == "":
                field["offset_bits"] = f.get_field_offsetof()
            if f.is_bitfield():
                field["bits"] = f.get_bitfield_width()
            entry["fields"].append(field)
        return key

    def enum(self, decl):
        decl = decl.get_definition() or decl
        key = decl.get_usr() or f"{self.tu}:{decl.location.line}:{decl.location.column}"
        if key not in self.enums:
            self.enums[key] = {"name": decl.spelling,
                               "values": {c.spelling: c.enum_value for c in decl.get_children()
                                          if c.kind == CK.ENUM_CONSTANT_DECL}}
        return key

    def visit(self, tu_cursor, source):
        for c in tu_cursor.get_children():
            if c.kind == CK.TYPEDEF_DECL:
                self.typedefs[c.spelling] = self.tref(c.underlying_typedef_type)
            elif c.kind in (CK.STRUCT_DECL, CK.UNION_DECL) and c.is_definition():
                self.record(c)
            elif c.kind == CK.ENUM_DECL and c.is_definition():
                self.enum(c)
            elif c.kind == CK.FUNCTION_DECL:
                defined_here = c.is_definition() and os.path.normpath(str(c.location.file)) == os.path.normpath(source)
                self.functions.append({
                    "name": c.spelling, "static": c.linkage == ci.LinkageKind.INTERNAL,
                    "defined": defined_here, "type": self.tref(c.type),
                    "param_names": [a.spelling for a in c.get_arguments()],
                })
            elif c.kind == CK.VAR_DECL:
                defined_here = os.path.normpath(str(c.location.file)) == os.path.normpath(source)
                self.globals.append({
                    "name": c.spelling, "static": c.linkage == ci.LinkageKind.INTERNAL,
                    "defined": defined_here and c.storage_class != ci.StorageClass.EXTERN,
                    "type": self.tref(c.type),
                })


def parse_unit(args):
    root, unit_name, source = args
    os.chdir(root)
    index = ci.Index.create()
    tu = index.parse(source, args=FLAGS)
    errors = [f"{d.location}: {d.spelling}" for d in tu.diagnostics if d.severity >= ci.Diagnostic.Error]
    col = Collector(unit_name)
    col.visit(tu.cursor, source)
    return unit_name, source, errors, col.records, col.enums, col.typedefs, col.functions, col.globals


def main():
    root, out = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    os.chdir(root)
    units = [(u["name"].removeprefix("main/"), u["metadata"]["source_path"])
             for u in json.load(open("objdiff.json"))["units"]
             if (u.get("metadata") or {}).get("source_path", "").endswith(".c")]
    symbols = load_symbols("config/GALE01/symbols.txt")
    splits = load_splits("config/GALE01/splits.txt")
    starts = [s[0] for s in splits]

    def tu_of(addr):
        i = bisect.bisect_right(starts, addr) - 1
        return splits[i][2].rsplit(".", 1)[0] if i >= 0 and splits[i][0] <= addr < splits[i][1] else None

    by_name = {}
    for s in symbols:
        s["tu"] = tu_of(s["addr"])
        by_name.setdefault(s["name"], []).append(s)

    records, enums, typedefs, functions, globals_, errors = {}, {}, {}, {}, {}, {}
    with ProcessPoolExecutor() as pool:
        for unit, source, errs, recs, ens, tds, fns, gls in pool.map(
                parse_unit, [(root, u, s) for u, s in units], chunksize=8):
            if errs:
                errors[unit] = errs[:5]
            for k, v in recs.items():
                if k not in records or (v.get("complete") and not records[k].get("complete")):
                    records[k] = v
            enums.update(ens)
            typedefs.update(tds)
            for f in fns:
                if not f["defined"] and f["static"]:
                    continue
                cands = [s for s in by_name.get(f["name"], []) if s["type"] == "function"]
                if f["static"] or len(cands) > 1:
                    cands = [s for s in cands if s["tu"] == unit] or cands
                sym = cands[0] if len(cands) == 1 else None
                key = (sym["addr"] if sym else None, f["name"] if not sym else None)
                entry = functions.get(key)
                if entry is None or (f["defined"] and not entry.get("defined")):
                    functions[key] = {**f, "tu": unit if f["defined"] or f["static"] else (sym or {}).get("tu"),
                                      "addr": sym["addr"] if sym else None, "size": sym["size"] if sym else None}
            for g in gls:
                cands = [s for s in by_name.get(g["name"], []) if s["type"] == "object"]
                if g["static"] or len(cands) > 1:
                    cands = [s for s in cands if s["tu"] == unit] or cands
                sym = cands[0] if len(cands) == 1 else None
                if sym is None:
                    continue
                entry = globals_.get(sym["addr"])
                if entry is None or (g["defined"] and not entry.get("defined")):
                    globals_[sym["addr"]] = {**g, "tu": sym["tu"], "addr": sym["addr"], "size": sym["size"],
                                             "section": sym["section"]}

    json.dump({"records": records, "enums": enums, "typedefs": typedefs,
               "functions": sorted(functions.values(), key=lambda f: (f["addr"] is None, f["addr"] or 0, f["name"])),
               "globals": sorted(globals_.values(), key=lambda g: g["addr"]),
               "symbols": symbols, "errors": errors},
              open(out, "w"), indent=None)
    placed = sum(1 for f in functions.values() if f["addr"] is not None)
    print(f"{len(units)} units, {len(records)} records, {len(enums)} enums, {len(typedefs)} typedefs, "
          f"{placed} functions with addresses ({len(functions)} total), {len(globals_)} globals, "
          f"{len(errors)} units with errors")


if __name__ == "__main__":
    main()
