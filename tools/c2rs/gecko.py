"""Gecko code lists as Slippi playback applies them: Dolphin's `.ini` format, bare code lists,
and the list a Slippi replay records, without the codes playback leaves out."""

import os
import re
import struct

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
SLIPPI_DATA = os.path.join(ROOT, "crates", "slippi", "data")


class Code:
    """One code: its type (0x04, 0xC2, ...), the game address it writes, and its lines' words
    after the first line."""

    def __init__(self, kind, addr, first, words):
        self.kind = kind
        self.addr = addr
        self.first = first  # the first line's second word: a 04's value, a C2's line count
        self.words = words

    def __repr__(self):
        return f"Code({self.kind:02X}, {self.addr:#010x}, {len(self.words)} words)"


def lines_from_text(text, enabled=None):
    """(address word, data word) pairs of the codes in `[Gecko]` (all, or those `enabled`
    names), as Dolphin's INI reader takes them: a line with `=` in it is a key, not code."""
    out, name, section = [], None, "[Gecko]"
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("["):
            section = line
            continue
        if section != "[Gecko]":
            continue
        if "=" in line and not line.startswith(("$", "+", "*")):
            continue
        if line.startswith("$"):
            name = line[1:].split(" [")[0].strip()
            continue
        if enabled is not None and name not in enabled:
            continue
        body = line.split("#")[0].split()
        if len(body) == 2 and all(re.fullmatch(r"[0-9A-Fa-f]{8}", x) for x in body):
            out.append((int(body[0], 16), int(body[1], 16)))
    return out


def enabled_names(text):
    part = text.split("[Gecko_Enabled]", 1)
    if len(part) < 2:
        return set()
    names = set()
    for line in part[1].splitlines():
        line = line.strip()
        if line.startswith("["):
            break
        if line.startswith("$"):
            names.add(line[1:].strip())
    return names


def bootloader():
    text = open(os.path.join(SLIPPI_DATA, "bootloader.txt"), encoding="utf-8").read()
    return codes(lines_from_text(text))


def playback_set():
    text = open(os.path.join(SLIPPI_DATA, "playback.ini"), encoding="utf-8").read()
    return codes(lines_from_text(text, enabled_names(text)))


def codes(lines):
    """The codes in a list of lines, up to its terminator."""
    out = []
    i = 0
    while i < len(lines):
        a, d = lines[i]
        kind = a >> 24 & 0xFE
        if kind == 0xF0:
            break
        n = 0
        if kind in (0xC0, 0xC2):
            n = d
        elif kind == 0x06:
            n = (d + 7) // 8
        elif kind == 0x08:
            n = 1
        words = [w for pair in lines[i + 1:i + 1 + n] for w in pair]
        out.append(Code(kind, 0x8000_0000 | (a & 0x01FF_FFFF), d, words))
        i += 1 + n
    return out


def lines_from_bytes(raw):
    return [struct.unpack(">II", raw[i:i + 8]) for i in range(0, len(raw) - 7, 8)]


def denylist():
    text = open(os.path.join(ROOT, "crates", "slippi", "src", "denylist.rs"), encoding="utf-8").read()
    return {int(x, 16) for x in re.findall(r"0x([0-9A-F]{8})", text)}


def replay_list(path):
    """The Gecko code list a Slippi replay recorded, as raw bytes."""
    data = open(path, "rb").read()
    i = data.find(b"raw[$U#l")
    if i < 0:
        raise ValueError(f"{path}: no raw stream")
    n = struct.unpack(">I", data[i + 8:i + 12])[0]
    raw = data[i + 12:i + 12 + n]
    if raw[0] != 0x35:
        raise ValueError(f"{path}: no event payload sizes")
    sizes = {}
    for j in range(2, 1 + raw[1], 3):
        sizes[raw[j]] = struct.unpack(">H", raw[j + 1:j + 3])[0]
    at, out = 1 + raw[1], bytearray()
    while at < len(raw):
        cmd = raw[at]
        payload = raw[at + 1:at + 1 + sizes[cmd]]
        at += 1 + sizes[cmd]
        # Message splitter: 512 bytes of data, their length, the inner command, the last flag.
        if cmd == 0x10 and payload[514] == 0x3D:
            out += payload[:struct.unpack(">H", payload[512:514])[0]]
            if payload[515]:
                break
    return bytes(out)


def playback_replay_codes(path, deny=None):
    """The codes of a replay's list that playback applies: all but those at denylisted
    addresses, as `CEXISlippi::prepareGeckoList` leaves them."""
    deny = denylist() if deny is None else deny
    lines = lines_from_bytes(replay_list(path))
    if lines and lines[0] == (0x00D0C0DE, 0x00D0C0DE):
        lines = lines[1:]
    return [c for c in codes(lines) if c.addr not in deny]
