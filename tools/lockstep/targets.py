"""Lists, for ssbm-run's LOCKSTEP_TARGETS, what decides the branches to the code lockstep has
not reached in the functions below the bar.

    python tools/lockstep/targets.py <decomp root> REPORT_CSV OUT --coverage FILE... [--bar 0.9]

For each block not yet verified that a verified block branches to, the compare deciding that
branch is traced back to what set its register:

    call   a callee's result: `F call G`
    reg    an argument the function was called with: `F reg N = K` or `F reg N & MASK`
    load   a value the function loads: `F load PC SIZE = K` or `F load PC SIZE & MASK`

where K is the constant the compare tests against (a single's bits, for a float compared with
a constant) and MASK the bits a test such as `rlwinm.` looks at. The register is followed
through copies, lone predecessors, and a saved register set once in the entry block, as an
argument's copy is. A listed function's mutated checks take one of its targets half the time:
a callee then returns a small number instead of running, on both sides alike; an argument
becomes K or a neighbor of it, or has the bits toggled; a word the original loaded at PC in
the check being mutated does too.
"""

import argparse
import csv
import glob
import os
import struct
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(__file__))
import coverage  # noqa: E402
import feasible  # noqa: E402

# Instructions that pass a value on with any small value or bit it holds intact.
PASSING = ("mr", "extsb", "extsh", "clrlwi")
LOAD_SIZES = {"lbz": 1, "lbzx": 1, "lbzu": 1, "lbzux": 1, "lhz": 2, "lhzx": 2, "lhzu": 2,
              "lhzux": 2, "lha": 2, "lhax": 2, "lhau": 2, "lhaux": 2, "lwz": 4, "lwzx": 4,
              "lwzu": 4, "lwzux": 4}
ARGS = {f"r{i}" for i in range(3, 11)}
FVOLATILE = {f"f{i}" for i in range(14)}
CTR = (None, "dnz", "dz", "dnzt", "dnzf", "dzt", "dzf", "ns", "so", "un", "nu")


def mask(mb, me):
    """rlwinm's mask from bit mb to bit me, bit 0 the most significant."""
    m = 0
    for b in range(32):
        inside = mb <= b <= me if mb <= me else (b >= mb or b <= me)
        if inside:
            m |= 1 << (31 - b)
    return m


def rotation(m, r):
    """(shift, mask) of an instruction that rotates a register and masks it, as rlwinm and its
    aliases do: the value it writes is rotl(source, shift) & mask. Else None."""
    imm = [feasible.imm(x) for x in r[2:]]
    if any(i is None for i in imm):
        return None
    if m == "rlwinm" and len(imm) == 3:
        return imm[0], mask(imm[1], imm[2])
    if m == "srwi" and len(imm) == 1:
        return (32 - imm[0]) % 32, mask(imm[0], 31)
    if m == "slwi" and len(imm) == 1:
        return imm[0], mask(0, 31 - imm[0])
    if m == "rotlwi" and len(imm) == 1:
        return imm[0], 0xFFFFFFFF
    if m == "extrwi" and len(imm) == 2:
        n, b = imm
        return (b + n) % 32, mask(32 - n, 31)
    if m == "clrrwi" and len(imm) == 1:
        return 0, mask(0, 31 - imm[0])
    return None


def rotr(v, n):
    n %= 32
    return ((v >> n) | (v << (32 - n))) & 0xFFFFFFFF if n else v


def undo(steps, op, k):
    """The value (op "=") or bits (op "&") a register must hold for what the steps computed
    from it to equal k or have k's bits: the steps, outermost last, as source() lists them.
    None where no value does."""
    for step in reversed(steps):
        kind = step[0]
        if kind == "rot":
            sh, m = step[1], step[2]
            if op == "=" and k & ~m & 0xFFFFFFFF:
                return None
            k = rotr(k & m, sh)
        elif kind == "add":
            if op == "&":
                return None
            k = (k - step[1]) & 0xFFFFFFFF
        elif kind == "sra":
            if op == "&":
                k = (k << step[1]) & 0xFFFFFFFF
            else:
                k = (k << step[1]) & 0xFFFFFFFF
    return k


def writes(m, ops, reg):
    """Whether instruction `m ops` writes `reg`."""
    r = feasible.regs(ops)
    if coverage.is_call(m):
        return reg in feasible.VOLATILE
    return bool(r) and r[0] == reg and not m.startswith(feasible.NOT_WRITING)


