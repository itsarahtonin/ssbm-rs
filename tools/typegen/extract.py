#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Extracts types, globals and function prototypes from the Melee decomp with libclang.

Usage: extract.py <decomp root> <out.json>

Layouts come from clang with the decomp's lint flags (32-bit big-endian PowerPC, layout asserts
as static asserts), plus -fsigned-char to match MWCC. Symbols come from the decomp's config.
"""

import bisect
import ctypes
import json
import os
import re
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor

import clang.cindex as ci

TK = ci.TypeKind
CK = ci.CursorKind

FLAGS = [
    "-xc", "-std=c99", "-nostdinc", "-fno-builtin", "--target=ppc32-none-eabi", "-DLINT",
    "-fno-short-enums", "-fsigned-char", "-DVERSION_GALE01", "-DBUILD_VERSION=0", "-DMUST_MATCH",
    "-Isrc", "-isystemsrc/MSL", "-isystemlibs/dolphin/include", "-isystemlibs/dolphin/src",
    "-isystembuild/GALE01/include", "-isystemlibs/dolphin/src/dolphin", "-fdeclspec",
    # MWCC takes `return;` in a function returning a value, which the decomp's matching code
    # uses where the original returns whatever r3 holds. c2rs reads these warnings, and those
    # about variables used before they are set or that may be, to port such functions from
    # machine code.
    "-Wreturn-type", "-Wno-error=return-type", "-Wuninitialized", "-Wsometimes-uninitialized",
    "-Wconditional-uninitialized",
    "-include", os.path.join(os.path.dirname(os.path.abspath(__file__)), "prelude.h"),
]

SDK_SRC = "libs/dolphin/src"

# MWCC assembly functions: `asm void f(...) {`, with `static` before or after `asm`.
ASM_FN = re.compile(r"^(?:__declspec\((?:[^()]|\([^()]*\))*\)\s*)*(?:static\s+)?asm\s+(?:static\s+)?[^;{()]*?\b(\w+)\s*\([^;{]*?\)\s*\{",
                    re.M | re.S)


def flags_for(source):
    """Clang flags for a unit. The SDK's sources include their private headers from any of its
    folders, as MWCC's -ir allows, and call its intrinsics undeclared."""
    if not source.replace("\\", "/").startswith(SDK_SRC + "/"):
        return list(FLAGS)
    dirs = sorted({d for d, _, files in os.walk(SDK_SRC) if any(f.endswith(".h") for f in files)})
    prelude = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sdk_prelude.h")
    return FLAGS + [f"-I{d}" for d in dirs] + [
        "-include", "src/MetroTRK/intrinsics.h", "-include", prelude,
        "-Wno-error=incompatible-function-pointer-types", "-Wno-error=incompatible-pointer-types",
        "-Wno-error=implicit-function-declaration", "-Wno-error=int-conversion",
    ]


# MWCC's lvalue casts, `((u8*) buf) += n;` or `(u8*) p->buf += n;`, which clang refuses.
LVALUE_CAST = re.compile(r"\(\((\w[\w\s]*\*)\)\s*([\w.>-]+)\)\s*([+-])=\s*([^;]+);")
LVALUE_CAST_BARE = re.compile(r"(?<![\w)])\((\w[\w\s]*\*)\)\s*([\w.>-]+)\s*([+-])=\s*([^;]+);")
# `*((u32*) p)++ = x;`, a store through p as a u32* that steps it past the stored word.
LVALUE_CAST_STORE = re.compile(r"\*\(\((\w[\w\s]*\*)\)\s*([\w.>-]+)\)\+\+\s*=\s*([^;]+);")
# MWCC's variables at fixed addresses, `u16 x : 0x800030E0;`.
FIXED_ADDRESS = re.compile(r"^((?:extern[ \t]+)?(?:volatile[ \t]+)?(?:const[ \t]+)?[A-Za-z_][\w \t]*?[ \t\*]+)"
                           r"([A-Za-z_]\w*)[ \t]*:[ \t]*((?:0x[0-9A-Fa-f]+|\(|[A-Z_]\w*)[^;:\n]*);", re.M)
