"""Basic blocks no input can reach, from what the machine code itself rules out.

A conditional branch right after a compare can be decided by the compared registers' ranges:
a halfword just loaded with `lhz` is never above 65535, so a clamp at 999999 never runs.
Within the block, each register's range comes from its last definition: a load's width, a
mask, a sign extension, a constant, or a copy or sum of those; anything else may be anything.
Blocks only such impossible edges lead to are infeasible. Functions that jump through a
table (`bctr`) keep every block reachable from a label nothing in them branches to.

    python tools/lockstep/feasible.py <decomp root> [NAME...]
"""

import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))
import coverage  # noqa: E402

FULL = (-(1 << 31), (1 << 31) - 1)
BRANCH = re.compile(r"^b(lt|ge|gt|le|eq|ne|ns|so|un|nu|dnz|dz|dnzt|dnzf|dzt|dzf)?([+-])?$")
LOADS = {"lbz": (0, 255), "lbzx": (0, 255), "lbzu": (0, 255), "lbzux": (0, 255),
         "lhz": (0, 65535), "lhzx": (0, 65535), "lhzu": (0, 65535), "lhzux": (0, 65535),
         "lha": (-32768, 32767), "lhax": (-32768, 32767), "lhau": (-32768, 32767),
         "extsb": (-128, 127), "extsh": (-32768, 32767), "extsb.": (-128, 127),
         "extsh.": (-32768, 32767)}


def imm(s):
    """An immediate operand's value; None for a symbol's, such as `sym@ha`."""
    s = s.strip()
    neg = s.startswith("-")
    s = s.lstrip("-")
    try:
        v = int(s, 16) if s.lower().startswith("0x") else int(s)
    except ValueError:
        return None
    return -v if neg else v


def regs(ops):
    return [o.strip() for o in ops.split(",")]


def target(ops):
    """A branch's target address, if it names a label."""
    m = re.search(r"\.L_([0-9A-Fa-f]{8})", ops)
    return int(m.group(1), 16) if m else None


