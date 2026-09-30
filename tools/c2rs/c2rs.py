#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Translates the Melee decomp's C into Rust ports that run on ssbm-rt.

Usage: c2rs.py <decomp root> <types.json> <out dir> [unit ...]

Each C file becomes one Rust module. Struct fields go through the generated accessors, calls
go through the generated dispatch stubs, and every float operation goes through gekko-fp,
fused the way MWCC fuses them. A function the translator cannot handle yet (goto, inline asm,
static locals) is left out, so the original keeps running it.
"""

import bisect
import ctypes
import json
import os
import re
import struct
import subprocess
import sys
from collections import defaultdict
from concurrent.futures import ProcessPoolExecutor

import clang.cindex as ci

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "typegen"))
import extract  # noqa: E402
from gen_rust import Gen, ident, tu_mod  # noqa: E402

CK = ci.CursorKind
TK = ci.TypeKind

_lib = ci.conf.lib
_lib.clang_getCursorBinaryOperatorKind.argtypes = [ci.Cursor]
_lib.clang_getCursorBinaryOperatorKind.restype = ctypes.c_int
_lib.clang_getCursorUnaryOperatorKind.argtypes = [ci.Cursor]
_lib.clang_getCursorUnaryOperatorKind.restype = ctypes.c_int
_lib.clang_Cursor_Evaluate.argtypes = [ci.Cursor]
_lib.clang_Cursor_Evaluate.restype = ctypes.c_void_p
_lib.clang_EvalResult_getKind.argtypes = [ctypes.c_void_p]
_lib.clang_EvalResult_getKind.restype = ctypes.c_int
_lib.clang_EvalResult_getAsLongLong.argtypes = [ctypes.c_void_p]
_lib.clang_EvalResult_getAsLongLong.restype = ctypes.c_longlong
_lib.clang_EvalResult_getAsDouble.argtypes = [ctypes.c_void_p]
_lib.clang_EvalResult_getAsDouble.restype = ctypes.c_double
_lib.clang_EvalResult_dispose.argtypes = [ctypes.c_void_p]
_lib.clang_Cursor_getVarDeclInitializer.argtypes = [ci.Cursor]
_lib.clang_Cursor_getVarDeclInitializer.restype = ci.Cursor
_lib.clang_Cursor_getVarDeclInitializer.errcheck = ci.Cursor.from_result


def var_init(decl):
    """A variable declaration's initializer, or None."""
    c = _lib.clang_Cursor_getVarDeclInitializer(decl)
    if c is None or c.kind.is_invalid():
        return None
    return c

BINOPS = {3: "*", 4: "/", 5: "%", 6: "+", 7: "-", 8: "<<", 9: ">>", 11: "<", 12: ">", 13: "<=",
          14: ">=", 15: "==", 16: "!=", 17: "&", 18: "^", 19: "|", 20: "&&", 21: "||", 22: "=",
          23: "*=", 24: "/=", 25: "%=", 26: "+=", 27: "-=", 28: "<<=", 29: ">>=", 30: "&=",
          31: "^=", 32: "|=", 33: ","}
UNOPS = {1: "post++", 2: "post--", 3: "++", 4: "--", 5: "&", 6: "*", 7: "+", 8: "-", 9: "~",
         10: "!", 13: "__extension__"}


class Unsupported(Exception):
    pass


def evaluate(cursor):
    """An integer or float constant expression's value, or None."""
    res = _lib.clang_Cursor_Evaluate(cursor)
    if not res:
        return None
    try:
        kind = _lib.clang_EvalResult_getKind(res)
        if kind == 1:
            return _lib.clang_EvalResult_getAsLongLong(res)
        if kind == 2:
            return _lib.clang_EvalResult_getAsDouble(res)
        return None
    finally:
        _lib.clang_EvalResult_dispose(res)


def children(c):
    return list(c.get_children())


def f32(x):
    return struct.unpack(">f", struct.pack(">f", x))[0]


def rust_float(x):
    if x != x:
        return "f64::NAN"
    if x in (float("inf"), float("-inf")):
        return "f64::INFINITY" if x > 0 else "f64::NEG_INFINITY"
    r = repr(float(x))
    if "e" not in r and "." not in r:
        r += ".0"
    return r + "_f64" if "e" in r else r


# C types, in extract.py's shape.

INT_TYPES = {1: ("i8", "u8"), 2: ("i16", "u16"), 4: ("i32", "u32"), 8: ("i64", "u64")}


def is_int(t):
    return t["k"] in ("int", "enum")


def is_float(t):
    return t["k"] == "float"


def is_ptr(t):
    return t["k"] == "ptr"


def is_scalar(t):
    return is_int(t) or is_float(t) or is_ptr(t)


def int_info(t):
    """(size, signed) of an integer or enum type."""
    if t["k"] == "enum":
        return t.get("size", 4), True
    return t["size"], t["signed"]


INT = {"k": "int", "size": 4, "signed": True}
UINT = {"k": "int", "size": 4, "signed": False}
DOUBLE = {"k": "float", "size": 8}
FLOAT = {"k": "float", "size": 4}


def promote(t):
    if t["k"] == "enum":
        return INT
    size, signed = int_info(t)
    if size < 4:
        return INT
    return t


def arith(a, b):
    """C's usual arithmetic conversions."""
    if is_float(a) or is_float(b):
        if (is_float(a) and a["size"] == 8) or (is_float(b) and b["size"] == 8):
            return DOUBLE
        return FLOAT
    a, b = promote(a), promote(b)
    sa, ua = int_info(a)
    sb, ub = int_info(b)
    size = max(sa, sb)
    if sa == sb:
        signed = ua and ub
    else:
        signed = ua if sa > sb else ub
    return {"k": "int", "size": size, "signed": signed}


class Expr:
    """A translated rvalue: Rust code and its C type."""

    def __init__(self, code, ty, pure=False):
        self.code = code
        self.ty = ty
        self.pure = pure  # no side effects, cheap to repeat

    def __repr__(self):
        return f"Expr({self.code!r})"


class LValue:
    """Somewhere a value can be read from and written to."""

    def __init__(self, ty, read, write, addr=None, pure=True):
        self.ty = ty
        self._read = read
        self._write = write
        self._addr = addr
        self.pure = pure

    def read(self):
        return self._read()

    def write(self, value):
        return self._write(value)

    def addr(self):
        if self._addr is None:
            raise Unsupported("address of a register local or bitfield")
        return self._addr()


class Loop:
    def __init__(self, brk, cont):
        self.brk = brk
        self.cont = cont


class FnCtx:
    """Per-function translation state."""

    def __init__(self, unit, cursor):
        self.unit = unit
        self.cursor = cursor
        self.locals = {}  # USR or name -> (rust name, type, on_stack)
        self.names = set()
        self.frame = []  # (rust name, handle type, size, align)
        self.frame_size = 0
        self.labels = 0
        self.targets = []  # Loop objects and switch labels
        self.escaping = set()
        self.temps = 0
        self.ret = None
        self.sret = False

    def fresh(self, base):
        name = base
        n = 2
        while name in self.names:
            name = f"{base}_{n}"
            n += 1
        self.names.add(name)
        return name

    def temp(self):
        self.temps += 1
        return f"__t{self.temps}"

    def label(self, kind):
        self.labels += 1
        return f"'{kind}{self.labels}"


class Program:
    """What every unit shares: types, names, symbols, the DOL."""

    def __init__(self, root, types_path):
        self.root = root
        self.data = json.load(open(types_path))
        self.gen = Gen(self.data)
        self.functions_by_name = defaultdict(list)
        self.stub_scope = {}
        names_seen = set()
        for f in self.data["functions"]:
            self.functions_by_name[f["name"]].append(f)
            if f["addr"] is None or not re.match(r"^[A-Za-z_]\w*$", f["name"]):
                continue
            scope = f["tu"] if f["static"] else None
            if scope is None and f["name"] in names_seen:
                scope = f["tu"] or "dup"
            if scope is None:
                names_seen.add(f["name"])
            self.stub_scope[f["addr"]] = scope
        self.globals_by_name = defaultdict(list)
        for g in self.data["globals"]:
            self.globals_by_name[g["name"]].append(g)
        self.enum_first = {}
        for e in self.data["enums"].values():
            for cname, v in e["values"].items():
                self.enum_first.setdefault(cname, v)
        self.dol = load_dol(os.path.join(root, "build", "GALE01", "main.dol"))

    def function(self, name, unit, local=False):
        """The function `name` refers to from `unit`. A `local` (static or defined here) one
        must be the unit's own: a same-named function elsewhere is a different function."""
        cands = [f for f in self.functions_by_name.get(name, []) if f["addr"] is not None]
        if local:
            cands = [f for f in cands if f["tu"] == unit]
        elif len(cands) > 1:
            cands = [f for f in cands if f["tu"] == unit] or cands
        return cands[0] if len(cands) == 1 else None

    def global_(self, name, unit):
        cands = self.globals_by_name.get(name, [])
        if len(cands) > 1:
            cands = [g for g in cands if g["tu"] == unit] or cands
        return cands[0] if len(cands) == 1 else None

    def stub_path(self, f):
        scope = self.stub_scope.get(f["addr"])
        if scope is None:
            return f"fns::{ident(f['name'])}"
        return f"statics::{tu_mod(scope)}::{ident(f['name'])}"

    def abi_path(self, f):
        scope = self.stub_scope.get(f["addr"])
        if scope is None:
            return f"fns::abi::{ident(f['name'])}"
        return f"statics::{tu_mod(scope)}::abi::{ident(f['name'])}"

    def global_path(self, g):
        if g["static"]:
            return f"statics::{tu_mod(g['tu'])}::{ident(g['name'])}"
        return f"fns::{ident(g['name'])}"


def load_dol(path):
    """(address, bytes) of every section of the built DOL."""
    raw = open(path, "rb").read()
    sections = []
    for i in range(18):
        off, addr, size = (struct.unpack_from(">I", raw, base + 4 * i)[0] for base in (0, 0x48, 0x90))
        if size:
            sections.append((addr, raw[off:off + size]))
    return sections


