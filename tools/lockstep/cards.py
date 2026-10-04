"""Makes memory card images for the memory card screen's prompts, from the saved template (a
formatted 59-block card holding Melee's save), for jobs' --card:

    python tools/lockstep/cards.py TEMPLATE OUT_DIR
    python tools/lockstep/cards.py --snaps SNAPSHOTS OUT_DIR

  full.raw     Melee's save under another game's code and a 40-block file of that game, so the
               save is missing and the 8 free blocks can't hold a new one (lbCardResult 5,
               lbcardnew.c:493-496)
  badicon.raw  Melee's save with its directory entry's icon address moved off 0x40, which the
               game's load rejects (lbCardResult 3, card.c:1470-1472)

and, from a card holding Melee's save and snapshots (one Camera Mode filled), for the snapshot
album's (mnsnap.c):

  snap5.raw         the first 5 snapshots, so the album has two pages
  snap1.raw         the first snapshot alone, whose delete leaves the album empty
  snapdamaged.raw   all of them, the album's first (the newest) with its data damaged so its read
                    fails its check and the album marks it bad (mnsnap.c:248-252)
  snap1damaged.raw  the damaged one alone

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


def snapshots(card, d):
    """The directory entries of Melee's snapshots, in directory order (the album sorts them by name, a
    number, the highest first: taskListSnapshots, lbcardnew.c:729)."""
    found = []
    for i in range(127):
        o = d * BLOCK + i * ENTRY
        name = bytes(card[o + 8:o + 0x28]).split(b"\0")[0]
        if card[o:o + 4] == b"GALE" and name.isdigit():
            found.append(o)
    return found


def keep(template, n, damage):
    """Keeps the first n snapshots (all, for None), the album's first of them damaged if asked."""
    card = bytearray(template)
    d = copy_active(card, DIR, seal_dir)
    b = copy_active(card, BAT, seal_bat)
    bo = b * BLOCK
    snaps = snapshots(card, d)
    kept, dropped = (snaps, []) if n is None else (snaps[:n], snaps[n:])
    for o in dropped:
        blk, = struct.unpack_from(">H", card, o + 0x36)
        while blk != 0xFFFF:
            nxt, = struct.unpack_from(">H", card, bo + 0xA + 2 * (blk - 5))
            struct.pack_into(">H", card, bo + 0xA + 2 * (blk - 5), 0)
            free, = struct.unpack_from(">H", card, bo + 6)
            struct.pack_into(">H", card, bo + 6, free + 1)
            blk = nxt
        card[o:o + ENTRY] = b"\xff" * ENTRY
    if damage:
        # The album lists the newest (highest number) first, so the damaged one is under its cursor
        # at the start. The bytes are in the file's second block, past the 0x20 header whose first
        # 16 bytes are the checksum of the rest (HSD_Decrypt, crypt.c:197-203), so its read fails
        # with -0x105 (card.c:205-208), lbCardResult 2 (lbcardnew.c:311-315).
        first = max(kept, key=lambda o: int(bytes(card[o + 8:o + 0x28]).split(b"\0")[0]))
        start, = struct.unpack_from(">H", card, first + 0x36)
        second, = struct.unpack_from(">H", card, bo + 0xA + 2 * (start - 5))
        o = second * BLOCK + 0x100
        card[o:o + 16] = bytes(x ^ 0x5A for x in card[o:o + 16])
    for blk in DIR:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[d * BLOCK:(d + 1) * BLOCK]
        seal_dir(card, blk)
    for blk in BAT:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[b * BLOCK:(b + 1) * BLOCK]
        seal_bat(card, blk)
    return card


def main():
    snaps = sys.argv[1] == "--snaps"
    args = sys.argv[2:] if snaps else sys.argv[1:]
    template = open(args[0], "rb").read()
    os.makedirs(args[1], exist_ok=True)
    makes = (("full", full), ("badicon", badicon))
    if snaps:
        makes = (("snap5", lambda t: keep(t, 5, False)), ("snap1", lambda t: keep(t, 1, False)),
                 ("snapdamaged", lambda t: keep(t, None, True)),
                 ("snap1damaged", lambda t: keep(t, 1, True)))
    for name, make in makes:
        open(os.path.join(args[1], name + ".raw"), "wb").write(make(template))
        print(os.path.join(args[1], name + ".raw"))


if __name__ == "__main__":
    main()
