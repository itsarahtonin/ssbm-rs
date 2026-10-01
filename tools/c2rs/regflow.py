"""Which functions read registers their callers never pass them, from the decomp's listings.

A function whose code reads a register before writing it, other than its parameters and the
stack and data pointers, takes whatever its caller left there: a local the C reads before
setting, or a value the function returns on a path that never sets it. Only a port of the
machine code reproduces that, and only if its callers hold in those registers what the
original callers do, so callers that leave them as they found them pass the dependence on to
their own callers. c2rs ports those callers from machine code too (`callers_to_port`).

    python tools/c2rs/regflow.py <decomp root> local/typegen/types.json

lists every function whose code reads registers that way, with the registers, most of them on
paths their C never takes.

The other way round, code ported from machine code may read a register that a call leaves
and the callee's C doesn't return, such as r3 after a call to a `void` function, to return it
as a value its own C never sets. Only a port of the callee's machine code leaves that
register as the original does (`callees_to_port`).
"""

import glob
import json
import os
import re
import sys

INSN = re.compile(r"^/\* ([0-9A-F]{8}) [0-9A-F]{8}  (?:[0-9A-F]{2} ){4}\*/\t(\S+)\s*(.*)$")
REG = re.compile(r"^([rf])(\d+)$")
MEM = re.compile(r"^[^()]*\(r(\d+)\)$")
LABEL = re.compile(r"\.L_([0-9A-F]{8})")
JUMP_TABLE_ENTRY = re.compile(r"^\s+\.rel (\S+), \.L_([0-9A-F]{8})$")

# Registers no function receives from its caller as a value: the stack pointer and the small
# data bases.
FIXED = {("r", 1), ("r", 2), ("r", 13)}
# What a call leaves undefined: the volatile registers.
VOLATILE = {("r", 0)} | {("r", i) for i in range(3, 13)} | {("f", i) for i in range(0, 14)}
CALLEE_SAVED = {("r", i) for i in range(14, 32)} | {("f", i) for i in range(14, 32)}
ALL_ARGS = {("r", i) for i in range(3, 11)} | {("f", i) for i in range(1, 9)}

STORES = {"stb", "stbu", "stbx", "stbux", "sth", "sthu", "sthx", "sthux", "sthbrx", "stw", "stwu",
          "stwx", "stwux", "stwbrx", "stwcx", "stmw", "stfs", "stfsu", "stfsx", "stfsux", "stfd",
          "stfdu", "stfdx", "stfdux", "stfiwx", "psq_st", "psq_stu", "psq_stx", "psq_stux",
          "stswi", "stswx"}
READS_ONLY = {"cmp", "cmpi", "cmpl", "cmpli", "cmpw", "cmpwi", "cmplw", "cmplwi", "fcmpu", "fcmpo",
              "ps_cmpu0", "ps_cmpo0", "ps_cmpu1", "ps_cmpo1", "mtctr", "mtlr", "mtspr", "mtmsr",
              "mtsr", "mtsrin", "mtfsf", "mtcrf", "mtxer", "dcbf", "dcbi", "dcbst", "dcbt",
              "dcbtst", "dcbz", "dcbz_l", "icbi", "tw", "twi", "tlbie"}
NO_REGS = {"sync", "isync", "eieio", "rfi", "sc", "mtfsb0", "mtfsb1", "mcrf", "mcrfs", "mcrxr"}
# Instructions that also read the register they write.
READ_MODIFY_WRITE = {"rlwimi", "inslwi", "insrwi"}
# Code that reads registers only to keep them for later, as a prologue does: context switches,
# setjmp, the exception handlers and the debugger.
KEEPS_REGISTERS = {"OSSaveContext", "__OSSaveFPUContext", "__setjmp", "OSDefaultExceptionHandler",
                   "ExternalInterruptHandler", "DecrementerExceptionHandler", "InitMetroTRK",
                   "TRKInterruptHandler", "TRKSwapAndGo"}