class Unit:
    """Translation of one C file."""

    def __init__(self, prog, name, source):
        self.prog = prog
        self.name = name
        self.source = source
        self.col = extract.Collector(name)
        self.inlines = {}  # name -> Rust code of a translated inline function
        self.inline_pending = []
        self.data_ranges = self.read_data_ranges()
        self.skipped = []
        self.ported = []
        self.fuse_check = []

    def read_data_ranges(self):
        """The unit's data sections, and how many fused multiply-adds each function's asm has."""
        path = os.path.join(self.prog.root, "build", "GALE01", "asm", self.name + ".s")
        ranges = []
        self.fused_ops = {}
        self.calls = {}
        self.frame_sizes = {}
        if not os.path.exists(path):
            return ranges
        current = None
        for line in open(path, encoding="utf-8", errors="replace"):
            m = re.match(r"# 0x([0-9A-F]+)\.\.0x([0-9A-F]+) \| size: 0x[0-9A-F]+", line)
            if m:
                ranges.append((int(m.group(1), 16), int(m.group(2), 16)))
                continue
            m = re.match(r"\.fn (\S+),", line)
            if m:
                current = m.group(1)
                self.fused_ops[current] = 0
                self.calls[current] = set()
                continue
            if line.startswith(".endfn"):
                current = None
                continue
            if current and re.search(r"\tf(?:n)?m(?:add|sub)s?\b", line):
                self.fused_ops[current] += 1
            m = re.search(r"\tbl (\S+)", line)
            if current and m:
                self.calls[current].add(m.group(1))
            m = re.search(r"\tstwu r1, -0x([0-9a-fA-F]+)\(r1\)", line)
            if current and m and current not in self.frame_sizes:
                self.frame_sizes[current] = int(m.group(1), 16)
        return ranges

    def string_addr(self, data):
        needle = data + b"\0"
        found = []
        for start, end in self.data_ranges:
            for addr, blob in self.prog.dol:
                if addr <= start and end <= addr + len(blob):
                    chunk = blob[start - addr:end - addr]
                    at = chunk.find(needle)
                    while at >= 0:
                        found.append(start + at)
                        at = chunk.find(needle, at + 1)
        if not found:
            raise Unsupported(f"string literal {data[:24]!r} not found in the unit's data")
        aligned = [a for a in found if a % 4 == 0]
        return (aligned or found)[0]

    # Types.

    def ctype(self, t):
        return self.col.tref(t)

    def rust_value_ty(self, t):
        k = t["k"]
        if is_int(t):
            size, signed = int_info(t)
            return INT_TYPES[size][0 if signed else 1]
        if is_float(t):
            return "f64"
        if k == "ptr":
            return self.prog.gen.value_ty(t["to"])
        if k in ("rec", "arr"):
            return self.prog.gen.handle_ty(t)
        if k == "fn":
            return "FnPtr<'a>"
        if k == "void":
            return "()"
        raise Unsupported(f"type {t}")

    def storage_ty(self, t):
        """The handle type of storage holding a value of C type t."""
        return self.prog.gen.handle_ty(t)

    def size_of(self, t):
        k = t["k"]
        if is_int(t):
            return int_info(t)[0]
        if is_float(t):
            return t["size"]
        if k == "ptr" or k == "fn":
            return 4
        if k == "rec":
            rec = self.prog.data["records"].get(t["id"])
            if not rec or not rec.get("complete"):
                raise Unsupported("incomplete record")
            return rec["size"]
        if k == "arr":
            return self.size_of(t["of"]) * t["n"]
        raise Unsupported(f"size of {t}")

    def align_of(self, t):
        k = t["k"]
        if k == "rec":
            rec = self.prog.data["records"].get(t["id"])
            return rec.get("align", 4) if rec else 4
        if k == "arr":
            return self.align_of(t["of"])
        return min(self.size_of(t), 8)


def is_null_code(code):
    while code.startswith("(") and code.endswith(")"):
        code = code[1:-1]
    return code == "null(ctx)" or (code.startswith("null::<") and code.endswith(">(ctx)"))


def loop_head(cond):
    """`while cond`, or `loop` for a condition that is always true."""
    bare = cond
    while bare.startswith("(") and bare.endswith(")"):
        bare = bare[1:-1]
    if bare in ("true", "1_i32 != 0"):
        return "loop"
    return f"while {cond}"


def is_static(decl):
    return decl.linkage == ci.LinkageKind.INTERNAL


def same_type(a, b):
    return json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True)


def abi_class(t):
    """How a value travels in registers: int, float, struct, or nothing."""
    k = t["k"]
    if k == "void":
        return "void"
    if k == "float":
        return "f"
    if k == "rec":
        return "rec"
    if k in ("int", "enum") and t.get("size", 4) == 8:
        return "i64"
    return "i"


def same_signature(a, b):
    """Whether two prototypes pass arguments and results the same way."""
    if a["k"] != "fn" or b["k"] != "fn" or a.get("params") is None or b.get("params") is None:
        return False
    if bool(a.get("variadic")) != bool(b.get("variadic")) or len(a["params"]) != len(b["params"]):
        return False
    if abi_class(a["ret"]) != abi_class(b["ret"]):
        return False
    return all(abi_class(x) == abi_class(y) for x, y in zip(a["params"], b["params"]))


def strip(c):
    """Skips parentheses and implicit no-op wrappers."""
    while True:
        if c.kind == CK.PAREN_EXPR:
            c = children(c)[0]
        elif c.kind == CK.UNEXPOSED_EXPR and len(children(c)) == 1:
            c = children(c)[0]
        else:
            return c


