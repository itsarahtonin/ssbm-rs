#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generates the ssbm-types crate from extract.py's JSON.

Usage: gen_rust.py <types.json> <out dir>

Every C struct becomes a handle type with one accessor per field, every function a call stub
that dispatches by address plus an ABI adapter for registering a Rust port. C names are kept
verbatim so ports read like the decomp.
"""

import json
import os
import re
import sys
import zlib
from collections import defaultdict

KEYWORDS = {
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
    "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof",
    "unsized", "virtual", "yield", "try", "gen",
}
NOT_RAW = {"self", "Self", "super", "crate"}
RESERVED_TYPES = {
    "At", "Handle", "Val", "Ptr", "Addr", "FnPtr", "Arr", "ArrV", "ArrP", "Ctx", "F32", "F64", "StackFrame",
    "Mode", "Entry", "Native", "Backend", "Mmio", "Fault", "Regs", "RegsSnapshot", "Single", "VarArg", "VarArgs",
    "Arg", "Args", "Ret", "Scalar", "Mem", "Pages", "ArgRegs", "Ps", "FpCompare", "FmaMode",
}


def ident(name):
    if name in NOT_RAW:
        return name + "_"
    if name == "_":
        return "__"  # a data-ordering array the decomp names `_`
    if name in KEYWORDS:
        return "r#" + name
    return name


def plain(name):
    """An identifier usable inside a longer name, like set_<name>."""
    return name


class Gen:
    def __init__(self, data):
        self.d = data
        self.records = data["records"]
        self.rec_names = {}
        values = {f["name"] for f in data["functions"]} | {g["name"] for g in data["globals"]}
        values |= {c for e in data["enums"].values() for c in e["values"]}
        used = set()
        for key, r in sorted(self.records.items()):
            name = r["name"]
            if not name or r.get("anonymous") or not re.match(r"^[A-Za-z_]\w*$", name):
                name = "anon_" + format(zlib.crc32(key.encode()), "08x")
            if name in RESERVED_TYPES:
                name = name + "_"
            if name in values:
                name = name + "_t"
            base, n = name, 2
            while name in used:
                name = f"{base}_{n}"
                n += 1
            used.add(name)
            self.rec_names[key] = name
        self.type_names = set(self.rec_names.values()) | set(data["typedefs"])
        self.value_names = values

    # Types.

    def scalar(self, t):
        """(storage type, value type) for a scalar C type, or None."""
        k = t["k"]
        if k in ("int", "enum"):
            size = t.get("size", 4)
            signed = t.get("signed", True) if k == "int" else True
            rt = {1: "8", 2: "16", 4: "32", 8: "64"}[size]
            ty = ("i" if signed else "u") + rt
            return ty, ty
        if k == "float":
            return ("F32", "f64") if t["size"] == 4 else ("F64", "f64")
        return None

    def value_ty(self, t):
        """Rust type of a pointer value pointing at C type t (a handle)."""
        k = t["k"]
        if k == "rec":
            return f"{self.rec_names[t['id']]}<'a>"
        if k in ("void", "unknown"):
            return "Addr<'a>"
        if k == "fn":
            return "FnPtr<'a>"
        s = self.scalar(t)
        if s:
            return f"Val<'a, {s[0]}>"
        if k == "ptr":
            return f"Ptr<'a, {self.value_ty(t['to'])}>"
        if k == "arr":
            return self.handle_ty(t)
        return "Addr<'a>"

    def handle_ty(self, t):
        """Rust type of a handle to storage of C type t."""
        k = t["k"]
        if k == "rec":
            return f"{self.rec_names[t['id']]}<'a>"
        s = self.scalar(t)
        if s:
            return f"Val<'a, {s[0]}>"
        if k == "ptr":
            return f"Ptr<'a, {self.value_ty(t['to'])}>"
        if k == "arr":
            elem, n = t["of"], t["n"]
            es = self.scalar(elem)
            if es:
                return f"ArrV<'a, {es[0]}, {n}>"
            if elem["k"] == "ptr":
                return f"ArrP<'a, {self.value_ty(elem['to'])}, {n}>"
            return f"Arr<'a, {self.handle_ty(elem)}, {n}>"
        return "Addr<'a>"

    # Records.

    def fields(self, rec, base_bits=0):
        """Flattened (name, offset_bits, field) list; anonymous members inline their fields."""
        out = []
        for f in rec.get("fields", []):
            off = (f["offset_bits"] or 0) + base_bits
            if not f["name"]:
                if f["type"]["k"] == "rec":
                    out.extend(self.fields(self.records[f["type"]["id"]], off))
                continue
            out.append((f["name"], off, f))
        return out

    def record(self, key, rec):
        name = self.rec_names[key]
        size = rec.get("size") or 0
        lines = [
            f"/// C {rec['kind']} `{rec['name'] or '(anonymous)'}`, {size:#x} bytes.",
            "#[derive(Clone, Copy, PartialEq, Eq, Debug)]",
            f"pub struct {name}<'a>(pub At<'a>);",
            f"impl<'a> Handle<'a> for {name}<'a> {{",
            f"    const SIZE: u32 = {size:#x};",
            "    #[inline] fn from_at(at: At<'a>) -> Self { Self(at) }",
            "    #[inline] fn at(self) -> At<'a> { self.0 }",
            "}",
        ]
        methods, taken = [], set()

        def emit(mname, sig, body):
            if mname in taken:
                return
            taken.add(mname)
            methods.append(f"    #[inline] pub fn {ident(mname)}{sig} {{ {body} }}")

        for fname, off_bits, f in self.fields(rec):
            t = f["type"]
            byte = off_bits // 8
            if fname == "_":
                fname = f"_{byte:x}"
            if "bits" in f:
                st, vt = self.scalar(t) or ("u32", "u32")
                signed = vt.startswith("i")
                emit(fname, f"(self) -> {vt}", f"self.0.bits(0, {off_bits}, {f['bits']}, {str(signed).lower()}) as {vt}")
                emit(f"set_{fname}", f"(self, v: {vt})", f"self.0.set_bits(0, {off_bits}, {f['bits']}, v as i64)")
                continue
            s = self.scalar(t)
            if s:
                st, vt = s
                emit(fname, f"(self) -> {vt}", f"self.0.get::<{st}>({byte:#x})")
                emit(f"set_{fname}", f"(self, v: {vt})", f"self.0.set::<{st}>({byte:#x}, v)")
                emit(f"{fname}_ref", f"(self) -> Val<'a, {st}>", f"self.0.field({byte:#x})")
            elif t["k"] == "ptr":
                vt = self.value_ty(t["to"])
                emit(fname, f"(self) -> {vt}", f"self.0.ptr({byte:#x})")
                emit(f"set_{fname}", f"(self, v: {vt})", f"self.0.set_ptr({byte:#x}, v)")
                emit(f"{fname}_ref", f"(self) -> Ptr<'a, {vt}>", f"self.0.field({byte:#x})")
            else:
                emit(fname, f"(self) -> {self.handle_ty(t)}", f"self.0.field({byte:#x})")
        if methods:
            lines.append(f"impl<'a> {name}<'a> {{")
            lines.extend(methods)
            lines.append("}")
        return "\n".join(lines)

    # Functions.

    def param(self, t):
        """(Rust parameter type, marshalling template, ABI take type, ABI pass template)."""
        k = t["k"]
        s = self.scalar(t)
        if s:
            if k == "float" and t["size"] == 4:
                return "f64", "Single::round({})", "Single", "{}.0"
            return s[1], "{}", s[1], "{}"
        if k == "ptr":
            vt = self.value_ty(t["to"])
            return vt, "{}", vt, "{}"
        if k == "rec":
            rt = f"{self.rec_names[t['id']]}<'a>"
            return rt, "byval", rt, "{}"
        return "Addr<'a>", "{}", "Addr<'a>", "{}"

    def ret(self, t):
        k = t["k"]
        if k == "void":
            return "()"
        s = self.scalar(t)
        if s:
            return s[1]
        if k == "ptr":
            return self.value_ty(t["to"])
        if k == "rec":
            return "sret"
        return "u32"

    def function(self, f):
        name, addr = f["name"], f["addr"]
        ft = f["type"]
        rid = ident(name)
        if ft.get("params") is None:
            stub = (f"/// `{name}` has no prototype; arguments must already be in registers.\n"
                    f"#[inline] pub fn {rid}(ctx: &Ctx) {{ ctx.invoke({addr:#x}) }}")
            return stub, None
        pnames = list(f.get("param_names") or [])
        params, puts, abi_types, abi_args, abi_pass = [], [], [], [], []
        sret = self.ret(ft["ret"]) == "sret"
        # The EABI returns structs of up to 8 bytes in r3 and r4, not through a pointer.
        small = small_struct(self.records, ft["ret"]) if sret else None
        if small:
            params.append(f"__ret: {self.rec_names[ft['ret']['id']]}<'a>")
        if sret and not small:
            rty = f"{self.rec_names[ft['ret']['id']]}<'a>"
            params.append(f"__ret: {rty}")
            puts.append("__ret")
            abi_types.append(rty)
            abi_args.append("__ret")
            abi_pass.append("__ret")
        pre = []
        for i, pt in enumerate(ft["params"]):
            pn = pnames[i] if i < len(pnames) and pnames[i] else f"arg{i}"
            pn = "a_" + pn if pn in ("ctx", "__ret", "varargs") or pn in self.type_names or pn == "_" else pn
            rty, how, take_ty, pass_ = self.param(pt)
            params.append(f"{ident(pn)}: {rty}")
            abi_types.append(take_ty)
            abi_args.append(ident(pn))
            abi_pass.append(pass_.format(ident(pn)))
            if how == "byval":
                pre.append(f"let {ident(pn)}__copy = ctx.stack_alloc(<{rty} as Handle<'a>>::SIZE); "
                           f"let {ident(pn)}__tmp: {rty} = {ident(pn)}__copy.get(); {ident(pn)}__tmp.copy_from({ident(pn)});")
                puts.append(f"{ident(pn)}__tmp")
            else:
                puts.append(how.format(ident(pn)))
        ret = "()" if sret else self.ret(ft["ret"])
        tuple_ = "(" + "".join(p + ", " for p in puts) + ")"
        if ft.get("variadic"):
            params.append("varargs: &[VarArg]")
            body = "".join(pre) + f"ctx.call_variadic({addr:#x}, {tuple_}, varargs)"
        else:
            body = "".join(pre) + f"ctx.call({addr:#x}, {tuple_})"
        if small:
            body = "".join(pre) + f"ctx.call::<_, ()>({addr:#x}, {tuple_}); ctx.put_small_ret(Handle::addr(__ret), {small})"
        sig = ", ".join(["ctx: &'a Ctx"] + params)
        stub = f"#[inline] pub fn {rid}<'a>({sig}) -> {ret} {{ {body} }}"
        take = (f"let {tuple_args(abi_args)}: ({''.join(t.replace(chr(39) + 'a', chr(39) + '_') + ', ' for t in abi_types)}) = "
                f"Args::take_all(ctx); ")
        if small:
            rty = f"{self.rec_names[ft['ret']['id']]}<'a>"
            fn_types = ", ".join(["&'a Ctx", rty] + [t.replace("Single", "f64") for t in abi_types])
            abi = (f"#[inline] pub fn {rid}(ctx: &Ctx, __f: for<'a> fn({fn_types}) -> ()) {{ {take}"
                   f"let __slot = ctx.stack_alloc(8); __f(ctx, __slot.get(){''.join(', ' + a for a in abi_pass)}); "
                   f"ctx.take_small_ret(__slot.base(), {small}); }}")
            return stub, abi
        fn_types = ", ".join(["&'a Ctx"] + [t.replace("Single", "f64") for t in abi_types])
        abi = (f"#[inline] pub fn {rid}(ctx: &Ctx, __f: for<'a> fn({fn_types}) -> {ret}) {{ {take}"
               f"Ret::put(__f(ctx, {', '.join(abi_pass)}), ctx); }}")
        return stub, abi

    def global_(self, g):
        name, addr, t = g["name"], g["addr"], g["type"]
        return f"#[inline] pub fn {ident(name)}(ctx: &Ctx) -> {self.handle_ty(t).replace(chr(39) + 'a', chr(39) + '_')} {{ At::new(ctx, {addr:#x}).field(0) }}"


def small_struct(records, t):
    """The size of a struct the EABI returns in registers (4 or 8 bytes), else None."""
    if t["k"] != "rec":
        return None
    size = (records.get(t["id"]) or {}).get("size")
    return size if size in (4, 8) else None


def tuple_args(names):
    return "(" + "".join(n + ", " for n in names) + ")"


def tu_mod(tu):
    return re.sub(r"\W", "_", tu.replace("/", "__"))


HEADER = """// SPDX-License-Identifier: GPL-3.0-or-later
// Generated by tools/typegen/gen_rust.py from the decomp's headers. Do not edit.
use ssbm_rt::*;
use crate::records::*;
"""


def main():
    data = json.load(open(sys.argv[1]))
    out = sys.argv[2]
    os.makedirs(out, exist_ok=True)
    g = Gen(data)

    with open(os.path.join(out, "records.rs"), "w", encoding="utf-8", newline="\n") as w:
        w.write(HEADER.replace("use crate::records::*;\n", ""))
        for key in sorted(g.records, key=lambda k: g.rec_names[k]):
            w.write(g.record(key, g.records[key]) + "\n")
        aliases = []
        for tname, t in sorted(data["typedefs"].items()):
            if t["k"] == "rec" and t["id"] in g.rec_names and g.rec_names[t["id"]] != tname \
                    and re.match(r"^[A-Za-z_]\w*$", tname) and tname not in RESERVED_TYPES \
                    and tname not in g.rec_names.values() and tname not in KEYWORDS \
                    and tname not in g.value_names:
                aliases.append(f"pub type {tname}<'a> = {g.rec_names[t['id']]}<'a>;")
        w.write("\n".join(sorted(set(aliases))) + "\n")

    consts, seen = [], {}
    for key, e in sorted(data["enums"].items()):
        for cname, v in e["values"].items():
            if cname in seen:
                continue
            seen[cname] = v
            val = v if -(1 << 31) <= v < (1 << 31) else (v & 0xFFFF_FFFF) - (1 << 32) if v & 0x8000_0000 else v
            consts.append(f"pub const {ident(cname)}: i32 = {val};")
    with open(os.path.join(out, "enums.rs"), "w", encoding="utf-8", newline="\n") as w:
        w.write("// SPDX-License-Identifier: GPL-3.0-or-later\n// Generated by tools/typegen/gen_rust.py. Do not edit.\n")
        w.write("\n".join(consts) + "\n")

    globals_by_tu, fns_by_tu = defaultdict(list), defaultdict(list)
    for gl in data["globals"]:
        if not re.match(r"^[A-Za-z_]\w*$", gl["name"]):
            continue
        globals_by_tu[gl["tu"] if gl["static"] else None].append(gl)
    names_seen = set()
    for f in data["functions"]:
        if f["addr"] is None or not re.match(r"^[A-Za-z_]\w*$", f["name"]):
            continue
        scope = f["tu"] if f["static"] else None
        if scope is None and f["name"] in names_seen:
            scope = f["tu"] or "dup"
        if scope is None:
            names_seen.add(f["name"])
        fns_by_tu[scope].append(f)

    def write_scope(w, gls, fns, indent=""):
        gl_lines = [indent + g.global_(x) for x in gls]
        stubs, abis, addrs = [], [], []
        for f in fns:
            stub, abi = g.function(f)
            stubs.append(indent + stub)
            if abi:
                abis.append(indent + "    " + abi)
            addrs.append(f"{indent}    pub const {ident(f['name'])}: u32 = {f['addr']:#x};")
        w.write("\n".join(gl_lines + stubs) + "\n")
        w.write(f"{indent}/// Addresses of this scope's functions.\n{indent}pub mod addr {{\n" + "\n".join(addrs) + f"\n{indent}}}\n")
        w.write(f"{indent}/// Adapters that register a Rust port under C calling conventions.\n{indent}pub mod abi {{\n"
                f"{indent}    use super::*;\n" + "\n".join(abis) + f"\n{indent}}}\n")

    with open(os.path.join(out, "fns.rs"), "w", encoding="utf-8", newline="\n") as w:
        w.write(HEADER)
        write_scope(w, globals_by_tu[None], fns_by_tu[None])

    tus = sorted(set(k for k in list(globals_by_tu) + list(fns_by_tu) if k))
    with open(os.path.join(out, "tu.rs"), "w", encoding="utf-8", newline="\n") as w:
        w.write(HEADER)
        for tu in tus:
            w.write(f"/// Statics of `{tu}`.\npub mod {tu_mod(tu)} {{\n    use super::*;\n    use crate::fns::*;\n")
            write_scope(w, globals_by_tu.get(tu, []), fns_by_tu.get(tu, []), "    ")
            w.write("}\n")

    syms = sorted((s["addr"], s["size"], s["name"], s["type"]) for s in data["symbols"])
    with open(os.path.join(out, "symbols.rs"), "w", encoding="utf-8", newline="\n") as w:
        w.write("// SPDX-License-Identifier: GPL-3.0-or-later\n// Generated by tools/typegen/gen_rust.py. Do not edit.\n")
        w.write("/// (address, size, name, is_function) for every symbol in the decomp, by address.\n")
        w.write("pub static SYMBOLS: &[(u32, u32, &str, bool)] = &[\n")
        for a, sz, n, t in syms:
            w.write(f"    ({a:#x}, {sz:#x}, {json.dumps(n)}, {str(t == 'function').lower()}),\n")
        w.write("];\n")
        fn_tus = sorted((f["addr"], f["tu"] or "") for f in data["functions"] if f["addr"] is not None)
        w.write("/// (address, translation unit) for every function with a C prototype.\n")
        w.write("pub static FUNCTION_TUS: &[(u32, &str)] = &[\n")
        for a, tu in fn_tus:
            w.write(f"    ({a:#x}, {json.dumps(tu)}),\n")
        w.write("];\n")

    print(f"records {len(g.records)}, enum consts {len(consts)}, functions "
          f"{sum(len(v) for v in fns_by_tu.values())}, globals {sum(len(v) for v in globals_by_tu.values())}, "
          f"TU scopes {len(tus)}")


if __name__ == "__main__":
    main()
