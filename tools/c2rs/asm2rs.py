"""Transliterates a function's machine code into Rust.

For functions whose source is assembly, c2rs has no C to translate. This reads the function's
instruction words from the decomp's dtk listing and writes, for each, what the interpreter does
for it, with the instruction's fields as constants: the same register file, the same memory
accesses, the same floating-point and paired-single operations (gekko_fp), and the same helpers
for condition registers, carries and quantized loads and stores (ssbm_rt::cpu). Branches within
the function become a state machine over its blocks; `bl` becomes a call through dispatch.
"""

import re

LINE = re.compile(r"^/\* ([0-9A-F]{8}) [0-9A-F]{8}  ([0-9A-F]{2} [0-9A-F]{2} [0-9A-F]{2} [0-9A-F]{2}) \*/\t(.*)$")


class AsmUnsupported(Exception):
    pass


def function_words(listing, name):
    """[(pc, word, text)] of `name`'s instructions in a dtk listing, or None."""
    m = re.search(r"^\.fn " + re.escape(name) + r",.*?^\.endfn " + re.escape(name) + r"$", listing, re.M | re.S)
    if not m:
        return None
    out = []
    for line in m.group(0).splitlines():
        lm = LINE.match(line)
        if lm:
            out.append((int(lm.group(1), 16), int(lm.group(2).replace(" ", ""), 16), lm.group(3).strip()))
    return out


def s16(v):
    v &= 0xFFFF
    return v - 0x10000 if v & 0x8000 else v


def hexu(v):
    return f"{v & 0xFFFF_FFFF:#x}_u32"


def rot_mask(mb, me):
    begin = 0xFFFF_FFFF >> mb
    end = (0xFFFF_FFFF << (31 - me)) & 0xFFFF_FFFF
    return (begin & end) if mb <= me else (begin | end)


def g(i):
    return f"g[{i}].get()"


def ra0(a):
    """rA, or 0 for field 0 (D-form and indexed addressing)."""
    return "0_u32" if a == 0 else g(a)


def add(base, off):
    if off == 0:
        return base
    if base == "0_u32":
        return hexu(off)
    return f"{base}.wrapping_add({hexu(off)})"