VOLATILE = {"r0", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12"}
# Instructions whose first operand they read, not write.
NOT_WRITING = ("cmp", "fcmp", "st", "mt", "b", "tw", "dcb", "icb", "sync", "isync", "eieio")


def reg_range(cfg, b, i, reg, depth=0):
    """The range of `reg` before instruction `i` of block `b`, from its last definition there,
    or in the block's only predecessor before it."""
    blocks, preds = cfg
    block = blocks[b]
    if depth > 8:
        return FULL
    for j in range(i - 1, -1, -1):
        _, m, ops = block[j]
        r = regs(ops)
        if coverage.is_call(m):
            if reg in VOLATILE:
                return FULL
            continue
        if not r or r[0] != reg or m.startswith(NOT_WRITING):
            continue
        def known(*vals):
            return all(v is not None for v in vals)
        if m in LOADS:
            return LOADS[m]
        if m == "li" and len(r) == 2 and known(imm(r[1])):
            v = imm(r[1])
            return (v, v)
        if m == "lis" and len(r) == 2 and known(imm(r[1])):
            v = (imm(r[1]) << 16) & 0xFFFFFFFF
            v = v - (1 << 32) if v >= 1 << 31 else v
            return (v, v)
        if m in ("addi", "subi") and len(r) == 3 and known(imm(r[2])):
            lo, hi = reg_range(cfg, b, j, r[1], depth + 1) if r[1] != "r0" else (0, 0)
            d = imm(r[2]) * (-1 if m == "subi" else 1)
            lo, hi = lo + d, hi + d
            return (lo, hi) if FULL[0] <= lo and hi <= FULL[1] else FULL
        if m == "mr" and len(r) == 2:
            return reg_range(cfg, b, j, r[1], depth + 1)
        if m in ("clrlwi", "clrlwi.") and len(r) == 3 and known(imm(r[2])):
            n = imm(r[2])
            return (0, (1 << (32 - n)) - 1) if 0 < n < 32 else FULL
        if m in ("rlwinm", "rlwinm.") and len(r) == 5 and known(imm(r[3]), imm(r[4])):
            mb, me = imm(r[3]), imm(r[4])
            if mb <= me and mb > 0:
                return (0, (1 << (32 - mb)) - 1)
            return FULL
        if m == "andi." and len(r) == 3 and known(imm(r[2])):
            return (0, imm(r[2]))
        return FULL
    if len(preds[b]) == 1:
        p = preds[b][0]
        return reg_range(cfg, p, len(blocks[p]), reg, depth + 1)
    return FULL


def outcomes(a, b, signed):
    """Which of less, greater and equal a compare of ranges `a` and `b` can give."""
    if not signed:
        def unsigned(r):
            lo, hi = r
            return (lo, hi) if lo >= 0 else (0, (1 << 32) - 1)
        a, b = unsigned(a), unsigned(b)
    lt = a[0] < b[1]
    gt = a[1] > b[0]
    eq = a[0] <= b[1] and b[0] <= a[1]
    return lt, gt, eq


def edges(cfg, b):
    """For a block ending in a branch on a compare in it: whether its taken and fall-through
    edges can happen. None where the block doesn't decide it."""
    block = cfg[0][b]
    _, m, ops = block[-1]
    bm = BRANCH.match(m)
    if not bm or bm.group(1) in (None, "dnz", "dz", "dnzt", "dnzf", "dzt", "dzf", "ns", "so",
                                 "un", "nu"):
        return None
    cond = bm.group(1)
    r = regs(ops)
    field = r[0] if len(r) == 2 else "cr0"
    for i in range(len(block) - 2, -1, -1):
        a, cm, cops = block[i]
        c = regs(cops)
        if cm in ("cmpw", "cmplw", "cmpwi", "cmplwi"):
            cf = c[0] if len(c) == 3 else "cr0"
            if cf != field:
                continue
            x, y = (c[1], c[2]) if len(c) == 3 else (c[0], c[1])
            ra = reg_range(cfg, b, i, x)
            if cm.endswith("i"):
                if imm(y) is None:
                    return None
                rb = (imm(y), imm(y))
            else:
                rb = reg_range(cfg, b, i, y)
            lt, gt, eq = outcomes(ra, rb, not cm.startswith("cmpl"))
            taken = {"lt": lt, "ge": gt or eq, "gt": gt, "le": lt or eq, "eq": eq,
                     "ne": lt or gt}[cond]
            fall = {"lt": gt or eq, "ge": lt, "gt": lt or eq, "le": gt, "eq": lt or gt,
                    "ne": eq}[cond]
            return taken, fall
        if cm.endswith(".") or cm.startswith("cmp") or cm.startswith("fcmp") or \
                coverage.is_call(cm) or cm.startswith("mtcr") or cm.startswith("cr"):
            # The condition comes from somewhere this doesn't model.
            if cf_set(cm, cops, field):
                return None
    return None


def cf_set(m, ops, field):
    """Whether instruction `m` may set condition field `field`."""
    if coverage.is_call(m):
        return True
    if m.endswith("."):
        return field == "cr0"
    if m.startswith("cmp") or m.startswith("fcmp"):
        r = regs(ops)
        return (r[0] if r and r[0].startswith("cr") else "cr0") == field
    return m.startswith("mtcr") or m.startswith("cr") or m == "mcrf"


def infeasible(insns, labels):
    """The start addresses of a function's blocks no input can reach."""
    blocks = coverage.blocks(insns, labels)
    starts = [b[0][0] for b in blocks]
    index = {a: i for i, a in enumerate(starts)}
    targeted = set()
    for b in blocks:
        t = target(b[-1][2])
        if t is not None:
            targeted.add(t)
    preds = [[] for _ in blocks]
    for i, b in enumerate(blocks):
        _, m, ops = b[-1]
        t = target(ops)
        if t is not None and t in index and not coverage.is_call(m):
            preds[index[t]].append(i)
        ends = m in ("b", "blr", "bctr", "rfi")
        if not ends and i + 1 < len(blocks):
            preds[i + 1].append(i)
    cfg = (blocks, preds)
    roots = [0]
    if any(m in ("bctr", "bctrl") for _, m, _ in insns):
        roots += [index[a] for a in starts if a in labels and a not in targeted and a in index]
    seen, todo = set(), list(roots)
    while todo:
        i = todo.pop()
        if i in seen:
            continue
        seen.add(i)
        b = blocks[i]
        _, m, ops = b[-1]
        nxt = i + 1 if i + 1 < len(blocks) else None
        t = target(ops)
        ti = index.get(t) if t is not None else None
        if m in ("blr", "bctr", "rfi") or m.startswith("blr") or m.startswith("bctr"):
            # A conditional return falls through when it isn't taken.
            if m not in ("blr", "bctr", "rfi") and nxt is not None:
                todo.append(nxt)
            continue
        if m == "b":
            if ti is not None:
                todo.append(ti)
            continue
        if BRANCH.match(m) and ti is not None:
            e = edges(cfg, i)
            taken, fall = e if e is not None else (True, True)
            if taken:
                todo.append(ti)
            if fall and nxt is not None:
                todo.append(nxt)
            continue
        if nxt is not None:
            todo.append(nxt)
    return {starts[i] for i in range(len(blocks)) if i not in seen}


def main():
    root = sys.argv[1]
    names = set(sys.argv[2:])
    total = 0
    funcs = 0
    for unit, name, insns, labels in coverage.functions(root):
        if names and name not in names:
            continue
        dead = infeasible(insns, labels)
        if dead:
            funcs += 1
            total += len(dead)
            if names:
                print(name, " ".join(f"{a:08X}" for a in sorted(dead)))
    print(f"{total} infeasible blocks in {funcs} functions", file=sys.stderr)


if __name__ == "__main__":
    main()