FIXED_PREFIX = "__c2rs_at_"

INLINE_ASM = re.compile(r"\basm\s*(?:volatile\s*)?\{")


def without_asm_bodies(text):
    """The source with MWCC assembly functions reduced to declarations, and inline `asm { }`
    blocks to a marker call, line numbers kept, since clang cannot read them. Returns (text,
    names of the assembly functions)."""
    text = without_inline_asm(text)
    text = LVALUE_CAST.sub(lambda m: f"{m[2]} = ({m[1]}) {m[2]} {m[3]} ({m[4]});", text)
    text = LVALUE_CAST_BARE.sub(lambda m: f"{m[2]} = ({m[1]}) {m[2]} {m[3]} ({m[4]});", text)
    text = LVALUE_CAST_STORE.sub(lambda m: f"*({m[1]}) {m[2]} = {m[3]}; {m[2]} = (void*) (({m[1]}) {m[2]} + 1);", text)
    # A fixed-address variable is a declaration, and a constant typegen reads its address from.
    text = FIXED_ADDRESS.sub(
        lambda m: f"{m[1]}{m[2]}; static const unsigned long {FIXED_PREFIX}{m[2]} = ({m[3]});", text)
    out, names, at = [], [], 0
    for m in ASM_FN.finditer(text):
        if m.start() < at:
            continue
        depth, i = 0, m.end() - 1
        while i < len(text):
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            if depth == 0:
                break
            i += 1
        head = text[m.start():m.end() - 1]
        decl = re.sub(r"\basm\b", "", head).rstrip() + ";"
        decl = re.sub(r"__declspec\((?:[^()]|\([^()]*\))*\)\s*", "", decl)
        if re.search(r"\bstatic\b", decl) and re.search(
                r"^(?!static)(?!.*\bstatic\b)[^\n;{}#]*\b" + re.escape(m.group(1)) + r"\s*\([^;{]*\)\s*;", text[:m.start()], re.M):
            decl = re.sub(r"\bstatic\s+", "", decl)
        if m.group(1) in ASM_PROTOTYPES:
            decl = ASM_PROTOTYPES[m.group(1)] + ";"
        body = text[m.end() - 1:i + 1]
        out.append(text[at:m.start()] + decl + "\n" * body.count("\n"))
        names.append(m.group(1))
        at = i + 1
    out.append(text[at:])
    return with_mwerks_declarations("".join(out)), names


def with_mwerks_declarations(text):
    """The declarations inside `#ifdef __MWERKS__` blocks without an `#else` repeated after
    them, so the code after the block can use the functions they declare."""
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        if lines[i].strip() == "#ifdef __MWERKS__":
            depth, j, decls, has_else = 1, i + 1, [], False
            while j < len(lines) and depth:
                t = lines[j].strip()
                if t.startswith("#if"):
                    depth += 1
                elif t.startswith("#endif"):
                    depth -= 1
                elif t.startswith("#el") and depth == 1:
                    has_else = True
                elif depth == 1 and re.match(r"^(?:static\s+)?\w[\w\s\*]*\([^;{}]*\)\s*;\s*$", lines[j]):
                    decls.append(lines[j].strip())
                j += 1
            if decls and not has_else and j < len(lines):
                lines[j] = " ".join(decls) + " " + lines[j]
            i = j
            continue
        i += 1
    return "\n".join(lines)


def without_inline_asm(text):
    out, at = [], 0
    for m in INLINE_ASM.finditer(text):
        if m.start() < at:
            continue
        # A whole assembly function starts its line: without_asm_bodies takes those.
        line_start = text.rfind("\n", 0, m.start()) + 1
        if re.match(r"(?:static\s+)?asm\s", text[line_start:m.end()]):
            continue
        depth, i = 0, m.end() - 1
        while i < len(text):
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            if depth == 0:
                break
            i += 1
        block = text[m.start():i + 1]
        out.append(text[at:m.start()] + "__c2rs_inline_asm();" + "\n" * block.count("\n"))
        at = i + 1
    out.append(text[at:])
    return "".join(out)


