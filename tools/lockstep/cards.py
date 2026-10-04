"""Makes memory card images for the memory card screen's prompts, from the saved template (a
formatted 59-block card holding Melee's save), for jobs' --card:

    python tools/lockstep/cards.py TEMPLATE OUT_DIR

  full.raw     Melee's save under another game's code and a 40-block file of that game, so the
               save is missing and the 8 free blocks can't hold a new one (lbCardResult 5,
               lbcardnew.c:493-496)
  badicon.raw  Melee's save with its directory entry's icon address moved off 0x40, which the
               game's load rejects (lbCardResult 3, card.c:1470-1472)

Both copies of the directory and of the block allocation table are rewritten alike, with their
checksums, so CARDCheck finds nothing to repair. The images are game data and stay in local/.
"""
import os
import struct
import sys

BLOCK = 0x2000
DIR, BAT = (1, 2), (3, 4)
ENTRY = 0x40


def checksum(data):
    """__CARDCheckSum (CARDCheck.c): sums of the big-endian u16 words and of their complements."""
    s = si = 0
    for (w,) in struct.iter_unpack(">H", data):
        s = (s + w) & 0xFFFF
        si = (si + (~w & 0xFFFF)) & 0xFFFF
    return (0 if s == 0xFFFF else s), (0 if si == 0xFFFF else si)


def seal_dir(card, blk):
    o = blk * BLOCK
    s, si = checksum(card[o:o + 0x1FFC])
    card[o + 0x1FFC:o + 0x2000] = struct.pack(">HH", s, si)


def seal_bat(card, blk):
    o = blk * BLOCK
    s, si = checksum(card[o + 4:o + BLOCK])
    card[o:o + 4] = struct.pack(">HH", s, si)


def copy_active(card, pair, seal):
    """Makes both copies the active one (the higher check code), resealed."""
    codes = [struct.unpack(">H", card[b * BLOCK + (0x1FFA if pair == DIR else 4):][:2])[0]
             for b in pair]
    src = pair[codes.index(max(codes))]
    for b in pair:
        card[b * BLOCK:(b + 1) * BLOCK] = card[src * BLOCK:(src + 1) * BLOCK]
        seal(card, b)
    return src


def melee_entry(card, d):
    for i in range(127):
        o = d * BLOCK + i * ENTRY
        if card[o:o + 4] == b"GALE":
            return o
    sys.exit("no Melee save on the template")


def full(template):
    card = bytearray(template)
    d = copy_active(card, DIR, seal_dir)
    b = copy_active(card, BAT, seal_bat)
    m = melee_entry(card, d)
    card[m:m + 4] = b"GZZE"
    o = melee_entry_free(card, d)
    start, length = 16, 40
    entry = bytearray(b"\xff" * ENTRY)
    entry[0:6] = b"GZZE01"
    entry[6:8] = b"\xff\x00"
    entry[8:0x28] = b"filler".ljust(32, b"\0")
    struct.pack_into(">I", entry, 0x28, 0)
    struct.pack_into(">BB", entry, 0x34, 4, 0)
    struct.pack_into(">HH", entry, 0x36, start, length)
    struct.pack_into(">H", entry, 0x3A, 0xFFFF)
    card[o:o + ENTRY] = entry
    bo = b * BLOCK
    for k in range(length):
        nxt = start + k + 1 if k + 1 < length else 0xFFFF
        struct.pack_into(">H", card, bo + 0xA + 2 * (start + k - 5), nxt)
    free, = struct.unpack_from(">H", card, bo + 6)
    struct.pack_into(">HH", card, bo + 6, free - length, start + length - 1)
    for blk in DIR:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[d * BLOCK:(d + 1) * BLOCK]
        seal_dir(card, blk)
    for blk in BAT:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[b * BLOCK:(b + 1) * BLOCK]
        seal_bat(card, blk)
    return card


def melee_entry_free(card, d):
    for i in range(127):
        o = d * BLOCK + i * ENTRY
        if card[o:o + 4] == b"\xff\xff\xff\xff":
            return o
    sys.exit("no free directory entry")


def badicon(template):
    card = bytearray(template)
    d = copy_active(card, DIR, seal_dir)
    copy_active(card, BAT, seal_bat)
    struct.pack_into(">I", card, melee_entry(card, d) + 0x2C, 0x80)
    for blk in DIR:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[d * BLOCK:(d + 1) * BLOCK]
        seal_dir(card, blk)
    return card


def main():
    template = open(sys.argv[1], "rb").read()
    os.makedirs(sys.argv[2], exist_ok=True)
    for name, make in (("full", full), ("badicon", badicon)):
        open(os.path.join(sys.argv[2], name + ".raw"), "wb").write(make(template))
        print(os.path.join(sys.argv[2], name + ".raw"))


if __name__ == "__main__":
    main()