def source(cfg, b, upto, reg, depth=0, steps=()):
    """What `reg` holds before instruction `upto` of block `b`: ("call", callee), ("reg", reg)
    for an argument, ("load", pc, size), or None, with the steps that computed reg from it
    last (rotations and masks, additions of a constant, arithmetic shifts), which undo() can
    take back. Follows copies, lone predecessors, and a register the whole function sets once,
    as the saved copy of an argument is."""
    blocks, preds = cfg
    block = blocks[b]
    if depth > 8:
        return None
    for j in range(upto - 1, -1, -1):
        pc, m, ops = block[j]
        r = feasible.regs(ops)
        if coverage.is_call(m):
            if reg == "r3" and m == "bl":
                return ("call", r[0])
            if reg in feasible.VOLATILE:
                return None
            continue
        if not writes(m, ops, reg):
            continue
        if m.rstrip(".") in PASSING and len(r) >= 2:
            return source(cfg, b, j, r[1], depth + 1, steps)
        rot = rotation(m.rstrip("."), r) if len(r) >= 2 else None
        if rot is not None:
            return source(cfg, b, j, r[1], depth + 1, (("rot",) + rot,) + steps)
        if m in ("addi", "subi") and len(r) == 3 and r[1] != "r0" and feasible.imm(r[2]) is not None:
            k = feasible.imm(r[2]) * (1 if m == "addi" else -1)
            return source(cfg, b, j, r[1], depth + 1, (("add", k),) + steps)
        if m.rstrip(".") == "srawi" and len(r) == 3 and feasible.imm(r[2]) is not None:
            return source(cfg, b, j, r[1], depth + 1, (("sra", feasible.imm(r[2])),) + steps)
        if m in LOAD_SIZES:
            return ("load", pc, LOAD_SIZES[m], steps)
        return None
    if b == 0:
        return ("reg", reg, steps) if reg in ARGS else None
    if len(preds[b]) == 1:
        p = preds[b][0]
        return source(cfg, p, len(blocks[p]), reg, depth + 1, steps)
    # A saved register set once, in the entry block, holds what it was set to throughout: the
    # epilogue's reload of the caller's value from the stack comes after every use.
    if reg not in feasible.VOLATILE:
        def reload(m, ops):
            r = feasible.regs(ops)
            return m == "lwz" and len(r) == 2 and r[1].endswith("(r1)")
        sets = [(i, j) for i, blk in enumerate(blocks) for j, (_, m, ops) in enumerate(blk)
                if writes(m, ops, reg) and not reload(m, ops)]
        if len(sets) == 1 and sets[0][0] == 0:
            return source(cfg, 0, sets[0][1] + 1, reg, depth + 1, steps)
    return None


def constants(root, unit):
    """The float constants a unit's listing defines, by name: (bits, size)."""
    path = os.path.join(root, "build", "GALE01", "asm", unit + ".s")
    out, name = {}, None
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith(".obj "):
            name = line[5:].split(",")[0].strip().strip('"')
        elif name and line.strip().startswith((".float ", ".double ")):
            kind, text = line.split(None, 1)
            try:
                v = float(text.split("#")[0].strip())
            except ValueError:
                v = None
            if v is not None and kind == ".float":
                out[name] = (struct.unpack(">I", struct.pack(">f", v))[0], 4)
            elif v is not None:
                out[name] = (struct.unpack(">Q", struct.pack(">d", v))[0], 8)
            name = None
        else:
            name = None if line.startswith(".endobj") else name
    return out


def fsource(cfg, b, upto, reg, consts, depth=0):
    """What float register `reg` holds before instruction `upto` of block `b`: ("const",
    bits, size) for a constant of the unit's, ("load", pc, size), or None."""
    blocks, preds = cfg
    block = blocks[b]
    if depth > 8:
        return None
    for j in range(upto - 1, -1, -1):
        pc, m, ops = block[j]
        r = feasible.regs(ops)
        if coverage.is_call(m):
            if reg in FVOLATILE:
                return None
            continue
        if not r or r[0] != reg or m.startswith(feasible.NOT_WRITING):
            continue
        if m in ("fmr", "frsp") and len(r) == 2:
            return fsource(cfg, b, j, r[1], consts, depth + 1)
        if m in ("lfs", "lfd") and len(r) == 2:
            size = 4 if m == "lfs" else 8
            if "@sda21" in r[1]:
                c = consts.get(r[1].split("@sda21")[0].strip('"'))
                return ("const", c[0], size) if c and c[1] == size else None
            return ("load", pc, size)
        return None
    if len(preds[b]) == 1:
        p = preds[b][0]
        return fsource(cfg, p, len(blocks[p]), reg, consts, depth + 1)
    return None