class Emitter:
    """Rust for one function's instructions."""

    def __init__(self, name, words, jump_targets=()):
        self.name = name
        self.words = words
        self.start = words[0][0]
        self.end = words[-1][0] + 4
        # Where the function's jump tables send its `bctr`s.
        self.jump_targets = {t for t in jump_targets if self.start <= t < self.end}
        # Block starts: the entry, branch targets inside the function, and what follows a branch.
        self.leaders = {self.start}
        for pc, w, _ in words:
            t = self.local_target(pc, w)
            if t is not None:
                self.leaders.add(t)
            if self.ends_block(w):
                self.leaders.add(pc + 4)
        self.leaders |= self.jump_targets
        self.leaders = {x for x in self.leaders if self.start <= x < self.end}

    def inside(self, t):
        return self.start <= t < self.end

    def branch_target(self, pc, w):
        op = w >> 26
        if op == 18:
            disp = ((w & 0x03FF_FFFC) << 6) & 0xFFFF_FFFF
            disp = (disp ^ 0x8000_0000) - 0x8000_0000 >> 6
            return (disp if w & 2 else pc + disp) & 0xFFFF_FFFF
        if op == 16:
            disp = s16(w & 0xFFFC)
            return (disp if w & 2 else pc + disp) & 0xFFFF_FFFF
        return None

    def local_target(self, pc, w):
        t = self.branch_target(pc, w)
        if t is not None and self.inside(t) and not (w & 1):
            return t
        return None

    @staticmethod
    def ends_block(w):
        op = w >> 26
        return op in (16, 18) or (op == 19 and ((w >> 1) & 0x3FF) in (16, 528, 50))

    # Branch conditions, as the interpreter's cond_ok and ctr_ok.

    @staticmethod
    def condition(bo, bi):
        parts = []
        if not bo & 0x04:
            parts.append(f"c::ctr_ok(ctx, {bo})")
        if not bo & 0x10:
            want = "true" if bo & 0x08 else "false"
            parts.append(f"(c::cr_bit(ctx, {bi}) == {want})")
        return " && ".join(parts) if parts else "true"

    def goto(self, pc, t, link):
        """Statements that transfer control to `t`."""
        if link:
            if self.inside(t):
                raise AsmUnsupported(f"call within the function at {pc:#010x}")
            return [f"c::call(ctx, {hexu(t)}, {hexu(pc + 4)});"]
        if self.inside(t):
            return [f"pc = {hexu(t)};", "continue;"]
        # A tail call: the callee returns to our caller.
        return [f"c::tail_call(ctx, {hexu(t)});", "return;"]

    def emit(self, pc, w):
        """Rust statements for the instruction `w` at `pc`, as the interpreter's step does it."""
        op = w >> 26
        d, a, b, cc = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31, (w >> 6) & 31
        simm = s16(w)
        uimm = w & 0xFFFF
        rc = w & 1
        if op == 3:
            return [f"if c::trap({d}, {g(a)}, {hexu(simm)}) {{ panic!(\"twi trap at {pc:#010x}\"); }}"]
        if op == 4:
            return self.paired(pc, w)
        if op == 7:
            return [f"g[{d}].set(({g(a)} as i32).wrapping_mul({simm}_i32) as u32);"]
        if op == 8:
            return [f"{{ let (v, ca, _) = c::add3(!{g(a)}, {hexu(simm)}, 1); g[{d}].set(v); c::set_ca(ctx, ca); }}"]
        if op == 10:
            return [f"{{ let (x, y) = ({g(a)}, {hexu(uimm)}); c::compare(ctx, {(w >> 23) & 7}, x < y, x > y); }}"]
        if op == 11:
            return [f"{{ let (x, y) = ({g(a)} as i32, {simm}_i32); c::compare(ctx, {(w >> 23) & 7}, x < y, x > y); }}"]
        if op in (12, 13):
            out = f"{{ let (v, ca, _) = c::add3({g(a)}, {hexu(simm)}, 0); g[{d}].set(v); c::set_ca(ctx, ca);"
            if op == 13:
                out += " c::update_cr0(ctx, v);"
            return [out + " }"]
        if op == 14:
            return [f"g[{d}].set({add(ra0(a), simm)});"]
        if op == 15:
            return [f"g[{d}].set({add(ra0(a), (w << 16) & 0xFFFF_FFFF)});"]
        if op == 16:
            bo, bi = d, a
            t = self.branch_target(pc, w)
            link = bool(w & 1)
            body = self.goto(pc, t, link)
            cond = self.condition(bo, bi)
            out = []
            if link:
                # The interpreter sets LR whether or not the branch is taken.
                out.append(f"ctx.regs.lr.set({hexu(pc + 4)});")
            if cond == "true":
                return out + body
            if link:
                return out + [f"if {cond} {{"] + ["    " + x for x in body] + ["}"]
            return [f"if {cond} {{"] + ["    " + x for x in body] + ["}"]
        if op == 17:
            return []  # sc: the SDK layer covers what the handler would do.
        if op == 18:
            t = self.branch_target(pc, w)
            return self.goto(pc, t, bool(w & 1))
        if op == 19:
            return self.op19(pc, w)
        if op == 20:
            m = rot_mask(cc, (w >> 1) & 31)
            out = f"{{ let v = ({g(d)}.rotate_left({b}) & {hexu(m)}) | ({g(a)} & {hexu(~m)}); g[{a}].set(v);"
            if rc:
                out += " c::update_cr0(ctx, v);"
            return [out + " }"]
        if op in (21, 23):
            sh = str(b) if op == 21 else f"({g(b)} & 31)"
            m = rot_mask(cc, (w >> 1) & 31)
            out = f"{{ let v = {g(d)}.rotate_left({sh}) & {hexu(m)}; g[{a}].set(v);"
            if rc:
                out += " c::update_cr0(ctx, v);"
            return [out + " }"]
        if op == 24:
            return [f"g[{a}].set({g(d)} | {hexu(uimm)});"]
        if op == 25:
            return [f"g[{a}].set({g(d)} | {hexu(uimm << 16)});"]
        if op == 26:
            return [f"g[{a}].set({g(d)} ^ {hexu(uimm)});"]
        if op == 27:
            return [f"g[{a}].set({g(d)} ^ {hexu(uimm << 16)});"]
        if op in (28, 29):
            imm = uimm if op == 28 else uimm << 16
            return [f"{{ let v = {g(d)} & {hexu(imm)}; g[{a}].set(v); c::update_cr0(ctx, v); }}"]
        if op == 31:
            return self.op31(pc, w)
        if 32 <= op <= 55:
            return self.load_store(pc, w)
        if op in (56, 57, 60, 61):
            upd = op in (57, 61)
            disp = ((w & 0xFFF) ^ 0x800) - 0x800
            base = g(a) if upd else ra0(a)
            ea = add(base, disp)
            fn = "psq_load" if op in (56, 57) else "psq_store"
            out = f"{{ let ea = {ea}; c::{fn}(ctx, ea, {d}, {'true' if (w >> 15) & 1 else 'false'}, {(w >> 12) & 7});"
            if upd:
                out += f" g[{a}].set(ea);"
            return [out + " }"]
        if op == 59:
            return self.fp_single(pc, w)
        if op == 63:
            return self.fp_double(pc, w)
        raise AsmUnsupported(f"instruction {w:08x} at {pc:#010x}")

    def op19(self, pc, w):
        d, a, b = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31
        xo = (w >> 1) & 0x3FF
        link = bool(w & 1)
        if xo == 0:
            return [f"c::set_cr_field(ctx, {(w >> 23) & 7}, c::cr_field(ctx, {(w >> 18) & 7}));"]
        if xo in (16, 528):
            reg = "lr" if xo == 16 else "ctr"
            cond = self.condition(d, a) if xo == 16 else self.condition(d | 0x04, a)
            if not link:
                if xo == 16:
                    # A return, unless the function put another address in LR to jump to.
                    body = ["let to = ctx.regs.lr.get() & !3;",
                            "if to != lr0 & !3 { c::tail_call(ctx, to); }",
                            "return;"]
                elif self.jump_targets:
                    # Through a jump table to a block of the function, or out of it.
                    body = ["let to = ctx.regs.ctr.get() & !3;",
                            f"if ({hexu(self.start)}..{hexu(self.end)}).contains(&to) {{ pc = to; continue; }}",
                            "c::tail_call(ctx, to);",
                            "return;"]
                else:
                    # Only a jump out: the function has no jump tables.
                    body = ["let to = ctx.regs.ctr.get() & !3;",
                            f"assert!(!({hexu(self.start)}..{hexu(self.end)}).contains(&to), \"{self.name}: bctr within the function\");",
                            "c::tail_call(ctx, to);",
                            "return;"]
            else:
                body = [f"c::call(ctx, ctx.regs.{reg}.get() & !3, {hexu(pc + 4)});"]
            if cond == "true":
                return body
            out = [f"if {cond} {{"] + ["    " + x for x in body] + ["}"]
            if link:
                out.append(f"else {{ ctx.regs.lr.set({hexu(pc + 4)}); }}")
            return out
        ops = {33: "!(x | y)", 129: "x & !y", 193: "x ^ y", 225: "!(x & y)", 257: "x & y", 289: "x == y",
               417: "x | !y", 449: "x | y"}
        if xo in ops:
            return [f"{{ let (x, y) = (c::cr_bit(ctx, {a}), c::cr_bit(ctx, {b})); c::set_cr_bit(ctx, {d}, {ops[xo]}); }}"]
        if xo == 150:
            return []  # isync
        if xo == 50:
            return [f"panic!(\"rfi at {pc:#010x}: exceptions are not emulated\");"]
        raise AsmUnsupported(f"instruction {w:08x} at {pc:#010x}")

    def op31(self, pc, w):
        d, a, b = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31
        rc = bool(w & 1)
        xo = (w >> 1) & 0x3FF
        ea_x = f"{ra0(a)}.wrapping_add({g(b)})" if a else g(b)
        ea_ux = f"{g(a)}.wrapping_add({g(b)})"

        def logical(v):
            out = f"{{ let v = {v}; g[{a}].set(v);"
            if rc:
                out += " c::update_cr0(ctx, v);"
            return [out + " }"]

        def upd(stmts):
            return [f"{{ let ea = {ea_ux}; {stmts} g[{a}].set(ea); }}"]

        if xo in (0, 32):
            if xo == 0:
                return [f"{{ let (x, y) = ({g(a)} as i32, {g(b)} as i32); c::compare(ctx, {(w >> 23) & 7}, x < y, x > y); }}"]
            return [f"{{ let (x, y) = ({g(a)}, {g(b)}); c::compare(ctx, {(w >> 23) & 7}, x < y, x > y); }}"]
        if xo == 4:
            return [f"if c::trap({d}, {g(a)}, {g(b)}) {{ panic!(\"tw trap at {pc:#010x}\"); }}"]
        if xo == 19:
            return [f"g[{d}].set(ctx.regs.cr.get());"]
        if xo in (20, 23):
            return [f"g[{d}].set(ctx.read_u32({ea_x}));"]
        if xo == 55:
            return upd(f"g[{d}].set(ctx.read_u32(ea));")
        if xo == 87:
            return [f"g[{d}].set(u32::from(ctx.read_u8({ea_x})));"]
        if xo == 119:
            return upd(f"g[{d}].set(u32::from(ctx.read_u8(ea)));")
        if xo == 279:
            return [f"g[{d}].set(u32::from(ctx.read_u16({ea_x})));"]
        if xo == 311:
            return upd(f"g[{d}].set(u32::from(ctx.read_u16(ea)));")
        if xo == 343:
            return [f"g[{d}].set(ctx.read_u16({ea_x}) as i16 as u32);"]
        if xo == 375:
            return upd(f"g[{d}].set(ctx.read_u16(ea) as i16 as u32);")
        if xo == 534:
            return [f"g[{d}].set(ctx.read_u32({ea_x}).swap_bytes());"]
        if xo == 790:
            return [f"g[{d}].set(u32::from(ctx.read_u16({ea_x}).swap_bytes()));"]
        if xo == 150:
            return [f"ctx.write_u32({ea_x}, {g(d)}); c::set_cr_field(ctx, 0, 2 | c::so(ctx));"]
        if xo == 151:
            return [f"ctx.write_u32({ea_x}, {g(d)});"]
        if xo == 183:
            return upd(f"ctx.write_u32(ea, {g(d)});")
        if xo == 215:
            return [f"ctx.write_u8({ea_x}, {g(d)} as u8);"]
        if xo == 247:
            return upd(f"ctx.write_u8(ea, {g(d)} as u8);")
        if xo == 407:
            return [f"ctx.write_u16({ea_x}, {g(d)} as u16);"]
        if xo == 439:
            return upd(f"ctx.write_u16(ea, {g(d)} as u16);")
        if xo == 662:
            return [f"ctx.write_u32({ea_x}, {g(d)}.swap_bytes());"]
        if xo == 918:
            return [f"ctx.write_u16({ea_x}, ({g(d)} as u16).swap_bytes());"]
        if xo == 535:
            return [f"c::fill(ctx, {d}, fp::lfs(ctx.read_u32({ea_x})));"]
        if xo == 567:
            return upd(f"c::fill(ctx, {d}, fp::lfs(ctx.read_u32(ea)));")
        if xo == 599:
            return [f"ctx.regs.set_f({d}, f64::from_bits(ctx.read_u64({ea_x})));"]
        if xo == 631:
            return upd(f"ctx.regs.set_f({d}, f64::from_bits(ctx.read_u64(ea)));")
        if xo == 663:
            return [f"ctx.write_u32({ea_x}, fp::stfs(ctx.regs.f({d})));"]
        if xo == 695:
            return upd(f"ctx.write_u32(ea, fp::stfs(ctx.regs.f({d})));")
        if xo == 727:
            return [f"ctx.write_u64({ea_x}, ctx.regs.f({d}).to_bits());"]
        if xo == 759:
            return upd(f"ctx.write_u64(ea, ctx.regs.f({d}).to_bits());")
        if xo == 983:
            return [f"ctx.write_u32({ea_x}, ctx.regs.f({d}).to_bits() as u32);"]
        if xo == 24:
            return logical(f"{{ let n = {g(b)} & 0x3F; if n >= 32 {{ 0 }} else {{ {g(d)} << n }} }}")
        if xo == 536:
            return logical(f"{{ let n = {g(b)} & 0x3F; if n >= 32 {{ 0 }} else {{ {g(d)} >> n }} }}")
        if xo == 792:
            out = (f"{{ let n = {g(b)} & 0x3F; let s = {g(d)} as i32; let v = if n >= 32 {{ c::set_ca(ctx, s < 0); "
                   f"if s < 0 {{ u32::MAX }} else {{ 0 }} }} else {{ c::set_ca(ctx, s < 0 && (s as u32) & ((1u32 << n) - 1) != 0); "
                   f"(s >> n) as u32 }}; g[{a}].set(v);")
            if rc:
                out += " c::update_cr0(ctx, v);"
            return [out + " }"]
        if xo == 824:
            n = b
            out = (f"{{ let s = {g(d)} as i32; c::set_ca(ctx, s < 0 && {'true' if n > 0 else 'false'} && "
                   f"(s as u32) & {hexu((1 << n) - 1)} != 0); let v = (s >> {n}) as u32; g[{a}].set(v);")
            if rc:
                out += " c::update_cr0(ctx, v);"
            return [out + " }"]
        logic = {26: f"{g(d)}.leading_zeros()", 28: f"{g(d)} & {g(b)}", 60: f"{g(d)} & !{g(b)}",
                 124: f"!({g(d)} | {g(b)})", 284: f"!({g(d)} ^ {g(b)})", 316: f"{g(d)} ^ {g(b)}",
                 412: f"{g(d)} | !{g(b)}", 444: f"{g(d)} | {g(b)}", 476: f"!({g(d)} & {g(b)})",
                 922: f"{g(d)} as i16 as u32", 954: f"{g(d)} as i8 as u32"}
        if xo in logic:
            return logical(logic[xo])
        if xo == 83:
            return [f"g[{d}].set(ctx.regs.msr.get());"]
        if xo == 146:
            return [f"ctx.set_msr({g(d)});"]
        if xo == 144:
            crm = (w >> 12) & 0xFF
            out = [f"{{ let v = {g(d)};"]
            for f in range(8):
                if crm & (0x80 >> f):
                    out.append(f"c::set_cr_field(ctx, {f}, v >> {28 - 4 * f});")
            return [" ".join(out) + " }"]
        if xo == 512:
            return [f"{{ let x = ctx.regs.xer.get(); c::set_cr_field(ctx, {(w >> 23) & 7}, x >> 28); ctx.regs.xer.set(x & 0x0FFF_FFFF); }}"]
        if xo in (339, 371):
            n = ((w >> 16) & 31) | (((w >> 11) & 31) << 5)
            if n == 268:
                return [f"g[{d}].set(ctx.read_tbl());"]
            return [f"g[{d}].set(ctx.regs.get_spr({n}));"]
        if xo == 467:
            n = ((w >> 16) & 31) | (((w >> 11) & 31) << 5)
            return [f"ctx.regs.set_spr({n}, {g(d)});"]
        if xo == 595:
            return [f"g[{d}].set(ctx.regs.get_spr({0x10000 + ((w >> 16) & 15):#x}));"]
        if xo == 210:
            return [f"ctx.regs.set_spr({0x10000 + ((w >> 16) & 15):#x}, {g(d)});"]
        if xo == 659:
            return [f"g[{d}].set(ctx.regs.get_spr(0x1_0000 + ({g(b)} >> 28)));"]
        if xo == 242:
            return [f"ctx.regs.set_spr(0x1_0000 + ({g(b)} >> 28), {g(d)});"]
        if xo == 1014:
            return [f"ctx.fill(({ea_x}) & !31, 0, 32);"]
        if xo in (54, 86, 246, 278, 470, 982, 598, 854, 306, 566):
            return []  # cache and sync
        return self.arith(pc, w)

    def arith(self, pc, w):
        d, a, b = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31
        oe = (w >> 10) & 1
        x, y = g(a), g(b)
        xo = (w >> 1) & 0x1FF
        forms = {
            266: (f"c::add3({x}, {y}, 0)", False), 10: (f"c::add3({x}, {y}, 0)", True),
            138: (f"c::add3({x}, {y}, c::ca(ctx))", True), 234: (f"c::add3({x}, u32::MAX, c::ca(ctx))", True),
            202: (f"c::add3({x}, 0, c::ca(ctx))", True), 40: (f"c::add3(!{x}, {y}, 1)", False),
            8: (f"c::add3(!{x}, {y}, 1)", True), 136: (f"c::add3(!{x}, {y}, c::ca(ctx))", True),
            232: (f"c::add3(!{x}, u32::MAX, c::ca(ctx))", True), 200: (f"c::add3(!{x}, 0, c::ca(ctx))", True),
            104: (f"c::add3(!{x}, 0, 1)", False),
        }
        if xo in forms:
            expr, carry = forms[xo]
            out = f"{{ let (v, ca, ov) = {expr}; g[{d}].set(v);"
            if carry:
                out += " c::set_ca(ctx, ca);"
            if oe:
                out += " c::set_ov(ctx, ov);"
            if w & 1:
                out += " c::update_cr0(ctx, v);"
            return [out + " let _ = (ca, ov); }"]
        if xo == 235:
            v = f"{{ let p = i64::from({x} as i32) * i64::from({y} as i32); (p as u32, p != i64::from(p as i32)) }}"
        elif xo == 75:
            v = f"((((i64::from({x} as i32) * i64::from({y} as i32)) >> 32) as u32), false)"
        elif xo == 11:
            v = f"((((u64::from({x}) * u64::from({y})) >> 32) as u32), false)"
        elif xo == 491:
            v = (f"{{ let (sx, sy) = ({x} as i32, {y} as i32); let o = sy == 0 || ({x} == 0x8000_0000 && sy == -1); "
                 f"(if o {{ if sx < 0 {{ u32::MAX }} else {{ 0 }} }} else {{ (sx / sy) as u32 }}, o) }}")
        elif xo == 459:
            v = f"{{ let (x, y) = ({x}, {y}); if y == 0 {{ (0, true) }} else {{ (x / y, false) }} }}"
        else:
            raise AsmUnsupported(f"instruction {w:08x} at {pc:#010x}")
        out = f"{{ let (v, ov) = {v}; g[{d}].set(v);"
        if oe:
            out += " c::set_ov(ctx, ov);"
        if w & 1:
            out += " c::update_cr0(ctx, v);"
        return [out + " let _ = ov; }"]

    def load_store(self, pc, w):
        op = w >> 26
        d, a = (w >> 21) & 31, (w >> 16) & 31
        simm = s16(w)
        update = op in (33, 35, 37, 39, 41, 43, 45, 49, 51, 53, 55)
        ea = add(g(a) if update else ra0(a), simm)
        body = {
            32: f"g[{d}].set(ctx.read_u32(ea));", 34: f"g[{d}].set(u32::from(ctx.read_u8(ea)));",
            40: f"g[{d}].set(u32::from(ctx.read_u16(ea)));", 42: f"g[{d}].set(ctx.read_u16(ea) as i16 as u32);",
            36: f"ctx.write_u32(ea, {g(d)});", 38: f"ctx.write_u8(ea, {g(d)} as u8);",
            44: f"ctx.write_u16(ea, {g(d)} as u16);",
            48: f"c::fill(ctx, {d}, fp::lfs(ctx.read_u32(ea)));",
            50: f"ctx.regs.set_f({d}, f64::from_bits(ctx.read_u64(ea)));",
            52: f"ctx.write_u32(ea, fp::stfs(ctx.regs.f({d})));",
            54: f"ctx.write_u64(ea, ctx.regs.f({d}).to_bits());",
        }
        if op == 46:
            return [f"{{ let ea = {ea}; for (i, reg) in ({d}..32).enumerate() {{ g[reg].set(ctx.read_u32(ea.wrapping_add(4 * i as u32))); }} }}"]
        if op == 47:
            return [f"{{ let ea = {ea}; for (i, reg) in ({d}..32).enumerate() {{ ctx.write_u32(ea.wrapping_add(4 * i as u32), g[reg].get()); }} }}"]
        key = op & ~1 if op not in (46, 47) else op
        stmt = body[key]
        out = f"{{ let ea = {ea}; {stmt}"
        if update:
            out += f" g[{a}].set(ea);"
        return [out + " }"]

    def fp_single(self, pc, w):
        d, a, b, c = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31, (w >> 6) & 31
        fa, fb, fc = f"ctx.regs.f({a})", f"ctx.regs.f({b})", f"ctx.regs.f({c})"
        forms = {18: f"fp::fdivs({fa}, {fb})", 20: f"fp::fsubs({fa}, {fb})", 21: f"fp::fadds({fa}, {fb})",
                 24: f"fp::fres({fb})", 25: f"fp::fmuls({fa}, {fc})", 28: f"fp::fmsubs({fa}, {fc}, {fb})",
                 29: f"fp::fmadds({fa}, {fc}, {fb})", 30: f"fp::fnmsubs({fa}, {fc}, {fb})",
                 31: f"fp::fnmadds({fa}, {fc}, {fb})"}
        xo = (w >> 1) & 0x1F
        if xo not in forms:
            raise AsmUnsupported(f"instruction {w:08x} at {pc:#010x}")
        out = [f"{{ let v = {forms[xo]}; c::fill(ctx, {d}, v); }}"]
        if w & 1:
            out.append("c::update_cr1(ctx);")
        return out

    def fp_double(self, pc, w):
        d, a, b, c = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31, (w >> 6) & 31
        fa, fb, fc = f"ctx.regs.f({a})", f"ctx.regs.f({b})", f"ctx.regs.f({c})"
        forms = {18: f"fp::fdiv({fa}, {fb})", 20: f"fp::fsub({fa}, {fb})", 21: f"fp::fadd({fa}, {fb})",
                 23: f"fp::fsel({fa}, {fc}, {fb})", 25: f"fp::fmul({fa}, {fc})", 26: f"fp::frsqrte({fb})",
                 28: f"fp::fmsub({fa}, {fc}, {fb})", 29: f"fp::fmadd({fa}, {fc}, {fb})",
                 30: f"fp::fnmsub({fa}, {fc}, {fb})", 31: f"fp::fnmadd({fa}, {fc}, {fb})"}
        xo5 = (w >> 1) & 0x1F
        xo = (w >> 1) & 0x3FF
        if xo5 in forms:
            out = [f"{{ let v = {forms[xo5]}; ctx.regs.set_f({d}, v); }}"]
        elif xo in (0, 32):
            out = [f"c::fp_compare(ctx, {(w >> 23) & 7}, {fa}, {fb});"]
        elif xo == 12:
            out = [f"{{ let v = fp::frsp({fb}); c::fill(ctx, {d}, v); }}"]
        elif xo == 14:
            out = [f"{{ let x = {fb}; ctx.regs.set_f({d}, f64::from_bits(fp::fcti_bits(x, fp::fctiw(x)))); }}"]
        elif xo == 15:
            out = [f"{{ let x = {fb}; ctx.regs.set_f({d}, f64::from_bits(fp::fcti_bits(x, fp::fctiwz(x)))); }}"]
        elif xo == 40:
            out = [f"{{ let v = fp::fneg({fb}); ctx.regs.set_f({d}, v); }}"]
        elif xo == 72:
            out = [f"{{ let v = {fb}; ctx.regs.set_f({d}, v); }}"]
        elif xo == 136:
            out = [f"{{ let v = fp::fnabs({fb}); ctx.regs.set_f({d}, v); }}"]
        elif xo == 264:
            out = [f"{{ let v = fp::fabs({fb}); ctx.regs.set_f({d}, v); }}"]
        elif xo == 583:
            out = [f"ctx.regs.set_f({d}, f64::from_bits(0xFFF8_0000_0000_0000 | u64::from(ctx.regs.fpscr.get())));"]
        elif xo == 711:
            fm = (w >> 17) & 0xFF
            mask = 0
            for f in range(8):
                if fm & (0x80 >> f):
                    mask |= 0xF000_0000 >> (4 * f)
            out = [f"{{ let v = {fb}.to_bits() as u32; let fpscr = ctx.regs.fpscr.get(); ctx.regs.fpscr.set((fpscr & !{hexu(mask)}) | (v & {hexu(mask)})); }}"]
        elif xo in (38, 70):
            m = 1 << (31 - ((w >> 21) & 31))
            if xo == 38:
                out = [f"ctx.regs.fpscr.set(ctx.regs.fpscr.get() | {hexu(m)});"]
            else:
                out = [f"ctx.regs.fpscr.set(ctx.regs.fpscr.get() & !{hexu(m)});"]
        elif xo == 134:
            sh = 28 - 4 * ((w >> 23) & 7)
            out = [f"ctx.regs.fpscr.set((ctx.regs.fpscr.get() & !{hexu(0xF << sh)}) | {hexu(((w >> 12) & 0xF) << sh)});"]
        elif xo == 64:
            src = (w >> 18) & 7
            out = [f"c::set_cr_field(ctx, {(w >> 23) & 7}, ctx.regs.fpscr.get() >> {28 - 4 * src});"]
        else:
            raise AsmUnsupported(f"instruction {w:08x} at {pc:#010x}")
        if w & 1:
            out.append("c::update_cr1(ctx);")
        return out

    def paired(self, pc, w):
        d, a, b, c = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31, (w >> 6) & 31
        x6 = (w >> 1) & 0x3F
        if x6 in (6, 7, 38, 39):
            update = x6 >= 38
            ea = f"{g(a) if update else ra0(a)}.wrapping_add({g(b)})"
            wbit, i = "true" if (w >> 10) & 1 else "false", (w >> 7) & 7
            fn = "psq_load" if not (w >> 1) & 1 else "psq_store"
            out = f"{{ let ea = {ea}; c::{fn}(ctx, ea, {d}, {wbit}, {i});"
            if update:
                out += f" g[{a}].set(ea);"
            return [out + " }"]
        pa, pb, pcc = f"f[{a}].get()", f"f[{b}].get()", f"f[{c}].get()"
        forms = {10: f"fp::ps_sum0({pa}, {pcc}, {pb})", 11: f"fp::ps_sum1({pa}, {pcc}, {pb})",
                 12: f"fp::ps_muls0({pa}, {pcc})", 13: f"fp::ps_muls1({pa}, {pcc})",
                 14: f"fp::ps_madds0({pa}, {pcc}, {pb})", 15: f"fp::ps_madds1({pa}, {pcc}, {pb})",
                 18: f"fp::ps_div({pa}, {pb})", 20: f"fp::ps_sub({pa}, {pb})", 21: f"fp::ps_add({pa}, {pb})",
                 23: f"fp::ps_sel({pa}, {pcc}, {pb})", 24: f"fp::ps_res({pb})", 25: f"fp::ps_mul({pa}, {pcc})",
                 26: f"fp::ps_rsqrte({pb})", 28: f"fp::ps_msub({pa}, {pcc}, {pb})", 29: f"fp::ps_madd({pa}, {pcc}, {pb})",
                 30: f"fp::ps_nmsub({pa}, {pcc}, {pb})", 31: f"fp::ps_nmadd({pa}, {pcc}, {pb})"}
        x5 = (w >> 1) & 0x1F
        x10 = (w >> 1) & 0x3FF
        if x5 in forms:
            v = forms[x5]
        elif x10 in (0, 32):
            return [f"c::fp_compare(ctx, {(w >> 23) & 7}, {pa}.ps0, {pb}.ps0);"]
        elif x10 in (64, 96):
            return [f"c::fp_compare(ctx, {(w >> 23) & 7}, {pa}.ps1, {pb}.ps1);"]
        elif x10 == 1014:
            return [f"ctx.fill(({ra0(a)}.wrapping_add({g(b)})) & !31, 0, 32);"]
        else:
            others = {40: f"fp::ps_neg({pb})", 72: pb, 136: f"fp::ps_nabs({pb})", 264: f"fp::ps_abs({pb})",
                      528: f"fp::ps_merge00({pa}, {pb})", 560: f"fp::ps_merge01({pa}, {pb})",
                      592: f"fp::ps_merge10({pa}, {pb})", 624: f"fp::ps_merge11({pa}, {pb})"}
            if x10 not in others:
                raise AsmUnsupported(f"instruction {w:08x} at {pc:#010x}")
            v = others[x10]
        out = [f"{{ let v = {v}; f[{d}].set(v); }}"]
        if w & 1:
            out.append("c::update_cr1(ctx);")
        return out

    def body(self):
        """The function's statements, as a block state machine where it branches."""
        blocks = []
        cur = None
        for pc, w, text in self.words:
            if pc in self.leaders:
                cur = [pc, []]
                blocks.append(cur)
            stmts = self.emit(pc, w)
            cur[1].append(f"// {text}")
            cur[1].extend(stmts)
        straight = len(blocks) == 1
        out = ["let g = &ctx.regs.gpr;", "let f = &ctx.regs.fpr;", "let lr0 = ctx.regs.lr.get();",
               "let _ = (g, f, lr0);"]
        if straight:
            out += blocks[0][1]
            return out
        out += [f"let mut pc: u32 = {hexu(self.start)};", "loop {", "    match pc {"]
        for i, (start, stmts) in enumerate(blocks):
            nxt = blocks[i + 1][0] if i + 1 < len(blocks) else None
            out.append(f"        {hexu(start)} => {{")
            out += ["            " + s for s in stmts]
            if nxt is not None:
                out.append(f"            pc = {hexu(nxt)};")
            else:
                out.append(f"            panic!(\"ran off the end of {self.name}\");")
            out.append("        }")
        out += [f"        _ => unreachable!(\"{self.name}: no block at {{pc:#010x}}\"),", "    }", "}"]
        return out


def jump_targets(listing):
    """The code addresses the listing's jump tables hold, as `.rel <function>, .L_<address>`."""
    return {int(t, 16) for t in re.findall(r"\.rel \w+, \.L_([0-9A-F]{8})", listing)}


def translate(listing, name):
    """Rust statements for the body of `name`'s port, from its instructions in `listing`."""
    words = function_words(listing, name)
    if not words:
        raise AsmUnsupported(f"{name} not in the listing")
    return Emitter(name, words, jump_targets(listing)).body()