class Translator:
    def __init__(self, unit, fn_cursor, fuse=None):
        self.u = unit
        self.f = FnCtx(unit.name, fn_cursor)
        self.line = fn_cursor.location.line
        # Whether to contract multiply-adds: only where the original's asm has fused ops.
        self.fuse = fuse if fuse is not None else unit.fused_ops.get(fn_cursor.spelling, 1) > 0
        self.in_args = 0

    # Entry point.

    def function(self):
        c = self.f.cursor
        name = c.spelling
        ft = self.u.ctype(c.type)
        if ft["k"] != "fn" or ft["params"] is None:
            raise Unsupported("no prototype")
        if ft["variadic"]:
            raise Unsupported("variadic definition")
        self.f.ret = ft["ret"]
        body = [x for x in children(c) if x.kind == CK.COMPOUND_STMT]
        if not body:
            raise Unsupported("no body")
        body = body[0]
        self.scan(body)
        params = []
        pre = []
        if ft["ret"]["k"] == "rec":
            self.f.sret = True
            params.append(f"__ret: {self.u.rust_value_ty(ft['ret'])}")
            self.f.names.add("__ret")
        args = [a for a in children(c) if a.kind == CK.PARM_DECL]
        for i, (a, pt) in enumerate(zip(args, ft["params"])):
            pname = a.spelling or f"arg{i}"
            rname = self.f.fresh(self.safe(pname))
            if pt["k"] == "rec":
                # By-value struct: the caller passes a copy's address.
                params.append(f"{ident(rname)}: {self.u.rust_value_ty(pt)}")
                self.f.locals[a.get_usr() or pname] = (rname, pt, "handle")
                continue
            if pt["k"] == "arr":
                pt = {"k": "ptr", "to": pt["of"]}
            params.append(f"{ident(rname)}: {self.u.rust_value_ty(pt)}")
            key = a.get_usr() or pname
            if key in self.f.escaping:
                slot = self.stack_slot(rname + "__slot", pt)
                pre.append(f"{slot}.set({ident(rname)});")
                self.f.locals[key] = (slot, pt, "stack")
            else:
                self.f.locals[key] = (rname, pt, "reg")
                pre.append(f"let mut {ident(rname)} = {ident(rname)};")
        stmts = self.stmts(children(body))
        ret = "" if self.f.ret["k"] == "void" or self.f.sret else " -> " + self.u.rust_value_ty(self.f.ret)
        lines = [f"pub fn {ident(name)}<'a>(ctx: &'a Ctx{''.join(', ' + p for p in params)}){ret} {{"]
        # The port takes the original's frame size, so functions it calls run at the same
        # stack addresses as under the original, and see the same stack leftovers.
        original = self.u.frame_sizes.get(name, 0) if self.f.cursor.linkage != ci.LinkageKind.INTERNAL or \
            name in self.u.frame_sizes else 0
        needed = (8 + self.f.frame_size + 7) & ~7 if self.f.frame else 0
        size = max(original, needed)
        if size:
            lines.append(f"    let __frame = ctx.stack_frame({size:#x});")
            for rname, hty, off in self.f.frame:
                lines.append(f"    let {ident(rname)}: {hty} = frame_at(ctx, &__frame, {off:#x});")
        lines += ["    " + p for p in pre]
        lines += ["    " + s for s in stmts]
        if self.f.ret["k"] != "void" and not self.f.sret and not self.ends_in_return(body):
            lines.append("    #[allow(unreachable_code)]")
            lines.append(f"    return {self.zero(self.f.ret)};")
        lines.append("}")
        return "\n".join(lines)

    def ends_in_return(self, body):
        kids = children(body)
        return bool(kids) and kids[-1].kind == CK.RETURN_STMT

    def safe(self, name):
        if name == "_":
            return "unused"
        if name in self.u.prog.gen.type_names or name in ("enums", "fns", "statics", "support", "ptr", "cstr", "fnptr", "frame_at", "null"):
            return name + "_"
        if name in ("ctx", "__frame") or name.startswith("__"):
            return "v_" + name.lstrip("_")
        return name

    def scan(self, node):
        """Finds locals whose address is taken; they must live on the emulated stack."""
        ext = self.f.cursor.extent
        for start, end in self.u.regions(str(ext.start.file)):
            if start <= ext.end.line and ext.start.line <= end:
                raise Unsupported("MWCC-only code")
        for n in node.walk_preorder():
            if n.kind == CK.GOTO_STMT or n.kind == CK.INDIRECT_GOTO_STMT:
                raise Unsupported("goto")
            if n.kind == CK.ASM_STMT or n.kind == CK.MS_ASM_STMT:
                raise Unsupported("inline asm")
            if n.kind == CK.UNARY_OPERATOR and _lib.clang_getCursorUnaryOperatorKind(n) == 5:
                target = strip(children(n)[0])
                if target.kind == CK.DECL_REF_EXPR and target.referenced is not None and \
                        target.referenced.kind in (CK.VAR_DECL, CK.PARM_DECL):
                    r = target.referenced
                    self.f.escaping.add(r.get_usr() or r.spelling)
            if n.kind == CK.VAR_DECL and n.storage_class == ci.StorageClass.STATIC:
                raise Unsupported("static local")

    # Stack slots.

    def stack_slot(self, name, t):
        size = max(self.u.size_of(t), 1)
        align = max(self.u.align_of(t), 4)
        off = (self.f.frame_size + align - 1) & ~(align - 1)
        self.f.frame_size = off + size
        rname = self.f.fresh(name)
        self.f.frame.append((rname, self.u.storage_ty(t), off))
        return rname

    # Statements.

    def stmts(self, nodes):
        out = []
        for n in nodes:
            out += self.stmt(n)
        return out

    def block(self, node):
        if node.kind == CK.COMPOUND_STMT:
            return self.stmts(children(node))
        return self.stmt(node)

    def indent(self, lines):
        return ["    " + x for x in lines]

    def stmt(self, n):
        self.line = n.location.line
        k = n.kind
        if k == CK.COMPOUND_STMT:
            return ["{"] + self.indent(self.stmts(children(n))) + ["}"]
        if k == CK.DECL_STMT:
            out = []
            for d in children(n):
                out += self.decl(d)
            return out
        if k == CK.RETURN_STMT:
            kids = children(n)
            if not kids:
                return ["return;"]
            if self.f.sret:
                v = self.expr(kids[0])
                return [f"Handle::copy_from(__ret, {v.code});", "return;"]
            v = self.convert(self.expr(kids[0]), self.f.ret)
            return [f"return {v.code};"]
        if k == CK.IF_STMT:
            kids = children(n)
            cond = self.cond(kids[0])
            out = [f"if {cond} {{"] + self.indent(self.block(kids[1])) + ["}"]
            if len(kids) > 2:
                els = self.block(kids[2])
                if kids[2].kind == CK.IF_STMT:
                    out[-1] = "} else " + els[0]
                    out += els[1:]
                else:
                    out[-1] = "} else {"
                    out += self.indent(els) + ["}"]
            return out
        if k == CK.WHILE_STMT:
            cond_node, body = children(n)
            brk, cont = self.f.label("l"), self.f.label("c")
            self.f.targets.append(Loop(brk, cont))
            inner = self.block(body)
            self.f.targets.pop()
            return [f"{brk}: {loop_head(self.cond(cond_node))} {{", f"    {cont}: {{"] + \
                self.indent(self.indent(inner)) + ["    }", "}"]
        if k == CK.DO_STMT:
            body, cond_node = children(n)
            brk, cont = self.f.label("l"), self.f.label("c")
            self.f.targets.append(Loop(brk, cont))
            inner = self.block(body)
            self.f.targets.pop()
            return [f"{brk}: loop {{", f"    {cont}: {{"] + self.indent(self.indent(inner)) + \
                ["    }", f"    if !({self.cond(cond_node)}) {{", f"        break {brk};", "    }", "}"]
        if k == CK.FOR_STMT:
            return self.for_stmt(n)
        if k == CK.BREAK_STMT:
            if not self.f.targets:
                raise Unsupported("break outside a loop")
            t = self.f.targets[-1]
            return [f"break {t.brk};"]
        if k == CK.CONTINUE_STMT:
            loops = [t for t in self.f.targets if t.cont]
            if not loops:
                raise Unsupported("continue outside a loop")
            return [f"break {loops[-1].cont};"]
        if k == CK.SWITCH_STMT:
            return self.switch(n)
        if k == CK.NULL_STMT:
            return []
        if k in (CK.LABEL_STMT, CK.GOTO_STMT):
            raise Unsupported("goto")
        if k in (CK.CASE_STMT, CK.DEFAULT_STMT):
            raise Unsupported("case label outside a switch body")
        # An expression statement.
        return self.effect(n)

    def for_stmt(self, n):
        # libclang lists only the parts that are present; tell them apart by position.
        toks = [t.spelling for t in n.get_tokens()]
        kids = children(n)
        # Find which clauses exist from the parenthesized header.
        depth, semis, i = 0, [], 0
        for i, tok in enumerate(toks):
            if tok == "(":
                depth += 1
            elif tok == ")":
                depth -= 1
                if depth == 0:
                    break
            elif tok == ";" and depth == 1:
                semis.append(i)
        if len(semis) != 2:
            raise Unsupported("for header")
        has_init = semis[0] > 2
        has_cond = semis[1] > semis[0] + 1
        has_incr = i > semis[1] + 1
        idx = 0
        init = kids[idx] if has_init else None
        idx += has_init
        cond_node = kids[idx] if has_cond else None
        idx += has_cond
        incr = kids[idx] if has_incr else None
        idx += has_incr
        body = kids[idx]
        out = ["{"]
        if init is not None:
            out += self.indent(self.stmt(init) if init.kind == CK.DECL_STMT else self.effect(init))
        brk, cont = self.f.label("l"), self.f.label("c")
        self.f.targets.append(Loop(brk, cont))
        inner = self.block(body)
        self.f.targets.pop()
        cond = self.cond(cond_node) if cond_node is not None else "true"
        loop = [f"{brk}: {loop_head(cond)} {{", f"    {cont}: {{"] + self.indent(self.indent(inner)) + ["    }"]
        if incr is not None:
            loop += self.indent(self.effect(incr))
        loop.append("}")
        out += self.indent(loop) + ["}"]
        return out

    def switch(self, n):
        cond_node, body = children(n)
        v = self.convert(self.expr(cond_node), promote(self.u.ctype(cond_node.type)))
        rty = self.u.rust_value_ty(v.ty)
        brk = self.f.label("s")
        self.f.targets.append(Loop(brk, None))
        # Flatten the body into (labels, statements) groups.
        groups = []  # [labels, stmts]
        items = children(body) if body.kind == CK.COMPOUND_STMT else [body]
        decls = []

        def add_labeled(node):
            labels = []
            while node.kind in (CK.CASE_STMT, CK.DEFAULT_STMT):
                kids = children(node)
                if node.kind == CK.CASE_STMT:
                    val = evaluate(kids[0])
                    if val is None:
                        raise Unsupported("case value")
                    labels.append(int(val))
                    node = kids[1]
                else:
                    labels.append("default")
                    node = kids[0]
            groups.append([labels, [node]])

        for it in items:
            if it.kind in (CK.CASE_STMT, CK.DEFAULT_STMT):
                add_labeled(it)
            else:
                if it.kind == CK.DECL_STMT and groups:
                    decls.append(it)
                if not groups:
                    groups.append([[], []])
                groups[-1][1].append(it)
        arms, default_idx = [], None
        for i, (labels, _) in enumerate(groups):
            for lab in labels:
                if lab == "default":
                    default_idx = i
                else:
                    arms.append((lab, i))
        end = len(groups)
        size, signed = int_info(v.ty)
        match = [f"let __case = match {v.code} {{"]
        for lab, i in arms:
            lit = self.int_literal(lab, v.ty)
            match.append(f"    {lit} => {i},")
        match.append(f"    _ => {default_idx if default_idx is not None else end},")
        match.append("};")
        out = [f"{brk}: {{"] + self.indent(match)
        for i, (_, nodes) in enumerate(groups):
            body_lines = []
            for node in nodes:
                body_lines += self.stmt(node)
            if body_lines:
                out += self.indent([f"if __case <= {i} {{"] + self.indent(body_lines) + ["}"])
        out.append("}")
        self.f.targets.pop()
        return out

    def decl(self, d):
        if d.kind != CK.VAR_DECL:
            if d.kind in (CK.STRUCT_DECL, CK.UNION_DECL, CK.ENUM_DECL, CK.TYPEDEF_DECL):
                return []
            raise Unsupported(f"declaration {d.kind}")
        if d.storage_class == ci.StorageClass.STATIC:
            raise Unsupported("static local")
        if d.storage_class == ci.StorageClass.EXTERN:
            return []
        t = self.u.ctype(d.type)
        key = d.get_usr() or d.spelling
        base = self.safe(d.spelling or "anon")
        init = var_init(d)
        on_stack = t["k"] in ("rec", "arr") or key in self.f.escaping
        if on_stack:
            slot = self.stack_slot(base, t)
            self.f.locals[key] = (slot, t, "stack")
            if init is None:
                return []
            return self.initialize(self.stack_lvalue(slot, t), t, init)
        rname = self.f.fresh(base)
        self.f.locals[key] = (rname, t, "reg")
        rty = self.u.rust_value_ty(t)
        if init is None:
            return [f"let mut {ident(rname)}: {rty} = {self.zero(t)};"]
        if init.kind == CK.INIT_LIST_EXPR:
            kids = children(init)
            if len(kids) != 1:
                raise Unsupported("scalar init list")
            init = kids[0]
        refers_to_self = any(n.kind == CK.DECL_REF_EXPR and n.referenced is not None and n.referenced == d
                             for n in init.walk_preorder())
        if refers_to_self:
            out = [f"let mut {ident(rname)}: {rty} = {self.zero(t)};"]
            return out + self.effect(init) if strip(init).kind == CK.BINARY_OPERATOR else \
                out + [f"{ident(rname)} = {self.convert(self.expr(init), t).code};"]
        v = self.convert(self.expr(init), t)
        return [f"let mut {ident(rname)}: {rty} = {v.code};"]

    def initialize(self, lv, t, init):
        """Statements that store initializer `init` into lvalue `lv` of type t."""
        if init.kind == CK.INIT_LIST_EXPR:
            kids = children(init)
            if t["k"] == "arr":
                out = []
                if len(kids) < t["n"]:
                    out.append(f"ctx.fill(Handle::addr({lv.addr_code}), 0, {self.u.size_of(t):#x});")
                for i, k in enumerate(kids):
                    out += self.initialize(self.index_lvalue(lv, t, Expr(str(i), INT, True)), t["of"], k)
                return out
            if t["k"] == "rec":
                rec = self.u.prog.data["records"][t["id"]]
                fields = [f for f in rec["fields"] if f["name"] or f["type"]["k"] == "rec"]
                out = []
                if len(kids) < len(fields) or rec["kind"] == "union":
                    out.append(f"ctx.fill(Handle::addr({lv.addr_code}), 0, {self.u.size_of(t):#x});")
                for f, k in zip(fields, kids):
                    if not f["name"]:
                        raise Unsupported("anonymous member initializer")
                    out += self.initialize(self.field_lvalue(lv, t, f["name"]), f["type"], k)
                    if rec["kind"] == "union":
                        break
                return out
            if len(kids) == 1:
                return self.initialize(lv, t, kids[0])
            raise Unsupported("init list")
        if t["k"] == "arr" and strip(init).kind == CK.STRING_LITERAL:
            raise Unsupported("char array from string")
        v = self.expr(init)
        if t["k"] == "rec":
            return [f"Handle::copy_from({lv.addr_code}, {v.code});"]
        return [lv.write(self.convert(v, t).code) + ";"]

    def zero(self, t):
        if is_int(t):
            return "0"
        if is_float(t):
            return "0.0"
        if t["k"] in ("ptr", "fn"):
            return "null(ctx)"
        raise Unsupported(f"zero of {t}")

    # Lvalues.

    def stack_lvalue(self, slot, t):
        return self.handle_lvalue(ident(slot), t)

    def handle_lvalue(self, h, t, pure=True):
        """The storage a handle expression `h` of C object type t names."""
        k = t["k"]
        if k in ("rec", "arr"):
            lv = LValue(t, lambda: h, None, lambda: h, pure)
            lv._write = lambda v: f"Handle::copy_from({h}, {v})"
        else:
            lv = LValue(t, lambda: f"{h}.get()", lambda v: f"{h}.set({v})", lambda: h, pure)
        lv.addr_code = h
        return lv

    def field_lvalue(self, base, base_t, field):
        """`base.field` where base is a record lvalue."""
        rec = self.u.prog.data["records"].get(base_t["id"])
        if rec is None or not rec.get("complete"):
            raise Unsupported("field of incomplete record")
        found = None
        for name, off, f in self.u.prog.gen.fields(rec):
            if name == field:
                found = (off, f)
                break
        if found is None:
            raise Unsupported(f"field {field}")
        off, f = found
        ft = f["type"]
        h = base.addr_code
        if field == "_":
            field = f"_{off // 8:x}"
        acc = ident(field)
        if "bits" in f:
            size, signed = int_info(ft) if is_int(ft) else (4, False)
            vt = INT_TYPES[size][0 if signed else 1]
            lv = LValue(ft, lambda: f"{h}.{acc}()", lambda v: f"{h}.set_{field}({v})", None, base.pure)
            lv.addr_code = None
            return lv
        if is_scalar(ft):
            lv = LValue(ft, lambda: f"{h}.{acc}()", lambda v: f"{h}.set_{field}({v})",
                        lambda: f"{h}.{field}_ref()", base.pure)
            lv.addr_code = f"{h}.{field}_ref()"
            return lv
        return self.handle_lvalue(f"{h}.{acc}()", ft, base.pure)

    def index_lvalue(self, base, base_t, idx):
        """`base[idx]` where base is an array lvalue."""
        et = base_t["of"]
        h = base.addr_code
        i = self.convert(idx, INT).code
        k = et["k"]
        if is_scalar(et) and not is_ptr(et):
            return self.handle_lvalue(f"{h}.at({i})", et, base.pure and idx.pure)
        if is_ptr(et):
            return self.handle_lvalue(f"{h}.at({i})", et, base.pure and idx.pure)
        if k in ("rec", "arr"):
            return self.handle_lvalue(f"{h}.get({i})", et, base.pure and idx.pure)
        raise Unsupported(f"index into {et}")

    def deref_lvalue(self, ptr):
        """`*ptr` for a pointer value."""
        to = ptr.ty["to"]
        if to["k"] in ("void", "unknown", "fn"):
            raise Unsupported("deref of void or function pointer")
        return self.handle_lvalue(f"({ptr.code})", to, ptr.pure)

    def lvalue(self, c):
        c = strip(c)
        k = c.kind
        if k == CK.DECL_REF_EXPR:
            r = c.referenced
            if r is None:
                raise Unsupported("unresolved name")
            if r.kind in (CK.VAR_DECL, CK.PARM_DECL):
                key = r.get_usr() or r.spelling
                if key in self.f.locals:
                    rname, t, where = self.f.locals[key]
                    if where == "reg":
                        n = ident(rname)
                        lv = LValue(t, lambda: n, lambda v: f"{n} = {v}", None, True)
                        lv.addr_code = None
                        return lv
                    if where == "handle":
                        return self.handle_lvalue(ident(rname), t)
                    return self.stack_lvalue(rname, t)
                g = self.u.prog.global_(r.spelling, self.u.name)
                if g is None:
                    raise Unsupported(f"global {r.spelling} without an address")
                t = self.u.ctype(r.type)
                if not same_type(t, g["type"]):
                    return self.handle_lvalue(f"ptr::<{self.u.storage_ty(t)}>(ctx, {g['addr']:#x})", t)
                return self.handle_lvalue(f"{self.u.prog.global_path(g)}(ctx)", t)
            raise Unsupported(f"lvalue of {r.kind}")
        if k == CK.MEMBER_REF_EXPR:
            kids = children(c)
            if not kids:
                raise Unsupported("implicit member")
            base = kids[0]
            bt = self.u.ctype(base.type)
            arrow = bt["k"] == "ptr"
            if arrow:
                p = self.expr(base)
                if p.ty["k"] != "ptr":
                    raise Unsupported("-> on non-pointer")
                blv = self.deref_lvalue(p)
                return self.field_lvalue(blv, p.ty["to"], c.spelling)
            blv = self.lvalue(base)
            if bt["k"] != "rec":
                raise Unsupported(". on non-record")
            return self.field_lvalue(blv, bt, c.spelling)
        if k == CK.ARRAY_SUBSCRIPT_EXPR:
            a, i = children(c)
            at = self.u.ctype(strip(a).type)
            idx = self.expr(i)
            if at["k"] == "arr" and self.is_pointer_local(strip(a)):
                at = {"k": "ptr", "to": at["of"]}
            if at["k"] == "arr":
                return self.index_lvalue(self.lvalue(a), at, idx)
            p = self.expr(a)
            if p.ty["k"] != "ptr":
                # C allows i[a].
                p, idx = idx, p
                if p.ty["k"] != "ptr":
                    raise Unsupported("subscript")
            return self.deref_lvalue(Expr(f"Handle::add({p.code}, {self.convert(idx, INT).code})", p.ty,
                                          p.pure and idx.pure))
        if k == CK.UNARY_OPERATOR and _lib.clang_getCursorUnaryOperatorKind(c) == 6:
            p = self.expr(children(c)[0])
            if p.ty["k"] != "ptr":
                raise Unsupported("deref of non-pointer")
            return self.deref_lvalue(p)
        if k == CK.CSTYLE_CAST_EXPR:
            raise Unsupported("cast as lvalue")
        if k == CK.CALL_EXPR:
            t = self.u.ctype(c.type)
            if t["k"] == "rec":
                v = self.expr(c)
                return self.handle_lvalue(v.code, t, False)
        if k == CK.COMPOUND_LITERAL_EXPR:
            raise Unsupported("compound literal")
        raise Unsupported(f"lvalue {k}")

    def is_pointer_local(self, c):
        """A reference to a local that C declares as an array but holds a pointer, such as
        an array parameter."""
        if c.kind != CK.DECL_REF_EXPR or c.referenced is None:
            return False
        r = c.referenced
        entry = self.f.locals.get(r.get_usr() or r.spelling)
        return entry is not None and entry[2] == "reg" and is_ptr(entry[1])

    def is_arrow(self, c):
        toks = [t.spelling for t in c.get_tokens()]
        # The operator right before the member name.
        name = c.spelling
        for i in range(len(toks) - 1, 0, -1):
            if toks[i] == name and toks[i - 1] in ("->", "."):
                return toks[i - 1] == "->"
        raise Unsupported("member operator")

    # Expressions.

    def effect(self, c):
        """Statements for an expression evaluated for its side effects."""
        c = strip(c)
        k = c.kind
        if k == CK.BINARY_OPERATOR:
            op = BINOPS.get(_lib.clang_getCursorBinaryOperatorKind(c))
            if op == "=":
                a, b = children(c)
                return self.assign(a, b)
            if op == ",":
                a, b = children(c)
                return self.effect(a) + self.effect(b)
        if k == CK.COMPOUND_ASSIGNMENT_OPERATOR:
            return self.compound(c)
        if k == CK.UNARY_OPERATOR:
            op = UNOPS.get(_lib.clang_getCursorUnaryOperatorKind(c))
            if op in ("++", "--", "post++", "post--"):
                return self.incdec(c, op)
        if k == CK.CSTYLE_CAST_EXPR and self.u.ctype(c.type)["k"] == "void":
            return self.effect(children(c)[-1])
        v = self.expr(c)
        if v.ty["k"] == "void":
            return [f"{v.code};"]
        if v.pure:
            return []
        return [f"let _ = {v.code};"]

    def assign(self, a, b):
        lv = self.lvalue(a)
        t = lv.ty
        if t["k"] == "rec":
            v = self.expr(b)
            return [f"Handle::copy_from({lv.addr_code}, {v.code});"]
        v = self.convert(self.expr(b), t)
        return [lv.write(v.code) + ";"]

    def compound(self, c):
        op = BINOPS[_lib.clang_getCursorBinaryOperatorKind(c)][:-1]
        a, b = children(c)
        lv = self.lvalue(a)
        pre = []
        if not lv.pure:
            raise Unsupported("compound assignment to an impure lvalue")
        cur = Expr(lv.read(), lv.ty, True)
        rhs = self.expr(b)
        if is_ptr(lv.ty):
            if op not in "+-":
                raise Unsupported("pointer compound op")
            n = self.convert(rhs, INT).code
            code = f"Handle::add({cur.code}, {n})" if op == "+" else f"Handle::add({cur.code}, {n}.wrapping_neg())"
            return pre + [lv.write(code) + ";"]
        if op in ("+", "-") and is_float(lv.ty):
            fused = self.try_fuse(op, cur, b, lv.ty)
            if fused is not None:
                return pre + [lv.write(self.convert(fused, lv.ty).code) + ";"]
        if op in ("<<", ">>"):
            res = self.shift(op, self.convert(cur, promote(lv.ty)), self.convert(rhs, promote(rhs.ty)))
        else:
            ct = arith(lv.ty, rhs.ty)
            res = self.arith_op(op, self.convert(cur, ct), self.convert(rhs, ct), ct)
        return pre + [lv.write(self.convert(res, lv.ty).code) + ";"]

    def incdec(self, c, op):
        lv = self.lvalue(children(c)[0])
        if not lv.pure:
            raise Unsupported("increment of an impure lvalue")
        delta = 1 if "+" in op else -1
        cur = lv.read()
        t = lv.ty
        if is_ptr(t):
            return [lv.write(f"Handle::add({cur}, {delta})") + ";"]
        if is_float(t):
            one = self.convert(Expr("1.0", DOUBLE, True), t)
            fn = "fadd" if t["size"] == 8 else "fadds"
            fn = fn if delta > 0 else fn.replace("add", "sub")
            return [lv.write(f"fp::{fn}({cur}, {one.code})") + ";"]
        if delta > 0:
            return [lv.write(f"{cur}.wrapping_add(1)") + ";"]
        return [lv.write(f"{cur}.wrapping_sub(1)") + ";"]

    def expr(self, c):
        self.line = c.location.line
        k = c.kind
        if k == CK.PAREN_EXPR:
            v = self.expr(children(c)[0])
            return Expr(f"({v.code})", v.ty, v.pure)
        if k == CK.UNEXPOSED_EXPR:
            kids = children(c)
            if len(kids) == 1:
                return self.implicit(c, kids[0])
            raise Unsupported("unexposed expression")
        if k == CK.INTEGER_LITERAL:
            t = self.u.ctype(c.type)
            val = evaluate(c)
            if val is None:
                raise Unsupported("integer literal")
            return Expr(self.int_literal(int(val), t), t, True)
        if k == CK.FLOATING_LITERAL:
            t = self.u.ctype(c.type)
            val = evaluate(c)
            if val is None:
                raise Unsupported("float literal")
            if t["size"] == 4:
                val = f32(val)
            return Expr(rust_float(val), t, True)
        if k == CK.CHARACTER_LITERAL:
            val = evaluate(c)
            t = self.u.ctype(c.type)
            return Expr(self.int_literal(int(val), t), t, True)
        if k == CK.STRING_LITERAL:
            data = self.string_bytes(c)
            addr = self.u.string_addr(data)
            return Expr(f"cstr(ctx, {addr:#x})", {"k": "arr", "of": {"k": "int", "size": 1, "signed": True},
                                                   "n": len(data) + 1, "string": True}, True)
        if k == CK.DECL_REF_EXPR:
            return self.decl_ref(c)
        if k == CK.MEMBER_REF_EXPR or k == CK.ARRAY_SUBSCRIPT_EXPR:
            lv = self.lvalue(c)
            if lv.ty["k"] in ("rec", "arr"):
                return Expr(lv.addr_code, lv.ty, lv.pure)
            return Expr(lv.read(), lv.ty, lv.pure)
        if k == CK.UNARY_OPERATOR:
            return self.unary(c)
        if k == CK.BINARY_OPERATOR:
            return self.binary(c)
        if k == CK.COMPOUND_ASSIGNMENT_OPERATOR:
            lv = self.lvalue(children(c)[0])
            stmts = self.compound(c)
            return self.stmt_expr(stmts, Expr(lv.read(), lv.ty, False))
        if k == CK.CONDITIONAL_OPERATOR:
            cond, a, b = children(c)
            t = self.u.ctype(c.type)
            va = self.convert(self.expr(a), t)
            vb = self.convert(self.expr(b), t)
            return Expr(f"(if {self.cond(cond)} {{ {va.code} }} else {{ {vb.code} }})", t,
                        va.pure and vb.pure)
        if k == CK.CALL_EXPR:
            return self.call(c)
        if k == CK.CSTYLE_CAST_EXPR:
            to = self.u.ctype(c.type)
            inner = children(c)[-1]
            if to["k"] == "void":
                return self.stmt_expr(self.effect(inner), Expr("()", to, False))
            return self.convert(self.expr(inner), to, explicit=True)
        if k == CK.CXX_UNARY_EXPR or k == CK.UNARY_EXPR:
            val = evaluate(c)
            if val is None:
                raise Unsupported("sizeof")
            t = self.u.ctype(c.type)
            return Expr(self.int_literal(int(val), t), t, True)
        if k == CK.INIT_LIST_EXPR:
            raise Unsupported("init list expression")
        raise Unsupported(f"expression {k}")

    def stmt_expr(self, stmts, value):
        return Expr("{ " + " ".join(stmts) + f" {value.code} }}", value.ty, False)

    def implicit(self, c, child):
        to = self.u.ctype(c.type)
        # Array to pointer, function to pointer, lvalue to rvalue, or a real conversion.
        inner = strip(child)
        from_t = self.u.ctype(child.type)
        if from_t["k"] == "enum" and is_int(to) and not int_info(to)[1] and int_info(to)[0] == 4:
            # MWCC compiles with `-enum int`: enums are signed ints, where clang makes an
            # enum without negative values unsigned.
            to = INT
        if to["k"] == "ptr" and from_t["k"] == "arr" and self.is_pointer_local(inner):
            v = self.expr(child)
            return self.convert(v, to)
        if to["k"] == "ptr" and from_t["k"] == "arr":
            if inner.kind == CK.STRING_LITERAL:
                v = self.expr(inner)
                return Expr(v.code, {"k": "ptr", "to": from_t["of"]}, True)
            lv = self.lvalue(child)
            return self.convert(self.decay(lv, from_t), to)
        if to["k"] == "ptr" and from_t["k"] == "fn":
            return self.convert(self.fn_value(inner), to)
        v = self.expr(child)
        return self.convert(v, to)

    def decay(self, lv, arr_t):
        et = arr_t["of"]
        h = lv.addr_code
        if is_scalar(et):
            code = f"{h}.at(0)"
        else:
            code = f"{h}.get(0)"
        return Expr(code, {"k": "ptr", "to": et}, lv.pure)

    def fn_value(self, c):
        r = c.referenced
        f = self.u.prog.function(r.spelling, self.u.name, is_static(r)) if r is not None else None
        if f is None:
            raise Unsupported("function pointer to a function without an address")
        return Expr(f"fnptr(ctx, {f['addr']:#x})", {"k": "ptr", "to": self.u.ctype(r.type)}, True)

    def decl_ref(self, c):
        r = c.referenced
        if r is None:
            raise Unsupported("unresolved reference")
        if r.kind == CK.ENUM_CONSTANT_DECL:
            val = r.enum_value
            t = self.u.ctype(c.type)
            name = r.spelling
            if self.u.prog.enum_first.get(name) == val and re.match(r"^[A-Za-z_]\w*$", name):
                return Expr(f"(enums::{ident(name)} as {self.u.rust_value_ty(t)})", t, True)
            return Expr(self.int_literal(val, t), t, True)
        if r.kind == CK.FUNCTION_DECL:
            return self.fn_value(c)
        if r.kind == CK.VAR_DECL and (r.get_usr() or r.spelling) not in self.f.locals \
                and self.u.prog.global_(r.spelling, self.u.name) is None:
            # A header constant (`static const T x = 0;`) that MWCC folded into its uses.
            val = evaluate(c)
            t = self.u.ctype(c.type)
            if val is not None and is_int(t):
                return Expr(self.int_literal(int(val), t), t, True)
            if val is not None and is_float(t):
                return Expr(rust_float(f32(val) if t["size"] == 4 else val), t, True)
        lv = self.lvalue(c)
        if lv.ty["k"] in ("rec", "arr"):
            return Expr(lv.addr_code, lv.ty, True)
        return Expr(lv.read(), lv.ty, True)

    def string_bytes(self, c):
        toks = [t.spelling for t in c.get_tokens()]
        data = b""
        for tok in toks:
            if not tok.startswith('"'):
                raise Unsupported("string literal token")
            data += c_unescape(tok[1:-1])
        return data

    def int_literal(self, val, t):
        size, signed = int_info(t) if is_int(t) else (4, True)
        bits = size * 8
        val &= (1 << bits) - 1
        if signed and val >= 1 << (bits - 1):
            val -= 1 << bits
        rty = INT_TYPES[size][0 if signed else 1]
        if val < 0:
            if val == -(1 << (bits - 1)):
                return f"{rty}::MIN"
            return f"({val}_{rty})"
        if val >= 10 and val > 255:
            return f"{val:#x}_{rty}"
        return f"{val}_{rty}"

    def unary(self, c):
        op = UNOPS.get(_lib.clang_getCursorUnaryOperatorKind(c))
        kid = children(c)[0]
        t = self.u.ctype(c.type)
        if op == "&":
            inner = strip(kid)
            if inner.kind == CK.DECL_REF_EXPR and inner.referenced is not None and \
                    inner.referenced.kind == CK.FUNCTION_DECL:
                return self.fn_value(inner)
            lv = self.lvalue(kid)
            if lv.ty["k"] in ("rec", "arr") or lv.addr_code is not None:
                code = lv.addr_code if lv.ty["k"] in ("rec", "arr") else lv.addr()
                return Expr(code, {"k": "ptr", "to": lv.ty}, lv.pure)
            raise Unsupported("address of a register local")
        if op == "*":
            p = self.expr(kid)
            if p.ty["k"] != "ptr":
                raise Unsupported("deref")
            if p.ty["to"]["k"] == "fn":
                return p
            lv = self.deref_lvalue(p)
            if lv.ty["k"] in ("rec", "arr"):
                return Expr(lv.addr_code, lv.ty, p.pure)
            return Expr(lv.read(), lv.ty, p.pure)
        if op in ("++", "--", "post++", "post--"):
            lv = self.lvalue(kid)
            stmts = self.incdec(c, op)
            if op.startswith("post"):
                tmp = self.f.temp()
                return Expr("{ " + f"let {tmp} = {lv.read()}; " + " ".join(stmts) + f" {tmp} }}", lv.ty, False)
            return self.stmt_expr(stmts, Expr(lv.read(), lv.ty, False))
        if op == "!":
            return Expr(f"(!({self.cond(kid)}) as i32)", INT, True)
        if op == "-":
            v = self.convert(self.expr(kid), t)
            if is_float(t):
                return Expr(f"fp::fneg({v.code})", t, v.pure)
            return Expr(f"{v.code}.wrapping_neg()", t, v.pure)
        if op == "~":
            v = self.convert(self.expr(kid), t)
            return Expr(f"(!{v.code})", t, v.pure)
        if op == "+":
            return self.convert(self.expr(kid), t)
        if op == "__extension__":
            return self.expr(kid)
        raise Unsupported(f"unary {op}")

    def binary(self, c):
        op = BINOPS.get(_lib.clang_getCursorBinaryOperatorKind(c))
        a, b = children(c)
        t = self.u.ctype(c.type)
        if op == "=":
            # The value of an assignment is the value stored; reading the lvalue back would
            # repeat any side effects in it, such as the ++q in `*++q = *++p`.
            lv = self.lvalue(a)
            if lv.ty["k"] == "rec":
                if not lv.pure:
                    raise Unsupported("struct assignment to an impure lvalue as a value")
                stmts = self.assign(a, b)
                return self.stmt_expr(stmts, Expr(lv.addr_code, lv.ty, False))
            v = self.convert(self.expr(b), lv.ty)
            tmp = self.f.temp()
            return Expr("{ " + f"let {tmp} = {v.code}; {lv.write(tmp)}; {tmp}" + " }", lv.ty, False)
        if op == ",":
            return self.stmt_expr(self.effect(a), self.expr(b))
        if op in ("&&", "||"):
            rop = "&&" if op == "&&" else "||"
            return Expr(f"(({self.cond(a)}) {rop} ({self.cond(b)})) as i32", INT, True)
        if op in ("<", ">", "<=", ">=", "==", "!="):
            return Expr(f"({self.compare(op, a, b)}) as i32", INT, True)
        va, vb = self.expr(a), self.expr(b)
        if op in ("+", "-") and (is_ptr(va.ty) or is_ptr(vb.ty)):
            return self.ptr_arith(op, va, vb)
        if op in ("<<", ">>"):
            return self.shift(op, self.convert(va, promote(va.ty)), self.convert(vb, promote(vb.ty)))
        if op in ("+", "-") and is_float(t):
            fused = self.try_fuse_binary(op, a, b, t)
            if fused is not None:
                return fused
        if op in ("/", "%") and is_int(t):
            # Signedness from the operands as MWCC types them (enums are signed there).
            t = arith(va.ty, vb.ty)
        return self.arith_op(op, self.convert(va, t), self.convert(vb, t), t)

    def ptr_arith(self, op, va, vb):
        if is_ptr(va.ty) and is_ptr(vb.ty):
            size = self.u.size_of(va.ty["to"]) if va.ty["to"]["k"] not in ("void", "unknown") else 1
            return Expr(f"((Handle::addr({va.code}).wrapping_sub(Handle::addr({vb.code})) as i32) / {size})", INT,
                        va.pure and vb.pure)
        if is_ptr(vb.ty):
            va, vb = vb, va
        n = self.convert(vb, INT).code
        if op == "-":
            n = f"{n}.wrapping_neg()"
        return Expr(f"Handle::add({va.code}, {n})", va.ty, va.pure and vb.pure)

    def shift(self, op, va, vb):
        t = va.ty
        rty = self.u.rust_value_ty(t)
        n = self.convert(vb, UINT).code
        size, signed = int_info(t)
        if op == "<<":
            fn = "shl"
        else:
            fn = "sar" if signed else "shr"
        return Expr(f"{fn}_{rty}({va.code}, {n})", t, va.pure and vb.pure)

    def arith_op(self, op, va, vb, t):
        pure = va.pure and vb.pure
        if is_float(t):
            names = {"+": "fadd", "-": "fsub", "*": "fmul", "/": "fdiv"}
            if op not in names:
                raise Unsupported(f"float {op}")
            fn = names[op] + ("s" if t["size"] == 4 else "")
            return Expr(f"fp::{fn}({va.code}, {vb.code})", t, pure)
        rty = self.u.rust_value_ty(t)
        if op == "+":
            return Expr(f"{va.code}.wrapping_add({vb.code})", t, pure)
        if op == "-":
            return Expr(f"{va.code}.wrapping_sub({vb.code})", t, pure)
        if op == "*":
            return Expr(f"{va.code}.wrapping_mul({vb.code})", t, pure)
        if op == "/":
            return Expr(f"div_{rty}({va.code}, {vb.code})", t, pure)
        if op == "%":
            return Expr(f"rem_{rty}({va.code}, {vb.code})", t, pure)
        if op in ("&", "|", "^"):
            return Expr(f"({va.code} {op} {vb.code})", t, pure)
        raise Unsupported(f"int {op}")

    # Fused multiply-add, as MWCC contracts them.

    def product(self, node, t):
        """(a, c) if node is a multiply in precision t, else None."""
        n = strip(node)
        if n.kind != CK.BINARY_OPERATOR or BINOPS.get(_lib.clang_getCursorBinaryOperatorKind(n)) != "*":
            return None
        if not same_type(self.u.ctype(n.type), t):
            return None
        a, c = children(n)
        return self.convert(self.expr(a), t), self.convert(self.expr(c), t)

    def fused(self, kind, a, c, b, t):
        fn = {"madd": "fmadd", "msub": "fmsub", "nmsub": "fnmsub", "nmadd": "fnmadd"}[kind]
        fn += "s" if t["size"] == 4 else ""
        return Expr(f"fp::{fn}({a.code}, {c.code}, {b.code})", t, a.pure and b.pure and c.pure)

    def try_fuse_binary(self, op, left, right, t):
        if not self.fuse or self.in_args:
            return None
        p = self.product(left, t)
        if p is not None:
            b = self.convert(self.expr(right), t)
            return self.fused("madd" if op == "+" else "msub", p[0], p[1], b, t)
        p = self.product(right, t)
        if p is not None:
            b = self.convert(self.expr(left), t)
            return self.fused("madd" if op == "+" else "nmsub", p[0], p[1], b, t)
        return None

    def try_fuse(self, op, cur, rhs_node, t):
        """`cur += a * c` and `cur -= a * c`."""
        if not self.fuse or self.in_args:
            return None
        p = self.product(rhs_node, t)
        if p is None:
            return None
        return self.fused("madd" if op == "+" else "nmsub", p[0], p[1], cur, t)

    # Conditions and comparisons.

    def cond(self, c):
        """A Rust bool for a C condition."""
        n = strip(c)
        if n.kind == CK.BINARY_OPERATOR:
            op = BINOPS.get(_lib.clang_getCursorBinaryOperatorKind(n))
            a, b = children(n)
            if op in ("<", ">", "<=", ">=", "==", "!="):
                return self.compare(op, a, b)
            if op == "&&":
                return f"({self.cond(a)}) && ({self.cond(b)})"
            if op == "||":
                return f"({self.cond(a)}) || ({self.cond(b)})"
        if n.kind == CK.UNARY_OPERATOR and UNOPS.get(_lib.clang_getCursorUnaryOperatorKind(n)) == "!":
            return f"!({self.cond(children(n)[0])})"
        v = self.expr(c)
        return self.truthy(v)

    def truthy(self, v):
        t = v.ty
        if is_ptr(t) or t["k"] == "fn":
            return f"!Handle::is_null({v.code})"
        if is_float(t):
            return f"({v.code} != 0.0)"
        if t["k"] == "arr":
            return "true"
        return f"({v.code} != 0)"

    def compare(self, op, a, b):
        va, vb = self.expr(a), self.expr(b)
        if is_ptr(va.ty) or is_ptr(vb.ty) or va.ty["k"] == "fn" or vb.ty["k"] == "fn":
            def addr(v):
                if is_null_code(v.code):
                    return "0"
                return f"Handle::addr({v.code})" if not is_int(v.ty) else f"({v.code} as u32)"
            ca, cb = addr(va), addr(vb)
            if op in ("==", "!=") and "0" in (ca, cb):
                p = va if cb == "0" else vb
                return f"Handle::is_null({p.code})" if op == "==" else f"!Handle::is_null({p.code})"
            return f"{ca} {op} {cb}"
        t = arith(va.ty, vb.ty)
        return f"{self.convert(va, t).code} {op} {self.convert(vb, t).code}"

    # Calls.

    def call(self, c):
        kids = children(c)
        callee = strip(kids[0])
        args = kids[1:]
        t = self.u.ctype(c.type)
        if callee.kind == CK.DECL_REF_EXPR and callee.referenced is not None and \
                callee.referenced.kind == CK.FUNCTION_DECL:
            name = callee.referenced.spelling
            special = self.builtin(name, args, t)
            if special is not None:
                return special
            ft = self.u.ctype(callee.referenced.type)
            f = self.u.prog.function(name, self.u.name, is_static(callee.referenced))
            if f is not None and not same_signature(ft, f["type"]):
                # The decomp declares this function differently here than where its stub came
                # from; call it by address with the types this call site uses.
                return self.raw_call(f["addr"], ft, args, t)
            if f is not None:
                # The stub has the defining unit's types, which may name the same registers
                # differently (u32 for s32, another pointer type).
                st = f["type"]
                argv = self.call_args(st, args)
                path = self.u.prog.stub_path(f)
                if st.get("variadic"):
                    fixed, extra = argv
                    res = Expr(f"{path}(ctx{''.join(', ' + a for a in fixed)}, &[{', '.join(extra)}])",
                               st["ret"], False)
                elif t["k"] == "rec":
                    return self.sret_call(path, argv, t)
                else:
                    res = Expr(f"{path}(ctx{''.join(', ' + a for a in argv)})", st["ret"], False)
                if t["k"] == "void" or st["ret"]["k"] == "void":
                    return Expr(res.code, t, False)
                return self.convert(res, t, explicit=True)
            argv = self.call_args(ft, args, inlined=True)
            defn = callee.referenced.get_definition()
            if defn is None:
                raise Unsupported(f"call to {name}, which has no address or body")
            rname = self.u.request_inline(defn, self.fuse)
            if t["k"] == "rec":
                return self.sret_call(rname, argv, t)
            return Expr(f"{rname}(ctx{''.join(', ' + a for a in argv)})", t, False)
        # Through a function pointer.
        fp_ = self.expr(kids[0])
        ft = fp_.ty["to"] if fp_.ty["k"] == "ptr" else fp_.ty
        if ft["k"] != "fn" or ft["params"] is None:
            raise Unsupported("call through a pointer without a prototype")
        if ft.get("variadic"):
            raise Unsupported("variadic call through a pointer")
        argv = self.call_args(ft, args, marshal=True)
        if t["k"] == "rec":
            raise Unsupported("struct return through a pointer")
        rty = "()" if t["k"] == "void" else self.u.rust_value_ty(t)
        return Expr(f"{fp_.code}.call::<_, {rty}>(({''.join(a + ', ' for a in argv)}))", t, False)

    def raw_call(self, addr, ft, args, t):
        if ft.get("params") is None:
            raise Unsupported("call without a prototype")
        if t["k"] == "rec":
            raise Unsupported("struct return through a mismatched prototype")
        rty = "()" if t["k"] == "void" else self.u.rust_value_ty(t)
        argv = self.call_args(ft, args, marshal=True)
        if ft.get("variadic"):
            fixed, extra = argv
            return Expr(f"ctx.call_variadic::<_, {rty}>({addr:#x}, ({''.join(a + ', ' for a in fixed)}), "
                        f"&[{', '.join(extra)}])", t, False)
        return Expr(f"ctx.call::<_, {rty}>({addr:#x}, ({''.join(a + ', ' for a in argv)}))", t, False)

    def sret_call(self, path, argv, t):
        slot = self.stack_slot("__ret_tmp", t)
        return Expr("{ " + f"{path}(ctx, {ident(slot)}{''.join(', ' + a for a in argv)}); {ident(slot)}" + " }",
                    t, False)

    def call_args(self, ft, args, marshal=False, inlined=False):
        # MWCC does not contract multiply-adds in the arguments of a call it inlines.
        self.in_args += inlined
        try:
            return self._call_args(ft, args, marshal)
        finally:
            self.in_args -= inlined

    def _call_args(self, ft, args, marshal=False):
        params = ft["params"]
        out = []
        for i, a in enumerate(args[:len(params)]):
            pt = params[i]
            if pt["k"] == "rec":
                out.append(self.expr(a).code)
                continue
            if pt["k"] == "arr":
                pt = {"k": "ptr", "to": pt["of"]}
            v = self.convert(self.expr(a), pt)
            if marshal and is_float(pt) and pt["size"] == 4:
                out.append(f"Single(fp::frsp({v.code}))")
            else:
                out.append(v.code)
        if ft.get("variadic"):
            extra = []
            for a in args[len(params):]:
                v = self.expr(a)
                if is_float(v.ty):
                    extra.append(f"VarArg::Float({self.convert(v, DOUBLE).code})")
                elif is_ptr(v.ty) or v.ty["k"] in ("arr", "fn"):
                    extra.append(f"VarArg::Int(Handle::addr({v.code}))")
                elif is_int(v.ty):
                    size, _ = int_info(v.ty)
                    if size == 8:
                        raise Unsupported("64-bit variadic argument")
                    extra.append(f"VarArg::Int({self.convert(v, promote(v.ty)).code} as u32)")
                else:
                    raise Unsupported("variadic argument type")
            return out, extra
        if len(args) != len(params):
            raise Unsupported("argument count")
        return out

    def builtin(self, name, args, t):
        simple = {"__fabs": "fp::fabs", "__fnabs": "fp::fnabs", "__frsqrte": "fp::frsqrte",
                  "__fres": "fp::fres"}
        if name in simple:
            v = self.convert(self.expr(args[0]), DOUBLE)
            return Expr(f"{simple[name]}({v.code})", t, v.pure)
        if name == "__cntlzw":
            v = self.convert(self.expr(args[0]), UINT)
            return Expr(f"({v.code}.leading_zeros() as i32)", t, v.pure)
        if name in ("__sync", "__isync", "__eieio"):
            return Expr("()", {"k": "void"}, False)
        if name.startswith("__builtin") or name in ("__dcbf", "__dcbi", "__dcbst", "__dcbz", "__icbi",
                                                     "__lhbrx", "__sthbrx", "__lwbrx", "__stwbrx", "__mfspr",
                                                     "__mtspr", "__mftb", "__mfmsr", "__mtmsr"):
            raise Unsupported(f"intrinsic {name}")
        return None

    # Conversions.

    def convert(self, v, to, explicit=False):
        fr = v.ty
        if same_type(fr, to):
            return v
        fk, tk = fr["k"], to["k"]
        pure = v.pure
        if tk == "void":
            return Expr(v.code, to, pure)
        if is_int(fr) and is_int(to):
            rty = self.u.rust_value_ty(to)
            if self.u.rust_value_ty(fr) == rty:
                return Expr(v.code, to, pure)
            return Expr(f"({v.code} as {rty})", to, pure)
        if is_int(fr) and is_float(to):
            size, signed = int_info(fr)
            if size == 8:
                raise Unsupported("64-bit int to float")
            code = f"({v.code} as f64)"
            if to["size"] == 4:
                code = f"fp::frsp{code}"
            return Expr(code, to, pure)
        if is_float(fr) and is_float(to):
            if to["size"] == 4:
                return Expr(f"fp::frsp({v.code})", to, pure)
            return Expr(v.code, to, pure)
        if is_float(fr) and is_int(to):
            size, signed = int_info(to)
            if size == 8:
                raise Unsupported("float to 64-bit int")
            if signed or size < 4:
                code = f"fp::fctiwz({v.code})"
                if size < 4:
                    code = f"({code} as {self.u.rust_value_ty(to)})"
                return Expr(code, to, pure)
            f = self.u.prog.function("__cvt_fp2unsigned", self.u.name)
            if f is None:
                raise Unsupported("float to unsigned")
            return Expr(f"cvt_fp2unsigned(ctx, {v.code})", to, False)
        if (fk in ("ptr", "arr", "fn")) and is_int(to):
            size, _ = int_info(to)
            code = f"Handle::addr({v.code})"
            if size != 4:
                code = f"({code} as {self.u.rust_value_ty(to)})"
            elif int_info(to)[1]:
                code = f"({code} as i32)"
            return Expr(code, to, pure)
        if is_int(fr) and tk == "ptr":
            if v.code.strip("()") in ("0_i32", "0_u32", "0"):
                return Expr(f"null::<{self.u.rust_value_ty(to)}>(ctx)", to, True)
            return Expr(f"ptr::<{self.u.rust_value_ty(to)}>(ctx, {v.code} as u32)", to, pure)
        if fk == "ptr" and tk == "ptr":
            a, b = self.u.rust_value_ty(fr), self.u.rust_value_ty(to)
            if a == b:
                return Expr(v.code, to, pure)
            if is_null_code(v.code):
                return Expr(f"null::<{b}>(ctx)", to, True)
            return Expr(f"Handle::cast::<{b}>({v.code})", to, pure)
        if fk == "arr" and tk == "ptr":
            if fr.get("string"):
                return self.convert(Expr(v.code, {"k": "ptr", "to": fr["of"]}, pure), to)
            return self.convert(Expr(f"{v.code}.at(0)" if is_scalar(fr["of"]) else f"{v.code}.get(0)",
                                     {"k": "ptr", "to": fr["of"]}, pure), to)
        if fk == "rec" and tk == "rec":
            return Expr(v.code, to, pure)
        if fk == "fn" and tk == "ptr":
            return Expr(v.code, to, pure)
        raise Unsupported(f"conversion {fk} -> {tk}")


