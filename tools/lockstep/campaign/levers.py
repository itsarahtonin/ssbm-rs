"""Write coverage-lever metrics and naturally ordered saved history."""
import argparse
import csv
import datetime
import json
import io
import re
from pathlib import Path

from reports import history_label, history_paths, identity, read_report, render, totals

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def words(path):
    data = Path(path).read_bytes()
    return [int.from_bytes(data[i:i + 8], 'little') for i in range(0, len(data), 8)]


def cost(path):
    if not path.exists():
        return None
    matches = re.findall(r'(\d+) fields, (\d+) M instructions', path.read_text(encoding='utf-8', errors='replace'))
    return int(matches[-1][1]) / 1000 if matches else None


def build_data(state, base_label, updated=None):
    state = Path(state)
    runs = []
    for row in csv.reader(io.StringIO((state / 'levers.tsv').read_text(encoding='utf-8')), delimiter='\t'):
        if not row or row[0].startswith('#'):
            continue
        date, lever, run, known, results, collect, note = (row + [''] * 7)[:7]
        bitmap = state / (results + '.bin')
        spent = cost(state / (results + '.txt'))
        if not bitmap.exists() or spent is None:
            continue
        actual, baseline = words(bitmap), words(state / known)
        new = sum((value & ~(baseline[i] if i < len(baseline) else 0)).bit_count() for i, value in enumerate(actual))
        collected = cost(state / (collect + '.txt')) if collect else None
        runs.append({'date': date, 'lever': lever, 'run': run, 'new': new,
                     'cost': spent + (collected or 0),
                     'collect': collected, 'note': note})
    timeline, base_rows = [], None
    for path in history_paths(state):
        label = history_label(path)
        rows = read_report(path)
        timeline.append({'label': label, **totals(rows)})
        if label == base_label:
            base_rows = rows
    if base_rows is None:
        raise ValueError(f'Unknown baseline round: {base_label}')
    latest = read_report(state / 'report.csv')
    now = totals(latest)
    if not timeline or any(timeline[-1][key] != value for key, value in now.items()):
        timeline.append({'label': 'working report', **now})
    before = {identity(row): int(row['verified_blocks']) + int(row.get('explained') or 0) for row in base_rows}
    gains = []
    for row in latest:
        gain = int(row['verified_blocks']) + int(row.get('explained') or 0) - before.get(identity(row), 0)
        if gain > 0:
            gains.append({'name': row['name'], 'unit': row['unit'], 'gained': gain,
                          'open': int(row['countable']) - int(row['verified_blocks']) - int(row.get('explained') or 0)})
    gains.sort(key=lambda row: -row['gained'])
    notes = [{'date': row[0], 'text': row[1]} for row in
             csv.reader(io.StringIO((state / 'levers-notes.tsv').read_text(encoding='utf-8')), delimiter='\t')
             if row and not row[0].startswith('#') and len(row) > 1]
    return {'updated': updated or datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M UTC'),
            'base': base_label, 'baseTotals': totals(base_rows), 'now': now, 'timeline': timeline,
            'runs': runs, 'gains': gains[:20], 'gainedFunctions': len(gains), 'notes': notes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base')
    parser.add_argument('out_html', type=Path)
    parser.add_argument('--state-dir', type=Path, default=ROOT / 'local/lockstep')
    parser.add_argument('--from-json', type=Path)
    parser.add_argument('--json', type=Path)
    args = parser.parse_args()
    data = json.loads(args.from_json.read_text(encoding='utf-8')) if args.from_json else build_data(args.state_dir, args.base)
    args.out_html.write_text(render(HERE / 'levers.template.html', data), encoding='utf-8', newline='\n')
    if args.json:
        args.json.write_text(json.dumps(data, separators=(',', ':'), allow_nan=False) + '\n', encoding='utf-8')
    print(f"{len(data['runs'])} runs, {len(data['timeline'])} timeline entries -> {args.out_html}")


if __name__ == '__main__':
    main()
