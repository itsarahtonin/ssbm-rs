"""Ports of the game functions Slippi's playback codes patch, transliterated from their machine
code together with the codes: what tools/run registers for a replay whose codes match, instead
of keeping those functions' original code.

    python tools/c2rs/patched.py <decomp root> <replay> [<replay> ...]

A replay contributes the codes playback applies from its list; the bootloader's and playback's
own codes come from crates/slippi/data. Every code set seen gets a port of each function it
patches, written to crates/game/src/patched.rs with a table of the codes each port was made for.

A port reads, at each address a code writes, the word there now and runs what it says: the
original instruction before playback applies its codes, the code's word or its injected code
after. Injected code runs where it was placed in the arena, which the branch at the injection
site gives; only the instructions reachable in it are transliterated, so the data codes keep
among them, such as per-match settings, is read from memory as the original reads it.
"""

import bisect
import hashlib
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "typegen"))
import asm2rs  # noqa: E402
import gecko  # noqa: E402
from asm2rs import AsmUnsupported, hexu  # noqa: E402

OUT = os.path.join(gecko.ROOT, "crates", "game", "src", "patched.rs")
SYNTH = 0xE000_0000  # injected code's states, apart from any game address
SYNTH_STRIDE = 0x0010_0000

# Places in codes' injected instructions that other code calls, such as a process a code gives
# a GObj, by the code's site: found by running replays with ORIGINAL_ENTRIES=1.
ENTRIES = {
    0x8016E748: [0x318, 0x33C, 0x4B0],
    0x8016E74C: [0x6E0, 0x788],
    0x8016E9B4: [0x40],
    0x801A45A0: [0x1C],
    0x801A6348: [0x54],
    0x80375380: [0x18C],
}


def branch_disp(w):
    op = w >> 26
    if op == 18:
        d = w & 0x03FF_FFFC
        return d - 0x0400_0000 if d & 0x0200_0000 else d
    d = w & 0xFFFC
    return d - 0x1_0000 if d & 0x8000 else d


def is_bclr(w):
    return w >> 26 == 19 and (w >> 1) & 0x3FF == 16


def is_bcctr(w):
    return w >> 26 == 19 and (w >> 1) & 0x3FF == 528


def unconditional(w):
    return (w >> 21) & 0x14 == 0x14


# Instruction classes that write rA rather than rD: logical, rotate and shift instructions.
WRITES_A = {20, 21, 23, 24, 25, 26, 27, 28, 29}
WRITES_A_31 = {24, 26, 28, 60, 124, 284, 316, 412, 444, 476, 536, 792, 824, 922, 954}
UPDATES_A = {33, 35, 37, 39, 41, 43, 45, 49, 51, 53, 55, 57, 61}


def written(w):
    """General registers an instruction may write, other than through the cases tracked."""
    op, d, a = w >> 26, (w >> 21) & 31, (w >> 16) & 31
    out = set()
    if op in WRITES_A or (op == 31 and (w >> 1) & 0x3FF in WRITES_A_31):
        out.add(a)
    elif op in (32, 33, 34, 35, 40, 41, 42, 43, 46, 7, 8, 10, 11, 12, 13, 14, 15) or \
            (op == 31 and (w >> 1) & 0x3FF not in (215, 247, 151, 183, 407, 439, 150, 662, 918, 470, 86, 54, 982, 1014, 246, 278, 758)):
        if op not in (10, 11):
            out.add(d)
    if op in UPDATES_A or (op == 31 and (w >> 1) & 0x3FF in (55, 119, 311, 375, 183, 247, 439, 567, 631, 695, 759)):
        out.add(a)
    if op == 46:
        out |= set(range(d, 32))
    return out


