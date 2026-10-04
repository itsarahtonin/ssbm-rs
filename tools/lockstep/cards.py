"""Makes memory card images for the memory card screen's prompts, from the saved template (a
formatted 59-block card holding Melee's save), for jobs' --card:

    python tools/lockstep/cards.py TEMPLATE OUT_DIR
    python tools/lockstep/cards.py --snaps SNAPSHOTS OUT_DIR
    python tools/lockstep/cards.py --saves TEMPLATE OUT_DIR

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

and, from the saved template, cards whose save the HSD card layer (card.c) reads, repairs and
rewrites by its less common paths, for jobs that check that layer:

    python tools/lockstep/cards.py --saves TEMPLATE OUT_DIR

  v1.raw    the save's mirror of the game data damaged, which the load rebuilds from the primary
            with CARD_CMD_READ_SECTOR and CARD_CMD_WRITE_SECTOR (fn_803AD16C, card.c:2539-2590)
  v2.raw    a third copy of the game data in the spare block, which the load clears (card.c:2526)
  v3a.raw   the directory entry's banner the small (indexed) one, and v3b.raw no banner, with the
            header's checksum where that shorter header ends: the header the autosave rewrites
            takes the switch's other cases (card.c:1368-1378)
  v4.raw    six RGB5A3 icons, so the header fills three sectors and the data blocks follow them
            (file 13 blocks): the header's reads, checks and writes go sector by sector
  v5.raw    the first name tag file as one of 9 blocks with file flags 3 (no copies), in a 19-block
            file; v5a-d.raw add a stale copy of its first block in the spare block, with sequence
            numbers either side of the 0/0xFF wrap (fn_803ACB74), whose read is refused
            (card.c:2901)
  v6.raw    the game data file as two blocks (0x2000 bytes) and their mirrors (file 13 blocks)
  v7.raw    the second name tag file two blocks long, so there are two spare blocks, and in them
            two older copies of the first name tag file, which a write frees (card.c:3848-3874)
  v8a.raw   a file 0 of file flags 1 in the header sector's tail (block 0); v8b.raw one of file
            flags 0, which the repair can't mirror (block 0 is no sector of its own), v8c.raw that
            with the last name tag's block damaged too; v8d.raw a flags-0 file 0 with an older copy
            in the spare block, v8e.raw that with the damaged name tag
  v9.raw    a file 0 of file flags 3 with an older copy in the spare block, which its read refuses;
            the autosave checks file 0 against the game's NULL buffer for it and faults the game
            itself, so v9 jobs stay on the title screen
  nosave.raw  the card without Melee's save, so the game offers to create one

The HSD layer's blocks are sectors of the card's file: a 0x20-byte header (a checksum of the rest,
the block's id and sequence number, and three entries of the file table) and data, encrypted with
HSD_Encrypt (crypt.c). Each card is checked by decrypting what it wrote back.

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


# The HSD card layer (card.c): blocks of SECTOR bytes, each a header and DATA bytes of data.
SECTOR = BLOCK
DATA = SECTOR - 0x20
KEYS = (0x26, 0xFF, 0xE8, 0xEF, 0x42, 0xD6, 0x01, 0x54, 0x14, 0xA3, 0x80, 0xFD, 0x6E)
# Each case of encryptByte (crypt.c) moves the bits of a byte: (shift, mask) pairs, a negative
# shift going right.
SHUFFLES = (
    ((0, 1), (3, 0x10), (-1, 2), (2, 0x20), (-2, 4), (1, 0x40), (-3, 8), (0, 0x80)),
    ((3, 8), (-1, 1), (0, 4), (3, 0x40), (1, 0x20), (-1, 0x10), (1, 0x80), (-6, 2)),
    ((6, 0x40), (4, 0x20), (-2, 1), (-2, 2), (-1, 8), (2, 0x80), (-4, 4), (-3, 0x10)),
    ((1, 2), (2, 8), (5, 0x80), (1, 0x10), (-4, 1), (-3, 4), (-1, 0x20), (-1, 0x40)),
    ((7, 0x80), (1, 4), (3, 0x20), (-3, 1), (2, 0x40), (-4, 2), (-2, 0x10), (-4, 8)),
    ((5, 0x20), (5, 0x40), (2, 0x10), (0, 8), (3, 0x80), (-5, 1), (-5, 2), (-5, 4)),
    ((2, 4), (0, 2), (4, 0x40), (4, 0x80), (0, 0x10), (-2, 8), (-6, 1), (-2, 0x20)),
)


def shuffle(case, v):
    return sum(((v << k) if k >= 0 else (v >> -k)) & m for k, m in SHUFFLES[case])


ENCRYPT = [[shuffle(p % 7, p ^ c ^ KEYS[p % 13]) for c in range(256)] for p in range(256)]
DECRYPT = [[0] * 256 for _ in range(256)]
for _p in range(256):
    for _c in range(256):
        DECRYPT[_p][ENCRYPT[_p][_c]] = _c


def hsd_checksum(data):
    """HSD_Checksum (crypt.c): MD5's initial state with the bytes added in, 16 at a time."""
    m = list(bytes.fromhex("0123456789abcdeffedcba9876543210"))
    for i, x in enumerate(data):
        m[i % 16] = (m[i % 16] + x) & 0xFF
    for i in range(1, 16):
        if m[i - 1] == m[i]:
            m[i] ^= 0xFF
    return bytes(m)