def sdk_headers():
    """Headers the SDK's sources see as MWCC does: its 32-bit integers are longs there."""
    path = "libs/dolphin/include/dolphin/types.h"
    text = open(path, encoding="utf-8").read()
    return [(path, text.replace("#ifdef __MWERKS__\ntypedef signed long s32;", "#if 1\ntypedef signed long s32;", 1))]


# The register interfaces of MWCC's runtime helpers, which the decomp declares as `void f(void)`
# since only assembly calls them: compiled code calls them for 64-bit division, shifts and
# conversions, with the operands in r3:r4 and r5:r6 (or r5, or f1) and the result in r3:r4 or f1.
ASM_PROTOTYPES = {
    "__div2u": "unsigned long long __div2u(unsigned long long a, unsigned long long b)",
    "__div2i": "long long __div2i(long long a, long long b)",
    "__mod2u": "unsigned long long __mod2u(unsigned long long a, unsigned long long b)",
    "__mod2i": "long long __mod2i(long long a, long long b)",
    "__shl2i": "long long __shl2i(long long a, int n)",
    "__shr2u": "unsigned long long __shr2u(unsigned long long a, int n)",
    "__shr2i": "long long __shr2i(long long a, int n)",
    "__cvt_sll_flt": "float __cvt_sll_flt(long long x)",
    "__cvt_ull_flt": "float __cvt_ull_flt(unsigned long long x)",
    "__cvt_sll_dbl": "double __cvt_sll_dbl(long long x)",
    "__cvt_ull_dbl": "double __cvt_ull_dbl(unsigned long long x)",
    "__cvt_dbl_ull": "unsigned long long __cvt_dbl_ull(double x)",
}
ASM_PROTO_DECL = re.compile(r"\bASM\s+void\s+(\w+)\s*\(\s*void\s*\)\s*;")
ASM_PROTO_DEF = re.compile(r"\bASM\s+void\s+(\w+)\s*\(\s*void\s*\)\s*\{")


def with_asm_prototypes(text):
    """Source with the runtime helpers' definitions under their register interfaces."""
    return ASM_PROTO_DEF.sub(
        lambda m: ASM_PROTOTYPES[m[1]] + " {" if m[1] in ASM_PROTOTYPES else m[0], text)


def runtime_header():
    """The runtime helpers' header with their register interfaces as prototypes."""
    path = "src/Runtime/runtime.h"
    text = open(path, encoding="utf-8").read()
    return [(path, ASM_PROTO_DECL.sub(
        lambda m: ASM_PROTOTYPES[m[1]] + ";" if m[1] in ASM_PROTOTYPES else m[0], text))]


def msl_headers():
    """MSL's stdarg.h as clang should see it: `va_arg` passes __va_arg the class of its type,
    which MWCC computes, as a call c2rs can compute it from."""
    path = "src/MSL/stdarg.h"
    text = open(path, encoding="utf-8").read()
    return [(path, text.replace("#define _var_arg_typeof(e) 0",
                                "unsigned char __c2rs_va_type(void*);\n"
                                "#define _var_arg_typeof(e) __c2rs_va_type((e*) 0)", 1))]