class Injection:
    """A C2 code's instructions, placed by playback at an address only its site's branch gives."""

    def __init__(self, k, site, words):
        self.k = k
        self.site = site
        self.words = words
        self.len = 4 * len(words)
        self.last = self.len - 4  # the word the code handler turns into a branch back
        self.synth = SYNTH + k * SYNTH_STRIDE
        self.entries = [0] + ENTRIES.get(site, [])
        self.reach, self.returns = self.reachable()
        self.leaders = self.find_leaders()

    def inside(self, off):
        return 0 <= off < self.len

    def reachable(self):
        """The offsets that run as instructions, found by following where each return goes:
        a `bl`'s next word runs only if something returns to it, since codes also `bl` over data
        to take its address, or to a `blrl` with data after it. Tracks which return address LR,
        CTR, the registers and the stack words hold, as far as `mflr`, `mtlr`, `mr` and loads and
        stores through r1 carry it. Returns (offsets, those `bl` returns to)."""
        seen, returns = set(), set()
        visits = {}
        todo = [(e, ("outer",), None, (), 0, ()) for e in self.entries]
        steps = 0
        while todo and steps < 200_000:
            steps += 1
            o, lr, ctr, regs, sp, slots = state = todo.pop()
            if not self.inside(o) or o == self.last:
                if o == self.last:
                    seen.add(o)
                continue
            if state in visits.setdefault(o, set()) or len(visits[o]) > 16:
                continue
            visits[o].add(state)
            seen.add(o)
            w = self.words[o // 4]
            regs_d = dict(regs)
            slots_d = dict(slots)
            op = w >> 26
            d, a, b = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31
            nxt = []

            def go(to, lr_=None, ctr_=None, regs_=None):
                nxt.append((to, lr if lr_ is None else lr_, ctr if ctr_ is None else ctr_,
                            regs_d if regs_ is None else regs_, sp, slots_d))

            def call_out():
                # A call out of the code returns here with the volatile registers unknown.
                kept = {r: v for r, v in regs_d.items() if r not in range(3, 13) and r != 0}
                go(o + 4, ("ret", o), ctr_=None, regs_=kept)

            def ret_to(value, lr_):
                if value is not None and value[0] == "ret":
                    returns.add(value[1] + 4)
                    go(value[1] + 4, lr_)

            if op == 18 or op == 16:
                link, aa = w & 1, w & 2
                t = o + branch_disp(w)
                cond = op == 16 and (d & 0x14) != 0x14
                if link:
                    if not aa and self.inside(t):
                        go(t, ("ret", o))
                    else:
                        call_out()
                    if cond:
                        go(o + 4, ("ret", o))
                else:
                    if not aa and self.inside(t):
                        go(t)
                    if cond:
                        go(o + 4)
            elif is_bclr(w) or is_bcctr(w):
                link = w & 1
                target = lr if is_bclr(w) else ctr
                cond = (d & 0x14) != 0x14 if is_bclr(w) else (d & 0x10) == 0
                if link:
                    if target is not None and target[0] == "ret":
                        # `blrl` back to a `bl`: its next word, with this one's in LR.
                        ret_to(target, ("ret", o))
                    else:
                        call_out()
                else:
                    ret_to(target, None)
                if cond:
                    go(o + 4, ("ret", o) if link else None)
            else:
                lr_, ctr_ = lr, ctr
                if op == 31 and (w >> 1) & 0x3FF in (339, 467):
                    spr = ((w >> 16) & 31) | (((w >> 11) & 31) << 5)
                    if (w >> 1) & 0x3FF == 339:
                        regs_d.pop(d, None)
                        v = lr if spr == 8 else ctr if spr == 9 else None
                        if v is not None:
                            regs_d[d] = v
                    elif spr == 8:
                        lr_ = regs_d.get(d)
                    elif spr == 9:
                        ctr_ = regs_d.get(d)
                elif op == 31 and (w >> 1) & 0x3FF == 444 and d == b:  # mr
                    regs_d.pop(a, None)
                    if d in regs_d:
                        regs_d[a] = regs_d[d]
                elif op in (36, 37) and a == 1 and sp is not None:  # stw, stwu
                    slots_d[sp + asm2rs.s16(w)] = regs_d.get(d)
                    if op == 37:
                        sp += asm2rs.s16(w)
                elif op == 47 and a == 1 and sp is not None:  # stmw
                    for r in range(d, 32):
                        slots_d[sp + asm2rs.s16(w) + 4 * (r - d)] = regs_d.get(r)
                elif op == 32 and a == 1 and sp is not None:  # lwz
                    regs_d.pop(d, None)
                    v = slots_d.get(sp + asm2rs.s16(w))
                    if v is not None:
                        regs_d[d] = v
                elif op == 46 and a == 1 and sp is not None:  # lmw
                    for r in range(d, 32):
                        regs_d.pop(r, None)
                        v = slots_d.get(sp + asm2rs.s16(w) + 4 * (r - d))
                        if v is not None:
                            regs_d[r] = v
                elif op == 14 and d == 1 and a == 1 and sp is not None:  # addi r1, r1, n
                    sp += asm2rs.s16(w)
                else:
                    for r in written(w):
                        regs_d.pop(r, None)
                        if r == 1:
                            sp = None
                nxt.append((o + 4, lr_, ctr_, regs_d, sp, slots_d))
            for to, lr_, ctr_, regs_, sp_, slots_ in nxt:
                todo.append((to, lr_, ctr_, tuple(sorted(regs_.items())), sp_,
                             tuple(sorted((k, v) for k, v in slots_.items() if v is not None))))
        if todo:
            raise AsmUnsupported(f"code at {self.site:#010x}: too many paths to follow")
        return seen, returns

    def find_leaders(self):
        leaders = set(self.entries) | (self.returns & self.reach)
        for o in self.reach:
            w = self.words[o // 4]
            if o - 4 not in self.reach or asm2rs.Emitter.ends_block(self.words[(o - 4) // 4]):
                leaders.add(o)
            if w >> 26 in (16, 18) and not w & 2:
                t = o + branch_disp(w)
                if t in self.reach:
                    leaders.add(t)
            if asm2rs.Emitter.ends_block(w) and o + 4 in self.reach:
                leaders.add(o + 4)
        return leaders

    def constant_targets(self):
        """Addresses the code loads with `lis` and `ori`/`addi` and jumps to through CTR or LR."""
        vals, out = {}, set()
        for o in sorted(self.reach):
            w = self.words[o // 4]
            op, d, a = w >> 26, (w >> 21) & 31, (w >> 16) & 31
            if op == 15 and a == 0:  # lis rD, hi
                vals[d] = (w & 0xFFFF) << 16
            elif op == 24:  # ori rA, rS, lo: rS in the D field
                if d in vals:
                    vals[a] = vals[d] | (w & 0xFFFF)
                else:
                    vals.pop(a, None)
            elif op == 14 and a != 0 and a in vals:  # addi rD, rA, lo
                vals[d] = (vals[a] + asm2rs.s16(w)) & 0xFFFF_FFFF
            elif op == 31 and (w >> 1) & 0x3FF == 467 and d in vals:  # mtctr, mtlr
                out.add(vals[d])
        return out

    def return_skip(self):
        """How far past its caller's call the code makes its function return, if it does: it
        loads the saved return address from the stack, adds to it and returns there."""
        loaded, moved = set(), {}
        for o in sorted(self.reach):
            w = self.words[o // 4]
            op, d, a = w >> 26, (w >> 21) & 31, (w >> 16) & 31
            if op == 32 and a == 1:
                loaded.add(d)
            elif op == 14 and d == a and a in loaded:
                moved[d] = asm2rs.s16(w)
            elif w & 0xFC1F_FFFF == 0x7C08_03A6 and d in moved:
                return moved[d]
        return None

    def identity(self):
        """(offset, word) of every reachable instruction: what a code set's copy must match."""
        return [(o, self.words[o // 4]) for o in sorted(self.reach) if o != self.last]


class PatchedEmitter(asm2rs.Emitter):
    """A function's instructions, with the words codes write at `patches` (address -> the
    words they may hold) and the code injected at each `Injection`'s site."""

    def __init__(self, name, words, patches, injections, stub=False, entry=0, resumable=None,
                 jump_targets=()):
        super().__init__(name, words)
        # Where the function's jump tables send its `bctr`s.
        for t in jump_targets:
            if self.inside(t):
                self.leaders.add(t)
        self.stub = stub  # a code outside any function, called at its address
        self.entry = entry  # where a stub's call goes in its code
        # `bl` sites whose callee may return past the call, by how far.
        self.resumable = resumable or {}
        for at, skips in self.resumable.items():
            for n in skips:
                if self.inside(at + 4 + n):
                    self.leaders.add(at + 4 + n)
        self.original = {pc: w for pc, w, _ in words}
        self.patches = patches
        self.injections = {i.site: i for i in injections}
        self.inj = injections
        for at in list(patches) + [i.site for i in injections]:
            self.leaders.add(at)
            if self.inside(at + 4):
                self.leaders.add(at + 4)
        for at, variants in patches.items():
            for w in variants:
                t = self.branch_target(at, w)
                if t is not None and self.inside(t):
                    self.leaders.add(t)
        for i in injections:
            for t in i.constant_targets():
                if self.inside(t):
                    self.leaders.add(t)

    # Control transfers the injected code decides at run time.

    def dispatch(self, to):
        """Statements that go to the run-time address `to`: back to our caller, a block of the
        function or its injected code, or out as a tail call."""
        return [f"let to: u32 = {to};",
                "if to == lr0 & !3 { return; }",
                "if let Some(s) = state(to, &pb[..]) { pc = s; continue; }",
                "c::jump_out(ctx, to);",
                "return;"]

    def emit_at(self, pc, w):
        if pc in self.injections:
            i = self.injections[pc]
            # Before playback puts a stub's branch there, nothing calls it.
            orig = [f"panic!(\"{self.name}: called before playback set it up\");"] if self.stub else self.emit(pc, w)
            return [f"let w = ctx.read_u32({hexu(pc)});",
                    f"if w == {hexu(w)} {{"] + ["    " + s for s in orig] + [
                    "} else if w & 0xfc00_0003 == 0x4800_0000 {",
                    f"    // Into the code injected here, where playback placed it.",
                    f"    pb[{i.k}] = {hexu(pc)}.wrapping_add((((w & 0x03ff_fffc) << 6) as i32 >> 6) as u32);",
                    f"    pc = {hexu(i.synth + self.entry)};",
                    "    continue;",
                    "} else {",
                    f"    panic!(\"{self.name}: unknown word {{w:#010x}} at {pc:#010x}\");",
                    "}"]
        if pc in self.patches:
            out = [f"match ctx.read_u32({hexu(pc)}) {{"]
            for v in [w] + [v for v in self.patches[pc] if v != w]:
                out.append(f"    {hexu(v)} => {{")
                out += ["        " + s for s in self.emit(pc, v)]
                out.append("    }")
            out += [f"    w => panic!(\"{self.name}: unknown word {{w:#010x}} at {pc:#010x}\"),", "}"]
            return out
        if pc in self.resumable:
            t = self.branch_target(pc, w)
            return [f"if let Some(t) = c::call_resumable(ctx, {hexu(t)}, {hexu(pc + 4)}) {{",
                    "    // The callee's code returned past this call.",
                    "    pc = t & !3;",
                    "    continue;",
                    "}"]
        return self.emit(pc, w)

    def op19(self, pc, w):
        if (w >> 1) & 0x3FF == 528 and not w & 1:
            # `bctr` through a jump table, into the function, or out of it.
            cond = self.condition(((w >> 21) & 31) | 0x04, (w >> 16) & 31)
            body = ["let to = ctx.regs.ctr.get() & !3;"] + self.dispatch("to")
            return body if cond == "true" else [f"if {cond} {{"] + ["    " + x for x in body] + ["}"]
        return super().op19(pc, w)

    def emit_injected(self, i, o, w):
        """Rust for the injected instruction `w` at offset `o` of injection `i`."""
        rt = lambda off: f"pb[{i.k}].wrapping_add({hexu(off)})"  # noqa: E731
        if o == i.last:
            return [f"pc = {hexu(i.site + 4)};", "continue;"]
        op = w >> 26
        link = bool(w & 1)
        if op in (16, 18):
            aa = bool(w & 2)
            disp = branch_disp(w)
            t = o + disp
            if aa:
                target = hexu(disp & 0xFFFF_FFFF)
            else:
                target = f"{rt(o)}.wrapping_add({hexu(disp & 0xFFFF_FFFF)})"
            if link:
                if not aa and i.inside(t):
                    body = [f"pc = {hexu(i.synth + t)};", "continue;"]
                else:
                    body = [f"c::call(ctx, {target}, {rt(o + 4)});"]
            elif not aa and i.inside(t):
                body = [f"pc = {hexu(i.synth + t)};", "continue;"]
            else:
                body = self.dispatch(target)
            out = [f"ctx.regs.lr.set({rt(o + 4)});"] if link else []
            if op == 18:
                return out + body
            cond = self.condition((w >> 21) & 31, (w >> 16) & 31)
            if cond == "true":
                return out + body
            return out + [f"if {cond} {{"] + ["    " + s for s in body] + ["}"]
        if is_bclr(w) or is_bcctr(w):
            bo, bi = (w >> 21) & 31, (w >> 16) & 31
            reg = "lr" if is_bclr(w) else "ctr"
            cond = self.condition(bo, bi) if reg == "lr" else self.condition(bo | 0x04, bi)
            if link and reg == "ctr":
                body = [f"c::call(ctx, ctx.regs.ctr.get() & !3, {rt(o + 4)});"]
            elif link:
                # Back into this code, where a `bl` put the data after this in LR, or a call.
                body = ["let to = ctx.regs.lr.get() & !3;",
                        f"ctx.regs.lr.set({rt(o + 4)});",
                        "if let Some(s) = state(to, &pb[..]) { pc = s; continue; }",
                        f"c::call(ctx, to, {rt(o + 4)});"]
            else:
                body = [f"let to = ctx.regs.{reg}.get() & !3;"] + self.dispatch("to")
            if cond == "true":
                return body
            out = [f"if {cond} {{"] + ["    " + s for s in body] + ["}"]
            if link:
                out.append(f"else {{ ctx.regs.lr.set({rt(o + 4)}); }}")
            return out
        return self.emit(i.synth + o, w)

    def state_fn(self):
        """A closure from a run-time address to the block that starts there."""
        lines = ["let state = |to: u32, pb: &[u32]| -> Option<u32> {"]
        lines.append(f"    if ({hexu(self.start)}..{hexu(self.end)}).contains(&to) {{ return Some(to); }}")
        for i in self.inj:
            lines.append(f"    if pb[{i.k}] != 0 && to.wrapping_sub(pb[{i.k}]) < {hexu(i.len)} {{"
                         f" return Some({hexu(i.synth)} + (to - pb[{i.k}])); }}")
        lines += ["    let _ = pb;", "    None", "};"]
        return lines

    def body(self):
        blocks = []
        cur = None
        for pc, w, text in self.words:
            if pc in self.leaders:
                cur = [pc, [], pc]
                blocks.append(cur)
            cur[1].append(f"// {text}")
            cur[1].extend(self.emit_at(pc, w))
            cur[2] = pc + 4
        for i in self.inj:
            cur = None
            for o in sorted(i.reach):
                if o in i.leaders or cur is None:
                    cur = [i.synth + o, [], None]
                    blocks.append(cur)
                w = i.words[o // 4]
                cur[1].append(f"// injected at {i.site:#010x}, +{o:#x}: {w:08x}")
                cur[1].extend(self.emit_injected(i, o, w))
                cur[2] = i.synth + o + 4 if o + 4 in i.reach else None
        out = ["let g = &ctx.regs.gpr;", "let f = &ctx.regs.fpr;", "let lr0 = ctx.regs.lr.get();",
               f"let mut pb = [0_u32; {max(1, len(self.inj))}];", "let _ = (g, f, lr0);"]
        out += self.state_fn()
        out += [f"let mut pc: u32 = {hexu(self.start)};", "loop {", "    match pc {"]
        for start, stmts, nxt in blocks:
            out.append(f"        {hexu(start)} => {{")
            out += ["            " + s for s in stmts]
            if nxt is not None and (nxt < self.end or nxt >= SYNTH):
                out.append(f"            pc = {hexu(nxt)};")
            else:
                out.append(f"            panic!(\"{self.name}: ran off a block at {{pc:#010x}}\");")
            out.append("        }")
        out += [f"        _ => unreachable!(\"{self.name}: no block at {{pc:#010x}}\"),", "    }", "}"]
        return out


# Functions and their machine code.

def load_functions(root):
    """[(start, end, name)] of the game's functions, sorted."""
    sys.path.insert(0, os.path.join(gecko.ROOT, "tools", "typegen"))
    import extract
    out = []
    for s in extract.load_symbols(os.path.join(root, "config", "GALE01", "symbols.txt")):
        if s["type"] == "function" and s["size"]:
            out.append((s["addr"], s["addr"] + s["size"], s["name"]))
    return sorted(out)


def listing_index(root):
    """Function name -> the listing file with its machine code."""
    index = {}
    for dirpath, _, files in os.walk(os.path.join(root, "build", "GALE01", "asm")):
        for f in files:
            if f.endswith(".s"):
                path = os.path.join(dirpath, f)
                for m in re.finditer(r"^\.fn (\w+),", open(path, encoding="utf-8").read(), re.M):
                    index[m.group(1)] = path
    return index


class Funcs:
    def __init__(self, root):
        self.list = load_functions(root)
        self.starts = [f[0] for f in self.list]

    def at(self, addr):
        i = bisect.bisect_right(self.starts, addr) - 1
        if i >= 0 and self.list[i][0] <= addr < self.list[i][1]:
            return self.list[i]
        return None


# Code sets.

def site_spec(code):
    """What a port must be made for at one code's site: a word, or injected instructions."""
    if code.kind == 0x04:
        return ("word", code.addr, code.first)
    inj = Injection(0, code.addr, code.words)
    return ("inject", code.addr, len(code.words), tuple(inj.identity()))


def port_name(fname, specs):
    h = hashlib.sha1(repr(specs).encode()).hexdigest()[:8]
    return f"{fname}__{h}"


def calls_index(index):
    """Function start -> [(caller, address of the `bl`)], from every listing."""
    out = {}
    for path in sorted(set(index.values())):
        listing = open(path, encoding="utf-8").read()
        for m in re.finditer(r"^\.fn (\w+),", listing, re.M):
            for pc, w, _ in asm2rs.function_words(listing, m.group(1)) or []:
                if w & 0xFC00_0003 == 0x4800_0001:
                    out.setdefault((pc + branch_disp(w)) & 0xFFFF_FFFF, []).append((m.group(1), pc))
    return out


def inject_rust(spec):
    return (f"Code::Inject {{ len: {spec[2]}, words: &["
            + ", ".join(f"({o:#x}, {w:#x})" for o, w in spec[3]) + "] }")


def main():
    root = sys.argv[1]
    replays = sys.argv[2:]
    funcs = Funcs(root)
    index = listing_index(root)
    callers = calls_index(index)
    deny = gecko.denylist()
    base = gecko.bootloader() + gecko.playback_set()
    code_sets = {}
    specs = {}
    for path in replays:
        codes = base + gecko.playback_replay_codes(path, deny)
        # Codes whose instructions are the same are the same code, whatever data they carry.
        key = []
        for c in codes:
            if c.kind in (0x04, 0xC2):
                ident = (c.kind, c.addr, c.first, tuple(c.words))
                if ident not in specs:
                    specs[ident] = site_spec(c)
                key.append(specs[ident])
            else:
                key.append((c.kind, c.addr, c.first, tuple(c.words)))
        code_sets.setdefault(tuple(key), []).append((path, codes))
    print(f"{len(replays)} replays, {len(code_sets)} code sets", file=sys.stderr)

    def spec_of(c):
        return specs[(c.kind, c.addr, c.first, tuple(c.words))]

    ports = {}  # port name -> (function, specs, codes, resumable)
    entries = {}  # port name -> (site, offset, spec, code)
    others = set()
    for members in code_sets.values():
        codes = members[0][1]
        by_func, resume = {}, {}
        for c in codes:
            if c.kind not in (0x04, 0xC2):
                others.add((c.kind, c.addr))
                continue
            f = funcs.at(c.addr)
            if f is None:
                if c.kind != 0xC2:
                    continue  # data
                # A stub the code's instructions stand behind, called as a function.
                f = (c.addr, c.addr + 4, f"stub_{c.addr:08X}")
            by_func.setdefault(f, []).append(c)
            if c.kind == 0xC2:
                inj = Injection(0, c.addr, c.words)
                for off in ENTRIES.get(c.addr, []):
                    name = f"code_{c.addr:08X}_{off:X}__" + hashlib.sha1(repr(spec_of(c)).encode()).hexdigest()[:8]
                    entries.setdefault(name, (c.addr, off, spec_of(c), c))
                n = inj.return_skip()
                if n is not None and funcs.at(c.addr) is not None:
                    # Callers of the code's function resume past their call.
                    for caller, at in callers.get(funcs.at(c.addr)[0], []):
                        cf = funcs.at(at)
                        if cf is not None:
                            resume.setdefault(cf, {}).setdefault(at, set()).add(n)
        for f in set(by_func) | set(resume):
            cs = by_func.get(f, [])
            fspecs = tuple(sorted(spec_of(c) for c in cs))
            res = {at: tuple(sorted(ns)) for at, ns in sorted(resume.get(f, {}).items())}
            name = port_name(f[2], (fspecs, tuple(res.items())))
            ports.setdefault(name, (f, fspecs, cs, res))
    if others:
        print("codes of other types: " + ", ".join(f"{k:02X} {a:#010x}" for k, a in sorted(others)),
              file=sys.stderr)

    out = ["// Generated by tools/c2rs/patched.py from Slippi's playback codes (GPL-3.0) and the game's",
           "// machine code; do not edit.",
           "",
           "//! Ports of the functions Slippi's playback codes patch, transliterated from their machine",
           "//! code together with the codes, and the codes each port was made for.",
           "",
           "#![allow(unused_labels, unreachable_code, unused_variables, unused_mut, unused_assignments, unused_parens,",
           "    clippy::all)]",
           "",
           "use gekko_fp as fp;",
           "use ssbm_rt::cpu as c;",
           "use ssbm_rt::*;",
           ""]
    table, entry_table, failed = [], [], []

    def emit_fn(name, doc, body):
        out.append(f"/// {doc}")
        out.append(f"pub fn {name}(ctx: &Ctx) {{")
        out.extend("    " + x for x in body)
        out.extend(["}", ""])

    for name in sorted(ports):
        (start, end, fname), fspecs, cs, res = ports[name]
        stub = fname.startswith("stub_")
        jump_targets = ()
        if stub:
            words = [(start, 0, "stub")]
        elif fname in index:
            listing = open(index[fname], encoding="utf-8").read()
            words = asm2rs.function_words(listing, fname)
            jump_targets = asm2rs.jump_targets(listing)
        else:
            words = None
        if not words:
            failed.append((fname, "not in the listing"))
            continue
        patches, injections = {}, []
        for c in cs:
            if c.kind == 0x04:
                patches.setdefault(c.addr, []).append(c.first)
            else:
                injections.append(Injection(len(injections), c.addr, c.words))
        try:
            body = PatchedEmitter(fname, words, patches, injections, stub,
                                  resumable={at: list(ns) for at, ns in res.items()},
                                  jump_targets=jump_targets).body()
        except AsmUnsupported as e:
            failed.append((fname, str(e)))
            continue
        what = []
        if fspecs:
            what.append("the codes at " + ", ".join(f"{sp[1]:#010x}" for sp in fspecs))
        if res:
            what.append("calls that a code may return past, at " + ", ".join(f"{at:#010x}" for at in res))
        emit_fn(name, f"{fname} with {' and '.join(what)}.", body)
        sites = []
        for sp in fspecs:
            if sp[0] == "word":
                sites.append(f"Site {{ addr: {sp[1]:#x}, code: Code::Word({sp[2]:#x}) }}")
            else:
                sites.append(f"Site {{ addr: {sp[1]:#x}, code: {inject_rust(sp)} }}")
        table.append(f"    Patched {{ function: {start:#x}, name: \"{fname}\", sites: &[{', '.join(sites)}], port: {name} }},")

    for name in sorted(entries):
        site, off, sp, c = entries[name]
        inj = Injection(0, site, c.words)
        try:
            body = PatchedEmitter(name, [(site, 0, "code site")], {}, [inj], stub=True, entry=off).body()
        except AsmUnsupported as e:
            failed.append((name, str(e)))
            continue
        emit_fn(name, f"The code at {site:#010x}, called {off:#x} into its instructions.", body)
        entry_table.append(f"    Entry {{ site: {site:#x}, offset: {off:#x}, code: {inject_rust(sp)}, port: {name} }},")

    out += ["/// Where a code writes and what: a word, or injected instructions (offset, word) among its",
            "/// `len` words.",
            "pub struct Site {",
            "    pub addr: u32,",
            "    pub code: Code,",
            "}",
            "",
            "pub enum Code {",
            "    Word(u32),",
            "    Inject { len: u32, words: &'static [(u32, u32)] },",
            "}",
            "",
            "/// A port of `function` for the codes at `sites`.",
            "pub struct Patched {",
            "    pub function: u32,",
            "    pub name: &'static str,",
            "    pub sites: &'static [Site],",
            "    pub port: fn(&Ctx),",
            "}",
            "",
            "/// A port of the code at `site` entered `offset` into its injected instructions, where",
            "/// other code calls it.",
            "pub struct Entry {",
            "    pub site: u32,",
            "    pub offset: u32,",
            "    pub code: Code,",
            "    pub port: fn(&Ctx),",
            "}",
            "",
            "pub static PATCHED: &[Patched] = &["] + table + ["];", "",
            "pub static ENTRIES: &[Entry] = &["] + entry_table + ["];", ""]
    with open(OUT, "w", encoding="utf-8", newline="\n") as w:
        w.write("\n".join(out))
    subprocess.run(["rustfmt", "--edition", "2024", OUT], check=False)
    print(f"{len(table)} ports of {len({p[0] for p in ports.values()})} functions, {len(entry_table)} entries",
          file=sys.stderr)
    for fname, why in failed:
        print(f"  left to the original: {fname}: {why}", file=sys.stderr)


if __name__ == "__main__":
    main()