FUSED_RE = re.compile(r"\bfp::f(?:n)?m(?:add|sub)s?\(")
INLINE_CALL_RE = re.compile(r"\b(inl_\w+)\(")


STUB_CALL_RE = re.compile(r"\b(?:fns|statics::\w+)::(\w+)\(ctx")


def fused_count(code, inlines, seen=(), local=None, asm_calls=None):
    """Fused multiply-adds in generated code, counting each inline call's body where it is
    called, as MWCC inlines it, and so too the bodies of this unit's functions that the
    original's asm does not call because MWCC inlined them."""
    n = len(FUSED_RE.findall(code))
    for name in INLINE_CALL_RE.findall(code):
        body = inlines.get(name)
        if body and name not in seen:
            n += fused_count(body.split("{", 1)[1], inlines, seen + (name,), local, None)
    if local is not None and asm_calls is not None:
        for name in STUB_CALL_RE.findall(code):
            body = local.get(name)
            if body and name not in asm_calls and name not in seen:
                n += fused_count(body.split("{", 1)[1], inlines, seen + (name,), local, set())
    return n


def mwcc_regions(path, gekko_defined):
    """Line ranges of preprocessor branches that only MWCC compiles (or only other compilers
    do) and that hold code rather than pragmas. What clang sees there is not what MWCC
    compiled, so functions overlapping them cannot be translated from it."""
    try:
        lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    except OSError:
        return []
    stack = []  # [depends on MWCC, branch start line, branch holds code]
    regions = []

    def mwcc(cond):
        return "__MWERKS__" in cond or ("MWERKS_GEKKO" in cond and not gekko_defined)

    for i, raw in enumerate(lines, 1):
        text = raw.strip()
        if text.startswith("#"):
            d = text[1:].strip()
            if d.startswith(("ifdef", "ifndef", "if")) and not d.startswith("include"):
                stack.append([mwcc(d) or bool(stack and stack[-1][0]), i, False])
                continue
            if d.startswith(("else", "elif")):
                if stack:
                    top = stack[-1]
                    if top[0] and top[2]:
                        regions.append((top[1], i))
                    top[1], top[2] = i, False
                continue
            if d.startswith("endif"):
                if stack:
                    top = stack.pop()
                    if top[0] and top[2]:
                        regions.append((top[1], i))
                continue
            continue
        if stack and stack[-1][0] and text and not text.startswith(("//", "/*", "*")):
            stack[-1][2] = True
    return regions