def parse(index, source, extra=()):
    """Parses a unit as its compiler would see it, as far as clang can. Returns (translation
    unit, names of assembly functions left out)."""
    text = open(source, encoding="utf-8", errors="replace").read()
    fixed, asm = without_asm_bodies(with_asm_prototypes(text))
    unsaved = [(source, fixed)] if fixed != text else []
    unsaved += msl_headers() + runtime_header()
    if source.replace("\\", "/").startswith(SDK_SRC + "/"):
        unsaved += sdk_headers()
    args = flags_for(source) + list(extra)
    tu = index.parse(source, args=args, unsaved_files=unsaved or None)
    if any(d.severity >= ci.Diagnostic.Error and "static declaration of" in d.spelling for d in tu.diagnostics):
        # A static definition after a header's non-static prototype, which MWCC takes and
        # clang only allows as a Microsoft extension.
        args += ["-fms-extensions"]
        tu = index.parse(source, args=args, unsaved_files=unsaved or None)
    lines = fixed.split("\n")
    dropped = False
    for d in tu.diagnostics:
        m = re.match(r"conflicting types for '(\w+)'", d.spelling)
        if d.severity < ci.Diagnostic.Error or not m or d.location.file is None or \
                os.path.normpath(d.location.file.name) != os.path.normpath(source):
            continue
        # `T f();` inside a function, after f's prototype: MWCC calls f there as unprototyped,
        # which only changes how it passes arguments already of their promoted types, and
        # clang refuses it. The prototype stands in for it.
        i = d.location.line - 1
        decl = re.compile(r"^\s*(?:extern\s+)?\w[\w\s\*]*\b" + m.group(1) + r"\s*\(\s*\)\s*;\s*$")
        if 0 <= i < len(lines) and decl.match(lines[i]):
            lines[i] = ""
            dropped = True
    if dropped:
        unsaved = [(source, "\n".join(lines))] + [u for u in unsaved if u[0] != source]
        tu = index.parse(source, args=args, unsaved_files=unsaved)
    return tu, asm


SIGNED = {TK.SCHAR, TK.CHAR_S, TK.SHORT, TK.INT, TK.LONG, TK.LONGLONG, TK.WCHAR}
UNSIGNED = {TK.UCHAR, TK.CHAR_U, TK.USHORT, TK.UINT, TK.ULONG, TK.ULONGLONG, TK.BOOL, TK.CHAR16, TK.CHAR32}
FLOATS = {TK.FLOAT, TK.DOUBLE, TK.LONGDOUBLE}


def load_symbols(path):
    syms = []
    # dtk leaves out `scope:` for global symbols.
    pat = re.compile(r"^(\S+) = (\.?\w+):0x([0-9A-Fa-f]+); // type:(\w+)(?: size:0x([0-9A-Fa-f]+))?(?:.*?scope:(\w+))?")
    for line in open(path):
        m = pat.match(line)
        if m:
            syms.append({"name": m.group(1), "section": m.group(2), "addr": int(m.group(3), 16),
                         "type": m.group(4), "size": int(m.group(5) or "0", 16), "scope": m.group(6) or "global"})
    return syms


def linker_symbols(elf, known):
    """Symbols the linker script defines, such as `_stack_end` or `__ArenaLo`, which only the
    built ELF has, as objects C code can declare `extern`."""
    nm = os.path.join("build", "binutils", "powerpc-eabi-nm" + (".exe" if os.name == "nt" else ""))
    if not os.path.exists(elf) or not os.path.exists(nm):
        return []
    out = subprocess.run([nm, elf], capture_output=True, text=True, check=True).stdout
    syms = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[1] == "A" and parts[2] not in known:
            syms.append({"name": parts[2], "section": "", "addr": int(parts[0], 16), "type": "object",
                         "size": 0, "scope": "global"})
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


def evaluate_int(cursor):
    """An integer constant expression's value, or None."""
    res = ci.conf.lib.clang_Cursor_Evaluate(cursor)
    if not res:
        return None
    try:
        if ci.conf.lib.clang_EvalResult_getKind(res) != 1:
            return None
        return ci.conf.lib.clang_EvalResult_getAsLongLong(res)
    finally:
        ci.conf.lib.clang_EvalResult_dispose(res)


