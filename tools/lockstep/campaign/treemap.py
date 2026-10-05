"""Write a coverage map from saved reports, or regenerate a portable JSON snapshot."""
import argparse
import collections
import datetime
import json
from pathlib import Path

from reports import history_label, history_paths, identity, read_report, render

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def build_data(state, label, reviewed_file, updated=None):
    state = Path(state)
    latest = read_report(state / 'report.csv')
    reviewed = set()
    for line in Path(reviewed_file).read_text(encoding='utf-8').splitlines():
        words = line.split('#')[0].split()
        if words:
            reviewed.add(words[-1] if words[0].startswith('0x') else words[0])
    names = collections.Counter(row['name'] for row in latest)
    ambiguous = [name for name in reviewed if names[name] > 1]
    if ambiguous:
        raise ValueError(f'Qualify reviewed functions with UNIT:NAME: {ambiguous}')

    def is_reviewed(row):
        return row['name'] in reviewed or f"{row['unit']}:{row['name']}" in reviewed

    units = list(dict.fromkeys(row['unit'] for row in latest))
    unit_index = {unit: i for i, unit in enumerate(units)}
    order = {identity(row): i for i, row in enumerate(latest)}
    rows = [[row['name'], unit_index[row['unit']], int(row['countable']),
             int(row['verified_blocks']), int(row['calls']), int(row['mismatches']),
             int(row['mutated']), int(is_reviewed(row)), int(row.get('explained') or 0),
             row['address']] for row in latest]

    def snapshot(name, report):
        values = [0] * (3 * len(rows))
        counts = [0] * len(rows)
        for row in report:
            key = identity(row)
            if key not in order:
                raise ValueError(f'{name}: historical function absent from latest: {key}')
            i = order[key]
            flags = (1 if int(row['calls']) > 0 else 0) | (2 if int(row['mismatches']) > 0 else 0) \
                | (4 if int(row['mutated']) > 0 and not is_reviewed(row) else 0)
            values[3 * i:3 * i + 3] = [int(row['verified_blocks']), int(row.get('explained') or 0), flags]
            counts[i] = int(row['countable'])
        if len(report) != len(rows):
            raise ValueError(f'{name}: changing function population needs an explicit migration')
        result = {'label': name, 'v': values}
        if counts != [row[2] for row in rows]:
            result['countable'] = counts
        return result

    snaps = [snapshot(history_label(path), read_report(path)) for path in history_paths(state)
             if history_label(path) != label]
    snaps.append(snapshot(label, latest))
    return {'units': units, 'fns': rows, 'snaps': snaps, 'bar': 0.9,
            'updated': updated or datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M UTC')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('label')
    parser.add_argument('out_html', type=Path)
    parser.add_argument('--state-dir', type=Path, default=ROOT / 'local/lockstep')
    parser.add_argument('--reviewed', type=Path, default=ROOT / 'tools/lockstep/reviewed.txt')
    parser.add_argument('--from-json', type=Path)
    parser.add_argument('--json', type=Path)
    args = parser.parse_args()
    data = json.loads(args.from_json.read_text(encoding='utf-8')) if args.from_json else \
        build_data(args.state_dir, args.label, args.reviewed)
    args.out_html.write_text(render(HERE / 'treemap.template.html', data), encoding='utf-8', newline='\n')
    if args.json:
        args.json.write_text(json.dumps(data, separators=(',', ':'), allow_nan=False) + '\n', encoding='utf-8')
    print(f"{len(data['fns'])} functions, {len(data['snaps'])} rounds -> {args.out_html}")


if __name__ == '__main__':
    main()