def c_unescape(s):
    out = bytearray()
    i = 0
    while i < len(s):
        ch = s[i]
        if ch != "\\":
            out += ch.encode("latin-1") if ord(ch) < 256 else ch.encode("utf-8")
            i += 1
            continue
        i += 1
        e = s[i]
        simple = {"n": 10, "t": 9, "r": 13, "0": 0, "\\": 92, '"': 34, "'": 39, "a": 7, "b": 8,
                  "f": 12, "v": 11, "?": 63}
        if e == "x":
            j = i + 1
            while j < len(s) and s[j] in "0123456789abcdefABCDEF":
                j += 1
            out.append(int(s[i + 1:j], 16) & 0xFF)
            i = j
        elif e in "01234567":
            j = i
            while j < len(s) and j < i + 3 and s[j] in "01234567":
                j += 1
            out.append(int(s[i:j], 8) & 0xFF)
            i = j
        elif e in simple:
            out.append(simple[e])
            i += 1
        else:
            raise Unsupported(f"escape \\{e}")
    return bytes(out)


HEADER = """// SPDX-License-Identifier: GPL-3.0-or-later
// Generated by tools/c2rs from {source}. Do not edit; fix the translator or port by hand.
// unit: {unit}
#![allow(unused_mut, unused_variables, unused_assignments, unused_parens, unused_braces)]
#![allow(unused_labels, unreachable_code, unused_imports, unused_comparisons, non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]
use gekko_fp as fp;
use ssbm_rt::*;
use ssbm_types::enums;
use ssbm_types::fns;
use ssbm_types::records::*;
use ssbm_types::tu as statics;

use crate::support::*;
"""