def encrypt(plain):
    """HSD_Encrypt: the checksum of the rest in the first 16 bytes, then each byte by the one
    before it as written."""
    out = bytearray(plain)
    out[:16] = hsd_checksum(plain[16:])
    for i in range(16, len(out)):
        out[i] = ENCRYPT[out[i - 1]][out[i]]
    return bytes(out)


def decrypt(data):
    """HSD_Decrypt: the bytes, and whether their checksum holds."""
    out = bytearray(data)
    for i in range(16, len(out)):
        out[i] = DECRYPT[data[i - 1]][data[i]]
    return bytes(out), hsd_checksum(out[16:]) == out[:16]


def header_size(banner, icons):
    """hsd_803AC340: comment, banner (format 1 or 2) and icons (format 1 shares a palette)."""
    size = 0x40 + {1: 0xE00, 2: 0x1800}.get(banner, 0)
    size += sum({1: 0x400, 2: 0x800}.get(f, 0) for f in icons)
    return size + (0x200 if 1 in icons else 0)


def file_table(sizes, flags, idx):
    """fn_803AC3F8: the entries of the file table a block of file idx carries, three files
    around it."""
    start = idx - 2 if idx + 1 >= 9 or sizes[idx + 1] == 0 else idx - 1
    out = b""
    for f in range(max(start, 0), max(start, 0) + 3):
        size, flag = (sizes[f], flags[f]) if f < 9 else (0, 0)
        out += bytes((f, (size >> 16) & 0x3F | (flag << 6) & 0xC0, (size >> 8) & 0xFF, size & 0xFF))
    return out


def hsd_block(block_id, seq, table, data, length=SECTOR):
    """A block as written: its header (block id, sequence number, file table) and data, encrypted
    as CARD_CMD_WRITE_BLOCK does."""
    plain = bytearray(length)
    plain[0x10:0x13] = bytes((block_id >> 8 & 0xFF, block_id & 0xFF, seq))
    plain[0x13:0x1F] = table
    plain[0x20:0x20 + len(data)] = data[:length - 0x20]
    return encrypt(bytes(plain))


