"""Where MWCC puts each function's locals, from its own debug info.

    python tools/c2rs/frames.py <decomp root> <out.json> [unit ...]

Compiles every C unit of the decomp's build with `-sym on`, which leaves the code as it is,
and reads the DWARF that decomp-toolkit dumps: for each function, its parameters and locals in
the order the compiler lists them, each with where it lives, `r1+0x10` or a register. c2rs
puts locals the original keeps in its frame at the same offsets, so the port's stack matches
the original's wherever its locals are.
"""

import json
import os
import re
import shlex
import struct
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

ANSI = re.compile(r"\x1b\[[0-9;]*m")
LOCAL = re.compile(r"^    (?:.*[\s*}])?([A-Za-z_]\w*)((?:\[[^\]]*\])*);\s*// (\S+)$")
WHERE = re.compile(r"/\* (\S+) \*/")
SECTIONS = ("Local variables", "References", "Inlined", "Blocks", "Labels")
HEAD = re.compile(r"^[A-Za-z_][^/(]*?\b([A-Za-z_]\w*)\(")


def build_units(root):
    """(unit, source, object, command) for each C unit in the decomp's build.ninja."""
    text = open(os.path.join(root, "build.ninja"), encoding="utf-8").read().replace("$\n", "")
    # Units by source, under the names objdiff.json gives them, as c2rs uses.
    names = {u["metadata"]["source_path"].replace("\\", "/"): u["name"].removeprefix("main/")
             for u in json.load(open(os.path.join(root, "objdiff.json"), encoding="utf-8"))["units"]
             if u.get("metadata", {}).get("source_path")}
    units = []
    for block in text.split("\nbuild ")[1:]:
        lines = block.splitlines()
        m = re.match(r"(\S+): (mwcc\w*) (\S+\.c)\b", re.sub(r"\s+", " ", lines[0]))
        if not m:
            continue
        obj, rule, src = m.groups()
        attrs = {}
        for line in lines[1:]:
            if not line.startswith("  "):
                break
            k, _, v = line.strip().partition(" = ")
            attrs[k] = re.sub(r"\s+", " ", v)
        cflags = attrs.get("cflags", "").replace("-sym off", "-sym on")
        if "-sym on" not in cflags:
            cflags += " -sym on"
        cc = [os.path.join(root, "build", "compilers", attrs["mw_version"], "mwcceppc.exe")]
        if "sjis" in rule:
            cc = [os.path.join(root, "build", "tools", "sjiswrap.exe")] + cc
        unit = names.get(src.replace("\\", "/"))
        if unit is not None:
            units.append((unit, src, obj, cc + shlex.split(cflags, posix=True)))
    return units


def parameters(header):
    """[name, where] for each parameter of a function's header, as `type name /* where */`."""
    out = []
    for m in WHERE.finditer(header):
        before = header[:m.start()].rstrip()
        # `type (* name)(params)` or `type (* name)[n]`, else `type name[n]`.
        name = re.search(r"\(\s*\*+\s*([A-Za-z_]\w*)\s*\)(?:\s*\([^()]*\)|\s*\[[^\]]*\])*$", before) or \
            re.search(r"([A-Za-z_]\w*)(?:\s*\[[^\]]*\])*$", before)
        if name:
            out.append([name.group(1), m.group(1)])
    return out