def returns_of(unit, cursor):
    rt = unit.ctype(cursor.type)["ret"]
    if rt["k"] in ("void", "rec"):
        return "Nothing"
    if is_float(rt):
        return "Float"
    if is_int(rt) and int_info(rt)[0] == 8:
        return "Int64"
    return "Int"


def manual_ports(out_dir, unit_name):
    """Functions of this unit ported by hand in `manual/<unit module>.rs`, which the
    translator leaves alone."""
    path = os.path.join(out_dir, "manual", tu_mod(unit_name) + ".rs")
    if not os.path.exists(path):
        return set()
    text = open(path, encoding="utf-8").read()
    return set(re.findall(r"^pub fn (\w+)", text, re.M))


def translate_unit(args):
    root, types_path, unit_name, source, only, out_dir = args
    os.chdir(root)
    prog = PROGRAM[0] if PROGRAM else Program(root, types_path)
    if not PROGRAM:
        PROGRAM.append(prog)
    index = ci.Index.create()
    # Code for MWCC on the Gekko (such as __va_arg) is plain C where the file parses with it.
    gekko = True
    tu = index.parse(source, args=extract.FLAGS + ["-DMWERKS_GEKKO"])
    errors = [d.spelling for d in tu.diagnostics if d.severity >= ci.Diagnostic.Error]
    if errors:
        gekko = False
        tu = index.parse(source, args=extract.FLAGS)
        errors = [d.spelling for d in tu.diagnostics if d.severity >= ci.Diagnostic.Error]
    if errors:
        return unit_name, source, None, [("*", "parse error: " + errors[0])], [], []
    unit = Unit(prog, unit_name, source)
    unit.gekko = gekko
    manual = manual_ports(out_dir, unit_name)
    unit.col.visit(tu.cursor, source)
    out_fns, regs = [], []
    src_norm = os.path.normpath(source)
    for c in tu.cursor.get_children():
        if c.kind != CK.FUNCTION_DECL or not c.is_definition():
            continue
        if os.path.normpath(str(c.location.file)) != src_norm:
            continue
        name = c.spelling
        if only and name not in only:
            continue
        f = prog.function(name, unit_name, local=True)
        if f is None:
            continue  # inlined everywhere; translated on demand
        if name in manual:
            # Ported by hand; register the manual port under the same signature.
            regs.append(f"    ctx.register_port({f['addr']:#x}, {unit.adapter(c, 'manual::' + ident(name))}, "
                        f"Returns::{returns_of(unit, c)});")
            unit.ported.append(name)
            continue
        tr = Translator(unit, c)
        try:
            code = tr.function()
        except Unsupported as e:
            unit.skipped.append((name, f"{e} (line {tr.line})"))
            continue
        except Exception as e:  # noqa: BLE001
            unit.skipped.append((name, f"translator error: {type(e).__name__}: {e} (line {tr.line})"))
            continue
        out_fns.append(code)
        unit.fuse_check.append((name, unit.fused_ops.get(name), code))
        regs.append(f"    ctx.register_port({f['addr']:#x}, {unit.adapter(c, ident(name))}, "
                    f"Returns::{returns_of(unit, c)});")
        unit.ported.append(name)
    inline_code = unit.finish_inlines()
    fuse = []
    local = {name: code for name, _, code in unit.fuse_check}
    for name, asm_n, code in unit.fuse_check:
        if asm_n is not None:
            fuse.append((name, asm_n, fused_count(code.split("{", 1)[1], unit.inlines, (name,), local,
                                                  unit.calls.get(name, set()))))
    unit.fuse_report = fuse
    if not out_fns and not regs:
        return unit_name, source, None, unit.skipped, unit.ported, fuse
    text = HEADER.format(source=source.replace("\\", "/"), unit=unit_name)
    if manual:
        text += f"use crate::manual::{tu_mod(unit_name)} as manual;\n"
    text += "\n"
    text += "\n\n".join(out_fns + inline_code) + "\n\n"
    text += "/// Registers this unit's ports.\npub fn register(ctx: &Ctx) {\n" + "\n".join(regs) + "\n}\n"
    return unit_name, source, text, unit.skipped, unit.ported, fuse


