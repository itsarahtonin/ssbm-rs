"""Merges coverage bitmaps into one, for LOCKSTEP_KNOWN: python known.py OUT DIR..."""
import glob
import sys

merged = 0
for d in sys.argv[2:]:
    for f in glob.glob(f"{d}/*.bin"):
        merged |= int.from_bytes(open(f, "rb").read(), "little")
n = max(1, (merged.bit_length() + 63) // 64)
open(sys.argv[1], "wb").write(merged.to_bytes(8 * n, "little"))
print(f"{bin(merged).count('1')} instructions known -> {sys.argv[1]}")