def parse(dump):
    """function -> {"params": [[name, where]], "locals": [[name, where]]}."""
    out, name, body, header, ranged = {}, None, None, [], False
    for line in dump.splitlines():
        line = ANSI.sub("", line).rstrip()
        if name is None:
            # A function's definition follows the comment that gives its address range.
            m = HEAD.match(line) if ranged else None
            ranged = line.startswith("// Range:")
            if not m:
                continue
            name, header, body = m.group(1), [], None
        if body is None:
            # A header ends at its closing parenthesis, which an empty body follows on the same
            # line; struct types expanded in it open braces of their own.
            header.append(line)
            if re.search(r"\)\s*\{\}$", line):
                out[name] = {"params": parameters(" ".join(header)), "locals": []}
                name = None
            elif re.search(r"\)\s*\{$", line):
                body = []
            continue
        if line == "}":
            params = parameters(" ".join(header))
            locals_, section = [], None
            for b in body:
                if b.startswith("    // ") and b[7:].startswith(SECTIONS):
                    section = b[7:]
                elif section == "Local variables":
                    m = LOCAL.match(b)
                    if m:
                        locals_.append([m.group(1), m.group(3)])
            out[name] = {"params": params, "locals": locals_}
            name = None
            continue
        body.append(line)
    return out


def code(path):
    """The code of each function an ELF object defines, by name."""
    d = open(path, "rb").read()
    shoff = struct.unpack_from(">I", d, 0x20)[0]
    size, count, _ = struct.unpack_from(">HHH", d, 0x2E)
    sections = [struct.unpack_from(">IIIIIIIIII", d, shoff + i * size) for i in range(count)]
    out = {}
    for _, kind, _, _, offset, length, link, _, _, entsize in sections:
        if kind != 2:  # SHT_SYMTAB
            continue
        strtab = sections[link][4]
        for i in range(length // entsize):
            name, value, sym_size, info, _, shndx = struct.unpack_from(">IIIBBH", d, offset + i * entsize)
            if info & 0xF == 2 and 0 < shndx < count:  # STT_FUNC
                n = d[strtab + name:d.index(b"\x00", strtab + name)].decode()
                at = sections[shndx][4] + value
                out[n] = d[at:at + sym_size]
    return out


def run(root, tmp, dtk, unit, src, obj, cmd):
    odir = os.path.join(tmp, unit.replace("/", "__"))
    os.makedirs(odir, exist_ok=True)
    r = subprocess.run(cmd + ["-c", src, "-o", odir], cwd=root, capture_output=True, text=True)
    produced = os.path.join(odir, os.path.basename(src)[:-2] + ".o")
    if r.returncode != 0 or not os.path.exists(produced):
        return unit, None, (r.stdout + r.stderr).strip()[:300]
    # Debug info must leave the code as the matching build has it.
    matching = os.path.join(root, obj)
    if os.path.exists(matching) and code(matching) != code(produced):
        return unit, None, "its code differs with -sym on"
    dump = os.path.join(odir, "dwarf.txt")
    r = subprocess.run([dtk, "dwarf", "dump", "--no-color", "-o", dump, produced], capture_output=True, text=True)
    if r.returncode != 0:
        return unit, None, r.stderr.strip()[:300]
    return unit, parse(open(dump, encoding="utf-8", errors="replace").read()), None


def main():
    root, out = sys.argv[1], sys.argv[2]
    only = set(sys.argv[3:])
    dtk = os.path.join(root, "build", "tools", "dtk.exe")
    units = [u for u in build_units(root) if not only or u[0] in only]
    result, failed = {}, []
    with tempfile.TemporaryDirectory() as tmp, ThreadPoolExecutor(8) as pool:
        for unit, frames, err in pool.map(lambda u: run(root, tmp, dtk, *u), units):
            if frames is None:
                failed.append((unit, err))
            else:
                result[unit] = frames
    if only and os.path.exists(out):
        merged = json.load(open(out, encoding="utf-8"))
        merged.update(result)
        result = merged
    json.dump(result, open(out, "w", encoding="utf-8"), indent=0, sort_keys=True)
    fns = sum(len(v) for v in result.values())
    on_stack = sum(1 for v in result.values() for f in v.values() for _, w in f["locals"] if w.startswith("r1+"))
    print(f"{len(result)} units, {fns} functions, {on_stack} locals in frames; {len(failed)} units failed")
    for unit, err in failed[:20]:
        print(f"  {unit}: {err}")


if __name__ == "__main__":
    main()