PROGRAM = []


def _request_inline(self, defn, fuse):
    """An inline function's Rust name, translating it on first use. MWCC contracts inlined
    code as its caller's, so there is a copy for each."""
    name = defn.spelling
    rname = "inl_" + name + ("" if fuse else "_unfused")
    if rname not in self.inlines:
        self.inlines[rname] = None
        try:
            code = Translator(self, defn, fuse).function()
            code = code.replace(f"pub fn {ident(name)}<'a>", f"fn {rname}<'a>", 1)
            self.inlines[rname] = code
        except Unsupported as e:
            del self.inlines[rname]
            raise Unsupported(f"inline {name}: {e}")
    return rname


def _regions(self, path):
    key = os.path.normpath(path)
    cache = self.__dict__.setdefault("_region_cache", {})
    if key not in cache:
        cache[key] = mwcc_regions(key, getattr(self, "gekko", False))
    return cache[key]


def _finish_inlines(self):
    return [code for code in self.inlines.values() if code]


def _adapter(self, cursor, rname):
    """A closure that takes the port's arguments from registers and puts its result back."""
    ft = self.ctype(cursor.type)
    names, types, passes = [], [], []
    if ft["ret"]["k"] == "rec":
        names.append("__a")
        types.append(self.rust_value_ty(ft["ret"]).replace("'a", "'_"))
        passes.append("__a")
    for i, pt in enumerate(ft["params"]):
        n = f"a{i}"
        names.append(n)
        if pt["k"] == "arr":
            pt = {"k": "ptr", "to": pt["of"]}
        if is_float(pt) and pt["size"] == 4:
            types.append("Single")
            passes.append(f"{n}.0")
        else:
            types.append(self.rust_value_ty(pt).replace("'a", "'_"))
            passes.append(n)
    take = ""
    if names:
        take = (f"let ({''.join(n + ', ' for n in names)}): ({''.join(t + ', ' for t in types)}) = "
                f"Args::take_all(ctx); ")
    return f"|ctx| {{ {take}Ret::put({rname}(ctx{''.join(', ' + a for a in passes)}), ctx); }}"


