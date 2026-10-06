"""Draws the coverage summary for the README as a static SVG, from the published snapshot
(docs/progress/data/coverage.json): a bar of all countable blocks, split into those lockstep
verified, those the gap ledger explains and those still open, then the same bar for each area of
the code, largest first, on a dark ground.

    python tools/lockstep/campaign/coverage_svg.py [SNAPSHOT.json] [OUT.svg]

OUT defaults to docs/progress/coverage.svg.
"""
import collections
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WIDTH = 960
COLORS = {"bg": "#15171b", "fg": "#e7e8ea", "muted": "#9aa0a9", "ok": "#4fae6c",
          "explained": "#73a9da", "open": "#d6a443", "track": "#2a2e35"}
# Plain names for the areas, by their first two path components.
NAMES = {
    "melee/ft": "Fighters", "melee/it": "Items", "melee/gr": "Stages", "melee/gm": "Game modes",
    "sysdolphin/baselib": "HAL's engine", "melee/mn": "Menus", "melee/lb": "Shared library",
    "melee/ty": "Trophies", "melee/mp": "Collision", "melee/pl": "Player records",
    "melee/if": "Interface", "melee/cm": "Camera", "melee/ef": "Effects", "melee/vi": "Cutscenes",
}
ROWS = 12  # areas drawn one by one; the rest share a row


def bar(parts, x, y, w, h, c):
    """A bar of `parts` (share, color) from the left, on a track."""
    out = [f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="3" fill="{c["track"]}"/>']
    at = x
    for share, color in parts:
        width = w * share
        if width > 0:
            out.append(f'<rect x="{at:.1f}" y="{y}" width="{width:.1f}" height="{h}" fill="{color}"/>')
        at += width
    return out


def main():
    source = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs/progress/data/coverage.json"
    target = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "docs/progress/coverage.svg"
    data = json.loads(source.read_text(encoding="utf-8"))
    units = data["units"]
    areas = collections.defaultdict(lambda: [0, 0, 0])  # countable, verified, explained
    for _name, unit, countable, verified, _calls, _mism, _mut, _rev, explained, *_ in data["fns"]:
        a = areas["/".join(units[unit].split("/")[:2])]
        a[0] += countable
        a[1] += verified
        a[2] += explained
    ranked = sorted(areas.items(), key=lambda kv: -kv[1][0])
    rest = [sum(v[i] for _, v in ranked[ROWS:]) for i in range(3)]
    rows = [(NAMES.get(a, a), a, v) for a, v in ranked[:ROWS]]
    rows.append(("Everything else", f"{len(ranked) - ROWS} more areas, the SDK's among them", rest))
    total = [sum(v[i] for v in areas.values()) for i in range(3)]
    label = data["snaps"][-1]["label"]
    date = data["updated"][:10]
    top, row_h = 132, 30
    height = top + row_h * len(rows) + 12
    c = COLORS

    def parts(v):
        return [(v[1] / v[0], c["ok"]), (v[2] / v[0], c["explained"]),
                (1 - (v[1] + v[2]) / v[0], c["open"])]

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {height}" width="{WIDTH}" '
           f'height="{height}" font-family="system-ui, -apple-system, Segoe UI, sans-serif">',
           f'<title>ssbm-rs port coverage: {total[1]:,} of {total[0]:,} blocks verified by lockstep '
           f'({100 * total[1] / total[0]:.1f}%), round {label}, {date}</title>',
           f'<rect width="{WIDTH}" height="{height}" fill="{c["bg"]}"/>',
           f'<text x="16" y="30" font-size="18" font-weight="600" fill="{c["fg"]}">Port coverage</text>',
           f'<text x="{WIDTH - 16}" y="30" font-size="13" text-anchor="end" fill="{c["muted"]}">'
           f'round {label} · {date}</text>']
    out += bar(parts(total), 16, 44, WIDTH - 32, 26, c)
    shares = [total[1] / total[0], total[2] / total[0], 1 - (total[1] + total[2]) / total[0]]
    x = 16
    for (text, share), color in zip((("verified by lockstep", shares[0]),
                                     ("explained by the gap ledger", shares[1]),
                                     ("open", shares[2])), (c["ok"], c["explained"], c["open"])):
        out.append(f'<rect x="{x}" y="84" width="12" height="12" rx="2" fill="{color}"/>')
        item = f'{100 * share:.1f}% {text}'
        out.append(f'<text x="{x + 18}" y="95" font-size="14" fill="{c["fg"]}">{item}</text>')
        x += 18 + 7.6 * len(item) + 28
    out.append(f'<text x="{WIDTH - 16}" y="95" font-size="13" text-anchor="end" fill="{c["muted"]}">'
               f'{total[0]:,} blocks</text>')
    out.append(f'<line x1="16" y1="114" x2="{WIDTH - 16}" y2="114" stroke="{c["track"]}"/>')
    for i, (name, path, v) in enumerate(rows):
        y = top + i * row_h
        out.append(f'<text x="16" y="{y + 14}" font-size="13" fill="{c["fg"]}">{name}</text>')
        out.append(f'<text x="16" y="{y + 26}" font-size="10" fill="{c["muted"]}">{path}</text>')
        out += bar(parts(v), 230, y + 4, 560, 14, c)
        out.append(f'<text x="{WIDTH - 16}" y="{y + 16}" font-size="13" text-anchor="end" fill="{c["fg"]}">'
                   f'{100 * v[1] / v[0]:.1f}%<tspan fill="{c["muted"]}" font-size="11"> of {v[0]:,}</tspan></text>')
    out.append("</svg>")
    target.write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")
    print(f"{total[1]:,} of {total[0]:,} blocks verified ({100 * total[1] / total[0]:.1f}%), "
          f"{total[2]:,} explained, round {label}")


if __name__ == "__main__":
    main()