# Functions that never return: what follows a call to one never runs. gm_801A4510 is the game's
# main loop.
NORETURN = {"__assert", "OSPanic", "HSD_Panic", "OSResetSystem", "PPCHalt", "gm_801A4510"}
# Where a return reads, for `flows`.
RETURN = ("return", 0)
# Callers a seed's dependence may pass through before the seed is left be.
FOLLOWED = 10
# Functions whose unset reads stay with them: their callers would pass the dependence up
# through most of the game.
IGNORED = {
    # Reads f27 and f28 unset on a path no lockstep check of HSD_JObjSetupMatrixSub has taken.
    "resolveIKJoint1",
}


def operands(text):
    """Splits an operand list at its commas."""
    text = text.split("#", 1)[0].strip()
    return [o.strip() for o in text.split(",")] if text else []


def reg(o):
    m = REG.match(o)
    return (m.group(1), int(m.group(2))) if m else None


def def_use(m, ops):
    """(registers written, registers read) of one instruction other than a branch."""
    regs = [reg(o) for o in ops]
    # As a base, r0 stands for zero; a small data access names it, which the linker makes r13
    # or r2.
    bases = {("r", int(mm.group(1))) for o in ops for mm in [MEM.match(o)]
             if mm and mm.group(1) != "0"}
    # So does an indexed access's or a cache operation's first address register.
    cache = m.startswith(("dcb", "icb"))
    at = 0 if cache else 1
    if (cache or (m.endswith("x") and m.startswith(("l", "st", "psq")))) and len(regs) > at \
            and regs[at] == ("r", 0):
        regs[at] = None
    updates = m.endswith("u") or m.endswith("ux")
    if m == "stmw":
        return set(), {("r", i) for i in range(regs[0][1], 32)} | bases
    if m == "lmw":
        return {("r", i) for i in range(regs[0][1], 32)}, bases
    if m in STORES:
        return (bases if updates else set()), {r for r in regs if r} | bases
    if m in READS_ONLY:
        return set(), {r for r in regs if r} | bases
    if m.startswith("mf"):
        return {r for r in regs[:1] if r}, set()
    if m in NO_REGS or m.startswith("cr"):
        return set(), set()
    if not regs or regs[0] is None:
        return set(), {r for r in regs if r} | bases
    written = {regs[0]}
    read = {r for r in regs[1:] if r} | bases
    if m in READ_MODIFY_WRITE:
        read.add(regs[0])
    if m.startswith("l") and updates:
        written |= bases
    return written, read


def functions(root):
    """Yields (name, [(address, mnemonic, operands)], {label addresses}, {jump table targets})
    for every function in the listings."""
    for path in sorted(glob.glob(os.path.join(root, "build", "GALE01", "asm", "**", "*.s"),
                              recursive=True)):
        name, insns, labels = None, [], set()
        lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
        tables = {}
        for line in lines:
            m = JUMP_TABLE_ENTRY.match(line)
            if m:
                tables.setdefault(m.group(1), set()).add(int(m.group(2), 16))
        for line in lines:
            if line.startswith(".fn "):
                name, insns, labels = line[4:].split(",")[0].strip(), [], set()
            elif line.startswith(".endfn"):
                if name and insns:
                    yield name, insns, labels, tables.get(name, set())
                name = None
            elif name:
                if line.startswith(".L_"):
                    labels.add(int(line[3:11], 16))
                else:
                    m = INSN.match(line.rstrip("\n"))
                    if m:
                        insns.append((int(m.group(1), 16), m.group(2).rstrip("+-"),
                                      operands(m.group(3))))