Unit.adapter = _adapter
Unit.regions = _regions
Unit.request_inline = _request_inline
Unit.finish_inlines = _finish_inlines


def main():
    root, types_path, out = (os.path.abspath(a) for a in sys.argv[1:4])
    wanted = sys.argv[4:]
    os.chdir(root)
    units = [(u["name"].removeprefix("main/"), u["metadata"]["source_path"])
             for u in json.load(open("objdiff.json"))["units"]
             if (u.get("metadata") or {}).get("source_path", "").endswith(".c")]
    only = {}
    if wanted:
        sel = []
        for w in wanted:
            unit, _, fn = w.partition(":")
            for u in units:
                if u[0] == unit or u[1] == unit:
                    sel.append(u)
                    if fn:
                        only.setdefault(u[0], set()).add(fn)
        units = sorted(set(sel))
    os.makedirs(os.path.join(out, "tu"), exist_ok=True)
    results = []
    jobs = [(root, types_path, u, s, only.get(u), out) for u, s in units]
    if len(jobs) == 1:
        results = [translate_unit(jobs[0])]
    else:
        with ProcessPoolExecutor() as pool:
            results = list(pool.map(translate_unit, jobs, chunksize=4))
    mods = []
    report = {}
    written = []
    total_ported = total_skipped = 0
    fuse_total = fuse_match = 0
    fuse_examples = []
    for unit_name, source, text, skipped, ported, fuse in results:
        for name, asm_n, ours in fuse:
            fuse_total += 1
            if asm_n == ours:
                fuse_match += 1
            elif len(fuse_examples) < 400:
                fuse_examples.append((unit_name, name, asm_n, ours))
        mod = tu_mod(unit_name)
        path = os.path.join(out, "tu", mod + ".rs")
        if text:
            with open(path, "w", encoding="utf-8", newline="\n") as w:
                w.write(text)
            mods.append((mod, unit_name))
            written.append(path)
        elif os.path.exists(path) and not only:
            os.remove(path)
        report[unit_name] = {"ported": ported, "skipped": skipped,
                             "fused": [f for f in fuse if f[1] != f[2]]}
        total_ported += len(ported)
        total_skipped += len(skipped)
    # The module list covers every unit translated so far, not just this run's.
    present = []
    for fname in sorted(os.listdir(os.path.join(out, "tu"))):
        if not fname.endswith(".rs") or fname == "mod.rs":
            continue
        with open(os.path.join(out, "tu", fname), encoding="utf-8") as r:
            head = r.read(512)
        m = re.search(r"^// unit: (\S+)$", head, re.M)
        if m:
            present.append((fname[:-3], m.group(1)))
    with open(os.path.join(out, "tu", "mod.rs"), "w", encoding="utf-8", newline="\n") as w:
        w.write("// SPDX-License-Identifier: GPL-3.0-or-later\n// Generated by tools/c2rs. Do not edit.\n")
        for mod, _ in present:
            w.write(f"pub mod {mod};\n")
        w.write("\n/// Registers a unit's ports.\npub type Register = fn(&ssbm_rt::Ctx);\n")
        w.write("\n/// (unit, register) for every translated unit.\n")
        w.write("pub static UNITS: &[(&str, Register)] = &[\n")
        for mod, unit_name in present:
            w.write(f"    ({json.dumps(unit_name)}, {mod}::register),\n")
        w.write("];\n")
    written.append(os.path.join(out, "tu", "mod.rs"))
    for i in range(0, len(written), 64):
        # A file rustfmt cannot parse will not compile either; the build reports it.
        subprocess.run(["rustfmt", "--edition", "2024", *written[i:i + 64]],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    json.dump(report, open(os.path.join(root, "build", "c2rs-report.json"), "w"), indent=1)
    reasons = defaultdict(int)
    for r in report.values():
        for _, why in r["skipped"]:
            reasons[re.sub(r"\b0x[0-9a-f]+|\d+", "N", why.split(" (line")[0].split(":")[0])] += 1
    print(f"{len(results)} units, {total_ported} functions translated, {total_skipped} left to the original")
    print(f"fused multiply-adds match the asm in {fuse_match} of {fuse_total} functions")
    more = sum(1 for e in fuse_examples if e[3] > e[2])
    print(f"  of the first {len(fuse_examples)} that differ, {more} have more fused ops than the asm")
    for why, n in sorted(reasons.items(), key=lambda x: -x[1])[:25]:
        print(f"  {n:6} {why}")


if __name__ == "__main__":
    main()
