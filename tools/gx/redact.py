"""Covers the HUD name boxes of the other players in the progress page's frame images, so a page
published with them doesn't show opponents' names: for each smoke replay, the ports whose Slippi
connect code isn't KEEP (read from the replay's Game Start event) get their box under the damage
percent filled in, in the run's three images in IMG_DIR (progress.py's img/).

    python tools/gx/redact.py IMG_DIR KEEP

KEEP is a connect code as Slippi shows it, such as ABCD#123. The box positions are those of a
640x480 frame, measured for ports 1 and 2, the only ones the smoke set uses; a replay with a
player on another port stops the script.
"""
import os
import re
import struct
import sys

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
# Ports 1 and 2's name boxes: centered under the damage percent, as wide as the name, rows 451-465.
CENTERS = (100, 252)
HALF_WIDTH, TOP, BOTTOM = 76, 448, 468


def game_start(path):
    """The Game Start event, from its command byte, of the replay at `path`."""
    data = open(path, "rb").read()
    i = data.index(b"raw[$U#l") + len(b"raw[$U#l") + 4
    assert data[i] == 0x35, path
    size = data[i + 1]
    sizes = {data[j]: struct.unpack(">H", data[j + 1:j + 3])[0] for j in range(i + 2, i + 1 + size, 3)}
    j = i + 1 + size
    assert data[j] == 0x36, path
    return data[j:j + 1 + sizes[0x36]]


def others(path, keep):
    """The ports, 0 to 3, that have a player whose connect code isn't `keep`."""
    g = game_start(path)
    out = []
    for p in range(4):
        if g[0x66 + 0x24 * p] == 3:  # no player
            continue
        code = g[0x221 + 0x0A * p:0x22B + 0x0A * p].split(b"\x00")[0].decode("shift_jis", "replace")
        if code.replace("＃", "#") != keep:
            out.append(p)
    return out


def replay_paths():
    """Each smoke replay by name, found under REPLAYS (local/env.sh)."""
    env = open(os.path.join(ROOT, "local", "env.sh"), encoding="utf-8").read()
    root = re.search(r'REPLAYS="([^"]+)"', env).group(1)
    found = {f.removesuffix(".slp"): os.path.join(r, f) for r, _, fs in os.walk(root) for f in fs if f.endswith(".slp")}
    names = []
    for line in open(os.path.join(ROOT, "local", "replays", "smoke-set.txt"), encoding="utf-8"):
        line = line.strip()
        if line and not line.startswith("#"):
            names.append(os.path.basename(line).removesuffix(".slp"))
    return {n: found[n] for n in names}


def main():
    img_dir, keep = sys.argv[1], sys.argv[2]
    covered = 0
    for name, path in replay_paths().items():
        ports = others(path, keep)
        for kind in ("ours", "dolphin", "diff"):
            image = os.path.join(img_dir, f"{name}-{kind}.png")
            if not os.path.exists(image):
                continue
            im = Image.open(image).convert("RGB")
            assert im.size == (640, 480), image
            draw = ImageDraw.Draw(im)
            for p in ports:
                assert p < len(CENTERS), f"{name}: no measured name box for port {p + 1}"
                draw.rectangle((CENTERS[p] - HALF_WIDTH, TOP, CENTERS[p] + HALF_WIDTH, BOTTOM), fill=(0, 0, 0))
                covered += 1
            im.save(image)
    print(f"{covered} name boxes covered")


if __name__ == "__main__":
    main()
