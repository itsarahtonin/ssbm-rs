"""Draws the coverage map as a static SVG treemap for the README, from the published snapshot
(docs/progress/data/coverage.json): every unit a tile sized by its countable blocks and colored
green for the share of them lockstep verified and amber for the rest (gray if none is verified,
a magenta edge for an unreviewed mismatch), grouped by area, under a line with the totals. Writes a light and a dark version.

    python tools/lockstep/campaign/coverage_svg.py [SNAPSHOT.json] [OUT_PREFIX]

OUT_PREFIX defaults to docs/progress/coverage, giving coverage-light.svg and coverage-dark.svg.
"""
import collections
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WIDTH, HEIGHT, TOP = 960, 400, 44
THEMES = {
    "light": {"bg": "#f6f5f1", "fg": "#1d1f24", "muted": "#646a74", "ok": "#3f9a5c",
              "lo": "#d9a23a", "never": "#c9c7c0", "edge": "#f6f5f1", "review": "#b83a8e"},
    "dark": {"bg": "#15171b", "fg": "#e7e8ea", "muted": "#9aa0a9", "ok": "#4fae6c",
             "lo": "#d6a443", "never": "#3a3e45", "edge": "#15171b", "review": "#d356ab"},
}


def squarify(items, x, y, w, h):
    """Bruls's squarified layout: (item, x, y, w, h) for each (weight, item), largest first."""
    items = sorted(items, key=lambda i: -i[0])
    total = sum(i[0] for i in items)
    out = []
    while items:
        scale = w * h / total
        short = min(w, h)
        row, best = [], float("inf")
        for item in items:
            trial = row + [item]
            areas = [i[0] * scale for i in trial]
            s = sum(areas)
            worst = max(max(short * short * a / (s * s), s * s / (short * short * a)) for a in areas)
            if worst > best:
                break
            row, best = trial, worst
        s = sum(i[0] for i in row) * scale
        if w >= h:
            col = s / h
            yy = y
            for weight, item in row:
                hh = weight * scale / col
                out.append((item, x, yy, col, hh))
                yy += hh
            x, w = x + col, w - col
        else:
            rowh = s / w
            xx = x
            for weight, item in row:
                ww = weight * scale / rowh
                out.append((item, xx, y, ww, rowh))
                xx += ww
            y, h = y + rowh, h - rowh
        total -= sum(i[0] for i in row)
        items = items[len(row):]
    return out


def main():
    source = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs/progress/data/coverage.json"
    prefix = sys.argv[2] if len(sys.argv) > 2 else str(ROOT / "docs/progress/coverage")
    data = json.loads(source.read_text(encoding="utf-8"))
    units = data["units"]
    stats = collections.defaultdict(lambda: [0, 0, False])  # countable, verified, open review
    for name, unit, countable, verified, _calls, mism, mutated, reviewed, *_ in data["fns"]:
        s = stats[units[unit]]
        s[0] += countable
        s[1] += verified
        s[2] |= bool(mism) or (bool(mutated) and not reviewed)
    areas = collections.defaultdict(list)
    for unit, (countable, verified, review) in stats.items():
        if countable:
            areas["/".join(unit.split("/")[:2])].append((countable, (unit, verified / countable, review)))
    countable = sum(s[0] for s in stats.values())
    verified = sum(s[1] for s in stats.values())
    label = data["snaps"][-1]["label"]
    caption = (f"{verified:,} of {countable:,} blocks verified by lockstep "
               f"({100 * verified / countable:.1f}%) · {label}, {data['updated'][:10]}")
    area_tiles = squarify([(sum(c for c, _ in us), a) for a, us in areas.items()],
                          0, TOP, WIDTH, HEIGHT - TOP)
    for theme, c in THEMES.items():
        parts = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}" '
                 f'width="{WIDTH}" height="{HEIGHT}" font-family="system-ui, -apple-system, Segoe UI, sans-serif">',
                 f'<title>ssbm-rs port coverage map: {caption}</title>',
                 f'<rect width="{WIDTH}" height="{HEIGHT}" fill="{c["bg"]}"/>',
                 f'<text x="2" y="18" font-size="15" font-weight="600" fill="{c["fg"]}">Port coverage map</text>',
                 f'<text x="2" y="36" font-size="13" fill="{c["muted"]}">{caption}</text>']
        for area, ax, ay, aw, ah in area_tiles:
            for (unit, share, review), x, y, w, h in squarify(areas[area], ax, ay, aw, ah):
                # The unit's verified share fills its tile in green from the left (or top), the
                # rest in the open blocks' color: the picture reads at the measured ratio.
                parts.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" '
                             f'fill="{c["never"] if share == 0 else c["lo"]}"/>')
                if share > 0:
                    gw, gh = (w * share, h) if w >= h else (w, h * share)
                    parts.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{gw:.1f}" height="{gh:.1f}" '
                                 f'fill="{c["ok"]}"/>')
                parts.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" '
                             f'fill="none" stroke="{c["review"] if review else c["edge"]}" '
                             f'stroke-width="{1.5 if review else 0.5}"/>')
            parts.append(f'<rect x="{ax:.1f}" y="{ay:.1f}" width="{aw:.1f}" height="{ah:.1f}" '
                         f'fill="none" stroke="{c["edge"]}" stroke-width="2"/>')
            if aw > 70 and ah > 22:
                parts.append(f'<text x="{ax + 5:.1f}" y="{ay + 15:.1f}" font-size="11" font-weight="600" '
                             f'fill="#1d1f24" fill-opacity="0.85">{area}</text>')
        parts.append("</svg>")
        Path(f"{prefix}-{theme}.svg").write_text("\n".join(parts) + "\n", encoding="utf-8", newline="\n")
    print(caption)


if __name__ == "__main__":
    main()