ci.conf.lib.clang_Cursor_Evaluate.argtypes = [ci.Cursor]
ci.conf.lib.clang_Cursor_Evaluate.restype = ctypes.c_void_p
ci.conf.lib.clang_EvalResult_getKind.argtypes = [ctypes.c_void_p]
ci.conf.lib.clang_EvalResult_getKind.restype = ctypes.c_int
ci.conf.lib.clang_EvalResult_getAsLongLong.argtypes = [ctypes.c_void_p]
ci.conf.lib.clang_EvalResult_getAsLongLong.restype = ctypes.c_longlong
ci.conf.lib.clang_EvalResult_dispose.argtypes = [ctypes.c_void_p]


class Collector:
    def __init__(self, tu):
        self.tu = tu
        self.records, self.enums, self.typedefs = {}, {}, {}
        self.functions, self.globals = [], []
        self.fixed = {}  # variables at fixed addresses: name -> address
        # Arrays sized by a const variable, which MWCC takes as constant: spelling -> length.
        self.vla_sizes = {}

    def tref(self, t):
        t = t.get_canonical()
        k = t.kind
        if k == TK.POINTER:
            return {"k": "ptr", "to": self.tref(t.get_pointee())}
        if k == TK.CONSTANTARRAY:
            return {"k": "arr", "of": self.tref(t.element_type), "n": t.element_count}
        if k == TK.INCOMPLETEARRAY:
            return {"k": "arr", "of": self.tref(t.element_type), "n": 0}
        if k == TK.VARIABLEARRAY and t.spelling in self.vla_sizes:
            return {"k": "arr", "of": self.tref(t.element_type), "n": self.vla_sizes[t.spelling]}
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
                if c.is_definition():
                    # Structs and enums a function body defines for its locals.
                    for d in c.walk_preorder():
                        if d.kind in (CK.STRUCT_DECL, CK.UNION_DECL) and d.is_definition():
                            self.record(d)
                        elif d.kind == CK.ENUM_DECL and d.is_definition():
                            self.enum(d)
                self.functions.append({
                    "name": c.spelling, "static": c.linkage == ci.LinkageKind.INTERNAL,
                    "defined": defined_here, "type": self.tref(c.type),
                    "param_names": [a.spelling for a in c.get_arguments()],
                })
            elif c.kind == CK.VAR_DECL and c.spelling.startswith(FIXED_PREFIX):
                init = [k for k in c.get_children() if k.kind.is_expression()]
                value = evaluate_int(init[-1]) if init else None
                if value is not None:
                    self.fixed[c.spelling[len(FIXED_PREFIX):]] = value & 0xFFFF_FFFF
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
    # As c2rs does: code for MWCC on the Gekko where the file parses with it.
    tu, asm = parse(index, source, ["-DMWERKS_GEKKO"])
    errors = [f"{d.location}: {d.spelling}" for d in tu.diagnostics if d.severity >= ci.Diagnostic.Error]
    if errors:
        tu, asm = parse(index, source)
        errors = [f"{d.location}: {d.spelling}" for d in tu.diagnostics if d.severity >= ci.Diagnostic.Error]
    col = Collector(unit_name)
    col.visit(tu.cursor, source)
    for f in col.functions:
        # An assembly function is defined here too, though clang sees only its declaration.
        if f["name"] in asm:
            f["asm"] = True
    return (unit_name, source, errors, col.records, col.enums, col.typedefs, col.functions, col.globals,
            col.fixed)