def arg_regs(ft):
    """The registers a function of C type `ft` takes its parameters in, and those it returns
    its value in, or (None, None) without a type."""
    if ft is None:
        return None, None
    if ft.get("variadic"):
        args = set(ALL_ARGS)
    else:
        args, g, f = set(), 3, 1
        for p in ft.get("params") or []:
            k = p.get("k")
            if k == "float":
                args.add(("f", f))
                f += 1
            elif k in ("int", "enum") and p.get("size") == 8:
                g += g % 2 == 0  # a pair starts at an odd register
                args |= {("r", g), ("r", g + 1)}
                g += 2
            else:
                args.add(("r", g))
                g += 1
        # Past r10 and f8, parameters go on the stack.
        args &= ALL_ARGS
    ret = ft.get("ret") or {"k": "void"}
    if ret.get("k") == "void":
        rets = set()
    elif ret.get("k") == "float":
        rets = {("f", 1)}
    elif ret.get("k") in ("int", "enum") and ret.get("size") == 8:
        rets = {("r", 3), ("r", 4)}
    else:
        rets = {("r", 3)}
    return args, rets


class Program:
    def __init__(self, root, types_path):
        types = json.load(open(types_path, encoding="utf-8"))
        self.types, self.units = {}, {}
        for f in types["functions"]:
            if f.get("addr") is not None:
                self.types[f.get("symbol") or f["name"]] = f["type"]
                self.units[f.get("symbol") or f["name"]] = f.get("tu")
        self.funcs = {name: (insns, labels, cases) for name, insns, labels, cases in functions(root)}
        self.callers = {}
        for name, (insns, _, _) in self.funcs.items():
            for _, mnem, ops in insns:
                if mnem in ("bl", "b") and ops and ops[0] in self.funcs and ops[0] != name:
                    self.callers.setdefault(ops[0], set()).add(name)

    def params(self, name):
        args, rets = arg_regs(self.types.get(name))
        if args is None:
            # Without a prototype, a call may pass anything.
            return set(ALL_ARGS), {("r", 3), ("r", 4), ("f", 1)}
        return args, rets

    def passed(self, name):
        """The registers a call to `name` surely passes it: a variadic function's, or one
        without a prototype, may be fewer than it can take."""
        ft = self.types.get(name)
        if ft is None or ft.get("variadic"):
            return {("r", 3)} if ft is None or ft.get("params") else set()
        return self.params(name)[0]

    def reads(self, name, extra):
        """The registers `name` reads before writing them on some path from its entry, beyond
        its parameters, given what each callee reads beyond its own (`extra`)."""
        if name in KEEPS_REGISTERS:
            return set()
        live, _ = self.liveness(name, extra)
        return live - self.params(name)[0] - FIXED

    def blocks(self, name, extra, rets):
        """`name`'s code as blocks: (instructions' effects, successors), each effect (callee or
        None, registers written, registers read), with what each callee reads beyond its
        parameters (`extra`) and what a return reads (`rets`)."""
        insns, labels, cases = self.funcs[name]
        addrs = [a for a, _, _ in insns]
        index = {a: i for i, a in enumerate(addrs)}
        starts = {addrs[0]} | (labels & set(addrs))
        for i, (_, mnem, _) in enumerate(insns):
            if mnem.startswith("b") and i + 1 < len(insns):
                starts.add(addrs[i + 1])
        order = sorted(starts)
        bounds = [(s, order[j + 1] if j + 1 < len(order) else addrs[-1] + 4)
                  for j, s in enumerate(order)]
        block_of = {s: j for j, (s, _) in enumerate(bounds)}
        out = []
        for s, e in bounds:
            effects, nxt, falls = [], [], True
            for a in range(s, e, 4):
                _, m, ops = insns[index[a]]
                if m in ("bl", "b") and ops and ops[0] in self.funcs:
                    callee = ops[0]
                    if callee.startswith(("_savegpr", "_savefpr")):
                        continue  # saves the caller's registers
                    if callee.startswith(("_restgpr", "_restfpr")):
                        n = int(re.sub(r"\D", "", callee) or 14)
                        effects.append((None, {("r" if "gpr" in callee else "f", i)
                                               for i in range(n, 32)}, set()))
                        continue
                    reads = self.passed(callee) | extra.get(callee, set())
                    if m == "b":
                        # A tail call returns for this function.
                        effects.append((callee, set(), reads | rets))
                        falls = False
                        break
                    effects.append((callee, set(VOLATILE), reads))
                    continue
                if m == "bl":
                    effects.append((None, set(VOLATILE), set()))  # code without a listing
                    continue
                if m.startswith("b"):
                    if m.endswith(("lrl", "ctrl")):
                        # An indirect call, whose arguments are set before it.
                        effects.append((None, set(VOLATILE), set()))
                        continue
                    if m.endswith("lr"):
                        effects.append((None, set(), set(rets)))  # a return, perhaps conditional
                        falls = m != "blr"
                        if falls:
                            continue
                        break
                    if m.endswith("ctr"):
                        # A jump table: the listing's tables give its cases.
                        nxt.extend(t for t in (cases or labels) if t in index)
                        falls = m != "bctr"
                        continue
                    for o in ops:
                        t = LABEL.match(o)
                        if t and int(t.group(1), 16) in index:
                            nxt.append(int(t.group(1), 16))
                    if m == "b":
                        falls = False
                    continue
                w, r = def_use(m, ops)
                if m in STORES and any(MEM.match(o) and MEM.match(o).group(1) == "1" for o in ops):
                    # Storing a callee-saved register to the frame saves the caller's value.
                    r = r - CALLEE_SAVED
                effects.append((None, w, r))
            if falls and e in index:
                nxt.append(e)
            out.append((effects, [block_of[t] for t in nxt]))
        return out

    def liveness(self, name, extra, rets=None):
        """(registers live at `name`'s entry, [(callee, registers live once it returns)]),
        given what each callee reads beyond its parameters (`extra`) and what `name`'s callers
        read once it returns (`rets`, by default what its C returns)."""
        if rets is None:
            rets = self.params(name)[1]
        blocks = self.blocks(name, extra, rets)

        def back(effects, live, calls=None):
            for callee, w, r in reversed(effects):
                if calls is not None and callee is not None:
                    # A tail call's callee returns to this function's callers.
                    calls.append((callee, set(live) if w else set(rets)))
                live = (live - w) | r
            return live

        live = [set() for _ in blocks]
        changed = True
        while changed:
            changed = False
            for j in reversed(range(len(blocks))):
                effects, succ = blocks[j]
                out = set().union(*(live[t] for t in succ)) if succ else set()
                new = back(effects, out)
                if new != live[j]:
                    live[j] = new
                    changed = True
        calls = []
        for effects, succ in blocks:
            back(effects, set().union(*(live[t] for t in succ)) if succ else set(), calls)
        return live[0] if live else set(), calls

    def leftovers(self, name, returned=frozenset()):
        """[(callee, registers)]: the calls in `name` after which its code reads registers the
        callee's C doesn't return, which the callee's code left."""
        if name in KEEPS_REGISTERS or name not in self.funcs:
            return []
        # A function without a known type, such as an inline one's copy, returns nothing more.
        rets = (self.params(name)[1] if name in self.types else set()) | returned
        out = []
        for callee, live in self.liveness(name, {}, rets)[1]:
            left = (live & VOLATILE) - self.params(callee)[1]
            if left and callee not in NORETURN:
                out.append((callee, left))
        return out

    def flows(self, name, regs):
        """(the registers of `regs` that `name`'s own code sets on some way to a return that
        keeps them there, [(callee, registers)] whose values reach a return as the callee left
        them)."""
        blocks = self.blocks(name, {}, {RETURN})

        def back(effects, live, own=None, calls=None):
            for callee, w, r in reversed(effects):
                if callee is not None and RETURN in r:
                    # A tail call: what this function returns, the callee leaves.
                    if calls is not None and regs & VOLATILE:
                        calls.append((callee, regs & VOLATILE))
                    live = set()
                elif callee is not None:
                    if calls is not None and live & VOLATILE:
                        calls.append((callee, live & VOLATILE))
                    live = live - w
                elif RETURN in r:
                    live = live | regs
                else:
                    if own is not None and r != VOLATILE:
                        own |= live & w
                    live = live - w
            return live

        live = [set() for _ in blocks]
        changed = True
        while changed:
            changed = False
            for j in reversed(range(len(blocks))):
                effects, succ = blocks[j]
                new = back(effects, set().union(*(live[t] for t in succ)) if succ else set())
                if new != live[j]:
                    live[j] = new
                    changed = True
        own, calls = set(), []
        for effects, succ in blocks:
            back(effects, set().union(*(live[t] for t in succ)) if succ else set(), own, calls)
        return own, calls

    def callees_to_port(self, ported):
        """{callee: (reader, registers)} for the functions that must be ported from machine
        code because code ported that way (`ported`, the reader) reads registers they set that
        their C doesn't return: what their own instructions leave there, which a port of their
        C never writes. Such a register may come down through callees that leave it as a callee
        of theirs did."""
        port, seen = {}, {}
        work = [(callee, regs, f) for f in sorted(set(ported)) for callee, regs in self.leftovers(f)]
        while work:
            callee, regs, reader = work.pop()
            if callee in ported or callee in NORETURN or callee not in self.funcs:
                continue
            # What its C returns, a port of it returns there too.
            regs = regs - self.params(callee)[1]
            known = seen.get(callee, set())
            if regs <= known:
                continue
            seen[callee] = known | regs
            own, calls = self.flows(callee, seen[callee])
            if own:
                first, before = port.get(callee, (reader, set()))
                port[callee] = (first, before | own)
            work.extend((c, r, reader) for c, r in calls)
        return port

    def callers_to_port(self, seeds):
        """({caller: callee}, [seeds left be]) for the callers that must be ported from
        machine code with `seeds`, the functions ported that way because their C reads values
        it never sets: those that call one reading registers its caller never passes it, and on
        up while a caller leaves such registers as it found them. A seed whose dependence
        passes through more than `FOLLOWED` callers is left be, as `IGNORED` ones are: it
        reaches that far only through paths that never set a register, which the game takes
        rarely if ever."""
        port, left = {}, []
        for seed in sorted(set(seeds)):
            if seed not in self.funcs or seed in IGNORED:
                continue
            found, passing = self.follow([seed])
            if passing > FOLLOWED:
                left.append(seed)
                continue
            for c, f in found.items():
                port.setdefault(c, f)
        return {c: f for c, f in port.items() if c not in seeds}, left

    def follow(self, seeds):
        """({caller: callee}, how many pass it on) for the callers the dependence of `seeds`
        carries to."""
        extra = {f: r for f in seeds for r in [self.reads(f, {})] if r}
        own, port = {}, {}
        passing = set()
        work = list(extra)
        while work:
            f = work.pop()
            for c in sorted(self.callers.get(f, ())):
                port.setdefault(c, f)
                if c not in own:
                    own[c] = self.reads(c, {})
                # Only what passes through from the callees counts: a caller's own unset reads,
                # on paths its C never takes, don't make its callers depend on anything.
                r = self.reads(c, extra) - own[c]
                if r and r != extra.get(c):
                    extra[c] = r
                    passing.add(c)
                    work.append(c)
        return port, len(passing)


def fmt(regs):
    return ", ".join(f"{k}{i}" for k, i in sorted(regs))


def main():
    root, types_path = sys.argv[1:3]
    prog = Program(root, types_path)
    found = {n: r for n in prog.funcs for r in [prog.reads(n, {})] if r}
    for name in sorted(found):
        print(f"{name}: {fmt(found[name])}")
    print(len(found), "functions read registers their callers never pass them", file=sys.stderr)


if __name__ == "__main__":
    main()