class Save:
    """Melee's save on the template: its directory entry, the chain of its card blocks, and the
    HSD layer's view of them (Melee's header is one sector: comment, the large banner and one
    indexed icon, 0x1E40 bytes, then their checksum; its blocks 1-10 are the game data, the seven
    name tag files, the spare and the game data's mirror)."""

    def __init__(self, template):
        self.card = bytearray(template)
        self.d = copy_active(self.card, DIR, seal_dir)
        self.b = copy_active(self.card, BAT, seal_bat)
        self.entry = melee_entry(self.card, self.d)
        self.chain = self.blocks_of()
        self.sectors = [bytes(self.card[k * BLOCK:(k + 1) * BLOCK]) for k in self.chain]
        self.header = self.sectors[0][:0x1E40]
        self.blocks = []
        for sector in self.sectors[1:]:
            plain, ok = decrypt(sector)
            assert ok, "a damaged block on the template"
            self.blocks.append((plain[0x10] << 8 | plain[0x11], plain[0x12], plain[0x20:]))
        self.sizes = [0, 6032] + [7980] * 7
        self.flags = [0, 0] + [1] * 7

    def fat(self, blk):
        return struct.unpack_from(">H", self.card, self.b * BLOCK + 0xA + 2 * (blk - 5))[0]

    def set_fat(self, blk, nxt):
        struct.pack_into(">H", self.card, self.b * BLOCK + 0xA + 2 * (blk - 5), nxt)

    def blocks_of(self):
        blk, = struct.unpack_from(">H", self.card, self.entry + 0x36)
        chain = []
        while blk != 0xFFFF:
            chain.append(blk)
            blk = self.fat(blk)
        return chain

    def data(self, block_id):
        """The data of the template's block with that id."""
        return next(data for i, _, data in self.blocks if i == block_id)

    def resize(self, length):
        """Makes the file `length` card blocks long, taking free blocks at its end."""
        bo = self.b * BLOCK
        while len(self.chain) < length:
            blk = next(k for k in range(5, len(self.card) // BLOCK) if self.fat(k) == 0)
            self.set_fat(self.chain[-1], blk)
            self.set_fat(blk, 0xFFFF)
            self.chain.append(blk)
            free, last = struct.unpack_from(">HH", self.card, bo + 6)
            struct.pack_into(">HH", self.card, bo + 6, free - 1, max(last, blk))
        struct.pack_into(">H", self.card, self.entry + 0x38, length)

    def write(self, headers, blocks, block0=None):
        """Writes the file: its header sectors, block 0 (block id 0, sequence number, file index,
        data) in the last one's tail, and blocks 1 on (block id, sequence number, the file index
        whose table it carries, data), the file tables from sizes and flags."""
        hs = header_size(self.card[self.entry + 7] & 3, self.icon_formats())
        assert len(headers) == (hs + 0x30 + SECTOR - 1) // SECTOR
        sectors = [bytearray(h) for h in headers]
        if block0 is not None:
            off = (hs + 0x30) % SECTOR
            seq, idx, data = block0
            sectors[-1][off:] = hsd_block(0, seq, file_table(self.sizes, self.flags, idx), data,
                                          SECTOR - off)
        for block_id, seq, idx, data in blocks:
            sectors.append(hsd_block(block_id, seq, file_table(self.sizes, self.flags, idx), data))
        self.resize(len(sectors))
        for blk, sector in zip(self.chain, sectors):
            self.card[blk * BLOCK:(blk + 1) * BLOCK] = sector
        self.check(hs, len(headers), block0, blocks)

    def check(self, hs, n, block0, blocks):
        """Decrypts what write wrote: every block's checksum holds and carries its id."""
        sectors = [self.card[k * BLOCK:(k + 1) * BLOCK] for k in self.blocks_of()]
        assert len(sectors) == n + len(blocks)
        if block0 is not None:
            plain, ok = decrypt(bytes(sectors[n - 1][(hs + 0x30) % SECTOR:]))
            assert ok and plain[0x10:0x12] == b"\0\0"
        for sector, (block_id, seq, _, _) in zip(sectors[n:], blocks):
            plain, ok = decrypt(bytes(sector))
            assert ok and (plain[0x10] << 8 | plain[0x11], plain[0x12]) == (block_id, seq)

    def icon_formats(self):
        fmt, speed = struct.unpack_from(">HH", self.card, self.entry + 0x30)
        out = []
        for k in range(8):
            if speed >> 2 * k & 3 == 0:
                break
            out.append(fmt >> 2 * k & 3)
        return out

    def default_blocks(self):
        """Blocks 1-10 as Melee writes them: game data (id 1), name tags (ids 2-8, file table of
        files 2-8), spare (id 0xFFFF), mirror (id 1)."""
        out = []
        for idx, (block_id, seq, data) in enumerate(self.blocks, 1):
            f = {1: 1, 10: 1, 9: 9}.get(idx, block_id)
            out.append((block_id, seq, f, data))
        return out

    def sealed(self):
        for blk in DIR:
            self.card[blk * BLOCK:(blk + 1) * BLOCK] = self.card[self.d * BLOCK:][:BLOCK]
            seal_dir(self.card, blk)
        for blk in BAT:
            self.card[blk * BLOCK:(blk + 1) * BLOCK] = self.card[self.b * BLOCK:][:BLOCK]
            seal_bat(self.card, blk)
        return self.card


def with_digest(header, hs):
    """Header sectors ending in the 0x30-byte digest after hs bytes: the checksum of each sector's
    share of the header (the CARD_CMD_READ_HEADER check, card.c:466-487)."""
    header = bytes(header[:hs])
    n = (hs + 0x30 + SECTOR - 1) // SECTOR
    digest = b"".join(hsd_checksum(header[k * SECTOR:(k + 1) * SECTOR]) for k in range(n))
    full = (header + digest.ljust(0x30, b"\0")).ljust(n * SECTOR, b"\0")
    return [full[k * SECTOR:(k + 1) * SECTOR] for k in range(n)]


def v_mirror(save):
    # The mirror is the file's last block; damage what follows its block header.
    blk = save.chain[-1]
    o = blk * BLOCK + 0x100
    save.card[o:o + 16] = bytes(x ^ 0x5A for x in save.card[o:o + 16])


def v_third(save):
    save.card[save.chain[9] * BLOCK:(save.chain[9] + 1) * BLOCK] = save.sectors[1]


def v_banner(fmt):
    def make(save):
        save.card[save.entry + 7] = save.card[save.entry + 7] & ~3 | fmt
        hs = header_size(fmt, save.icon_formats())
        save.write(with_digest(save.header, hs), save.default_blocks())
    return make


def v_icons(save):
    struct.pack_into(">HH", save.card, save.entry + 0x30, 0xAAA, 0xFFF)
    icons = (save.header[0x1840:0x1C40] * 2) * 6
    hs = header_size(2, [2] * 6)
    save.write(with_digest(save.header[:0x1840] + icons, hs), save.default_blocks())


def v_flags3(stale):
    def make(save):
        save.sizes[2], save.flags[2] = 9 * DATA, 3
        blocks = [(1, 3, 1, save.data(1))]
        seq = stale[0] if stale else 0
        blocks += [(2, seq, 2, save.data(2)), (3, seq, 2, save.data(3))]
        blocks += [(2 + k, seq, 2, b"") for k in range(2, 9)]
        blocks += [(11 + k, 0, 3 + k, save.data(3 + k)) for k in range(6)]
        if stale:
            blocks.append((2, stale[1], 2, save.data(2)))
        else:
            blocks.append((0xFFFF, 0, 9, b""))
        blocks.append((1, 3, 1, save.data(1)))
        save.write([save.sectors[0]], blocks)
    return make


def v_twoblocks(save):
    save.sizes[1] = 0x2000
    first, second = (1, 3, 1, save.data(1)), (2, 3, 1, b"")
    blocks = [first, second] + [(3 + k, 0, 2 + k, save.data(2 + k)) for k in range(7)]
    blocks += [(0xFFFF, 0, 9, b""), first, second]
    save.write([save.sectors[0]], blocks)


def v_excess(save):
    save.sizes[3] = DATA + 1
    tag = save.data(2)
    blocks = [(1, 3, 1, save.data(1)), (2, 2, 2, tag), (3, 0, 3, save.data(3)), (4, 0, 3, b"")]
    blocks += [(5 + k, 0, 4 + k, save.data(4 + k)) for k in range(5)]
    blocks += [(2, 1, 2, tag), (2, 0, 2, tag), (1, 3, 1, save.data(1))]
    save.write([save.sectors[0]], blocks)


def v_file0(flags, copy, damage):
    def make(save):
        save.sizes[0], save.flags[0] = 0x100, flags
        blocks = save.default_blocks()
        if copy:
            blocks[8] = (0, 0, 0, b"")
        save.write([save.sectors[0]], blocks, block0=(1 if copy else 0, 0, b""))
        if damage:
            o = save.chain[8] * BLOCK + 0x100
            save.card[o:o + 16] = bytes(x ^ 0x5A for x in save.card[o:o + 16])
    return make


def nosave(template):
    card = bytearray(template)
    d = copy_active(card, DIR, seal_dir)
    b = copy_active(card, BAT, seal_bat)
    o = melee_entry(card, d)
    bo = b * BLOCK
    blk, = struct.unpack_from(">H", card, o + 0x36)
    while blk != 0xFFFF:
        nxt, = struct.unpack_from(">H", card, bo + 0xA + 2 * (blk - 5))
        struct.pack_into(">H", card, bo + 0xA + 2 * (blk - 5), 0)
        free, = struct.unpack_from(">H", card, bo + 6)
        struct.pack_into(">H", card, bo + 6, free + 1)
        blk = nxt
    card[o:o + ENTRY] = b"\xff" * ENTRY
    for blk in DIR:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[d * BLOCK:(d + 1) * BLOCK]
        seal_dir(card, blk)
    for blk in BAT:
        card[blk * BLOCK:(blk + 1) * BLOCK] = card[b * BLOCK:(b + 1) * BLOCK]
        seal_bat(card, blk)
    return card


def crafted(make):
    def run(template):
        save = Save(template)
        make(save)
        return save.sealed()
    return run


SAVES = (
    ("v1", crafted(v_mirror)), ("v2", crafted(v_third)),
    ("v3a", crafted(v_banner(1))), ("v3b", crafted(v_banner(0))), ("v4", crafted(v_icons)),
    ("v5", crafted(v_flags3(None))), ("v5a", crafted(v_flags3((0, 0xFF)))),
    ("v5b", crafted(v_flags3((0xFF, 0)))), ("v5c", crafted(v_flags3((0x10, 0xF0)))),
    ("v5d", crafted(v_flags3((0xF0, 0x10)))), ("v6", crafted(v_twoblocks)),
    ("v7", crafted(v_excess)), ("v8a", crafted(v_file0(1, False, False))),
    ("v8b", crafted(v_file0(0, False, False))), ("v8c", crafted(v_file0(0, False, True))),
    ("v8d", crafted(v_file0(0, True, False))), ("v8e", crafted(v_file0(0, True, True))),
    ("v9", crafted(v_file0(3, True, False))), ("nosave", nosave),
)


def main():
    snaps, saves = sys.argv[1] == "--snaps", sys.argv[1] == "--saves"
    args = sys.argv[2:] if snaps or saves else sys.argv[1:]
    template = open(args[0], "rb").read()
    os.makedirs(args[1], exist_ok=True)
    makes = (("full", full), ("badicon", badicon))
    if saves:
        makes = SAVES
    if snaps:
        makes = (("snap5", lambda t: keep(t, 5, False)), ("snap1", lambda t: keep(t, 1, False)),
                 ("snapdamaged", lambda t: keep(t, None, True)),
                 ("snap1damaged", lambda t: keep(t, 1, True)))
    for name, make in makes:
        open(os.path.join(args[1], name + ".raw"), "wb").write(make(template))
        print(os.path.join(args[1], name + ".raw"))


if __name__ == "__main__":
    main()
