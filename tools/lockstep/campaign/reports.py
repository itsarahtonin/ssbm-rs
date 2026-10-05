"""Shared identities, bounds checks, and ordering for saved coverage reports."""
import csv
import re
from pathlib import Path


def identity(row):
    return row['unit'], row['address']


def read_report(path):
    with Path(path).open(encoding='utf-8') as file:
        rows = list(csv.DictReader(file))
    seen = set()
    for row in rows:
        key = identity(row)
        if key in seen:
            raise ValueError(f'{path}: duplicate function {key}')
        seen.add(key)
        countable, verified, explained = (int(row.get(k) or 0) for k in
                                          ('countable', 'verified_blocks', 'explained'))
        if not (0 <= verified <= countable and 0 <= explained <= countable - verified):
            raise ValueError(f'{path}: inconsistent coverage for {key}')
        for k in ('calls', 'mismatches', 'mutated'):
            if int(row.get(k) or 0) < 0:
                raise ValueError(f'{path}: negative {k} for {key}')
    return rows


def history_label(path):
    return Path(path).stem.split('-', 1)[1].replace('_', ' ')


def history_key(path):
    prefix, label = Path(path).stem.split('-', 1)
    match = re.fullmatch(r'f(\d+)(?:\.(\d+))?', label)
    if match:
        return int(prefix), 'f', int(match[1]), int(match[2]) if match[2] else float('inf')
    return int(prefix), label, 0, 0


def history_paths(state):
    return sorted((Path(state) / 'history').glob('*.csv'), key=history_key)


def totals(rows):
    result = {'countable': 0, 'verified': 0, 'explained': 0}
    for row in rows:
        for key, column in [('countable', 'countable'), ('verified', 'verified_blocks'),
                            ('explained', 'explained')]:
            result[key] += int(row.get(column) or 0)
    result['open'] = result['countable'] - result['verified'] - result['explained']
    return result


def render(template, data):
    import json
    encoded = json.dumps(data, separators=(',', ':'), allow_nan=False).replace('<', '\\u003c')
    return Path(template).read_text(encoding='utf-8').replace('/*DATA*/null', encoded)