def decider(cfg, b, consts):
    """What decides the conditional branch ending block `b`: (source, "=", K) for a compare
    with a constant, (source, "&", MASK) for a bit test, or None."""
    block = cfg[0][b]
    bm = feasible.BRANCH.match(block[-1][1])
    if not bm or bm.group(1) in CTR:
        return None
    for j in range(len(block) - 2, -1, -1):
        _, m, ops = block[j]
        r = feasible.regs(ops)
        if m in ("cmpwi", "cmplwi"):
            x, k = (r[1], r[2]) if len(r) == 3 else (r[0], r[1])
            k = feasible.imm(k)
            return (source(cfg, b, j, x), "=", k & 0xFFFFFFFF) if k is not None else None
        if m in ("fcmpu", "fcmpo") and len(r) == 3:
            # A loaded single against a constant: the compare looks for that value, and the
            # floats next to it either side.
            x, y = fsource(cfg, b, j, r[1], consts), fsource(cfg, b, j, r[2], consts)
            for a, c in ((x, y), (y, x)):
                if a and c and a[0] == "load" and c[0] == "const" and a[2] == c[2] == 4:
                    return (a, "=", c[1])
            return None
        if m.startswith(("cmp", "fcmp")) or coverage.is_call(m):
            return None
        if m.endswith("."):
            base = m[:-1]
            if base in ("mr", "or") and len(r) >= 2:
                return (source(cfg, b, j, r[1]), "=", 0)
            if base == "extsb" or base == "extsh":
                return (source(cfg, b, j, r[1]), "=", 0)
            if base == "andi" and len(r) == 3 and feasible.imm(r[2]) is not None:
                return (source(cfg, b, j, r[1]), "&", feasible.imm(r[2]))
            if base == "clrlwi" and len(r) == 3 and feasible.imm(r[2]) is not None:
                return (source(cfg, b, j, r[1]), "&", (1 << (32 - feasible.imm(r[2]))) - 1)
            if base == "extrwi" and len(r) == 4:
                n, at = feasible.imm(r[2]), feasible.imm(r[3])
                if n is not None and at is not None:
                    return (source(cfg, b, j, r[1]), "&", ((1 << n) - 1) << (32 - at - n))
            if base == "rlwinm" and len(r) == 5 and feasible.imm(r[2]) == 0:
                mb, me = feasible.imm(r[3]), feasible.imm(r[4])
                if mb is not None and me is not None:
                    return (source(cfg, b, j, r[1]), "&", mask(mb, me))
            return None
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("report")
    ap.add_argument("out")
    ap.add_argument("--coverage", nargs="+", default=[])
    ap.add_argument("--bar", type=float, default=0.9)
    args = ap.parse_args()
    under = {r["name"] for r in csv.DictReader(open(args.report, encoding="utf-8"))
             if int(r["countable"]) > 0 and float(r["share"]) < args.bar}
    funcs = list(coverage.functions(args.root))
    # Each name's functions, by unit: a static callee is the caller's unit's own.
    addrs = defaultdict(dict)
    for unit, name, insns, _ in funcs:
        addrs[name][unit] = insns[0][0]
    todo = [f for f in funcs if f[1] in under]
    spans = {i[0][0]: (i[0][0], i[-1][0] + 4) for _, _, i, _ in todo}
    bits = coverage.load_bits([p for g in args.coverage for p in glob.glob(g)], spans,
                              lambda path: set())
    lines = []
    kinds = defaultdict(int)
    unit_consts = {}
    for unit, name, insns, labels in todo:
        blocks = coverage.blocks(insns, labels)
        index = {blk[0][0]: i for i, blk in enumerate(blocks)}
        preds = [[] for _ in blocks]
        for i, blk in enumerate(blocks):
            _, m, ops = blk[-1]
            t = feasible.target(ops)
            if t is not None and t in index and not coverage.is_call(m):
                preds[index[t]].append(i)
            if m not in ("b", "blr", "bctr", "rfi") and i + 1 < len(blocks):
                preds[i + 1].append(i)
        cfg = (blocks, preds)
        if unit not in unit_consts:
            unit_consts[unit] = constants(args.root, unit)
        unreached = feasible.infeasible(insns, labels)
        hit = [any(coverage.covered(bits, a) for a, _, _ in blk) for blk in blocks]
        seen = set()
        for i, blk in enumerate(blocks):
            if not hit[i]:
                continue
            t = feasible.target(blk[-1][2])
            ways = [index.get(t)] if t is not None else []
            ways.append(i + 1 if i + 1 < len(blocks) else None)
            if all(w is None or hit[w] or blocks[w][0][0] in unreached for w in ways):
                continue
            d = decider(cfg, i, unit_consts[unit])
            if d is None or d[0] is None:
                continue
            src, op, k = d
            if src[0] in ("reg", "load") and isinstance(src[-1], tuple):
                k = undo(src[-1], op, k)
                if k is None or (op == "&" and not k):
                    continue
            if src[0] == "call":
                where = addrs.get(src[1], {})
                at = where.get(unit) or (next(iter(where.values())) if len(where) == 1 else None)
                if at is None or src[1] == name:
                    continue
                line = f"call {at:#010x}"
                note = f"{name} {src[1]}"
            elif src[0] == "reg":
                line = f"reg {src[1][1:]} {op} {k:#x}"
                note = name
            else:
                size = src[2]
                if op == "&":
                    k &= (1 << (8 * size)) - 1
                    if not k:
                        continue
                line = f"load {src[1]:#010x} {size} {op} {k:#x}"
                note = name
            if line not in seen:
                seen.add(line)
                kinds[src[0]] += 1
                lines.append(f"{insns[0][0]:#010x} {line} # {note}\n")
    with open(args.out, "w") as f:
        f.writelines(lines)
    print(f"{len(lines)} targets in {len({ln.split()[0] for ln in lines})} functions: "
          + ", ".join(f"{n} {k}" for k, n in sorted(kinds.items())), file=sys.stderr)


if __name__ == "__main__":
    main()