def main():
    root, out = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    os.chdir(root)
    units = [(u["name"].removeprefix("main/"), u["metadata"]["source_path"])
             for u in json.load(open("objdiff.json"))["units"]
             if (u.get("metadata") or {}).get("source_path", "").endswith(".c")]
    symbols = load_symbols("config/GALE01/symbols.txt")
    symbols += linker_symbols("build/GALE01/main.elf", {s["name"] for s in symbols})
    splits = load_splits("config/GALE01/splits.txt")
    starts = [s[0] for s in splits]

    def tu_of(addr):
        i = bisect.bisect_right(starts, addr) - 1
        return splits[i][2].rsplit(".", 1)[0] if i >= 0 and splits[i][0] <= addr < splits[i][1] else None

    by_name = {}
    by_base = {}
    for s in symbols:
        s["tu"] = tu_of(s["addr"])
        by_name.setdefault(s["name"], []).append(s)
        # C++-mangled names, such as sqrtf__Ff: MSL defines some C functions under
        # `#pragma cplusplus on`, so their out-of-line copies carry C++ names.
        m = re.match(r"^([A-Za-z]\w*?)__F\w*$", s["name"])
        if m:
            by_base.setdefault(m.group(1), []).append(s)

    records, enums, typedefs, functions, globals_, errors = {}, {}, {}, {}, {}, {}
    with ProcessPoolExecutor() as pool:
        for unit, source, errs, recs, ens, tds, fns, gls, fixed in pool.map(
                parse_unit, [(root, u, s) for u, s in units], chunksize=8):
            if errs:
                errors[unit] = errs[:5]
            for k, v in recs.items():
                if k not in records or (v.get("complete") and not records[k].get("complete")):
                    records[k] = v
            enums.update(ens)
            typedefs.update(tds)
            for f in fns:
                if not f["defined"] and f["static"] and not f.get("asm"):
                    continue
                cands = [s for s in by_name.get(f["name"], []) if s["type"] == "function"]
                if not cands:
                    cands = [s for s in by_base.get(f["name"], []) if s["type"] == "function"]
                if not cands and not f["defined"]:
                    # A label in assembly that C declares as a function, to take its address.
                    cands = [s for s in by_name.get(f["name"], []) if s["type"] == "label"]
                # A static function is its own unit's, never a same-named one elsewhere.
                if f["static"]:
                    cands = [s for s in cands if s["tu"] == unit]
                elif len(cands) > 1:
                    cands = [s for s in cands if s["tu"] == unit] or cands
                sym = cands[0] if len(cands) == 1 else None
                key = (sym["addr"] if sym else None, f["name"] if not sym else None)
                entry = functions.get(key)
                if entry is None or (f["defined"] and not entry.get("defined")):
                    functions[key] = {**f, "tu": unit if f["defined"] or f["static"] else (sym or {}).get("tu"),
                                      "addr": sym["addr"] if sym else None, "size": sym["size"] if sym else None,
                                      "symbol": sym["name"] if sym else None}
            for g in gls:
                # Labels, such as `_dtors`, name data C declares as extern arrays.
                cands = [s for s in by_name.get(g["name"], []) if s["type"] in ("object", "label")]
                if g["name"] in fixed:
                    cands = [{"name": g["name"], "addr": fixed[g["name"]], "size": 0, "section": "",
                              "tu": unit, "type": "object"}]
                if g["static"]:
                    cands = [s for s in cands if s["tu"] == unit]
                elif len(cands) > 1:
                    cands = [s for s in cands if s["tu"] == unit] or cands
                sym = cands[0] if len(cands) == 1 else None
                if sym is None:
                    continue
                entry = globals_.get((sym["addr"], g["name"]))
                if entry is None or (g["defined"] and not entry.get("defined")):
                    globals_[(sym["addr"], g["name"])] = {**g, "tu": sym["tu"], "addr": sym["addr"], "size": sym["size"],
                                             "section": sym["section"]}

    json.dump({"records": records, "enums": enums, "typedefs": typedefs,
               "functions": sorted(functions.values(), key=lambda f: (f["addr"] is None, f["addr"] or 0, f["name"])),
               "globals": sorted(globals_.values(), key=lambda g: (g["addr"], g["name"])),
               "symbols": symbols, "errors": errors},
              open(out, "w"), indent=None)
    placed = sum(1 for f in functions.values() if f["addr"] is not None)
    print(f"{len(units)} units, {len(records)} records, {len(enums)} enums, {len(typedefs)} typedefs, "
          f"{placed} functions with addresses ({len(functions)} total), {len(globals_)} globals, "
          f"{len(errors)} units with errors")


if __name__ == "__main__":
    main()
