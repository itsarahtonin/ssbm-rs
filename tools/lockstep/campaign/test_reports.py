import csv
import tempfile
import unittest
from pathlib import Path

import levers
import reports
import treemap
import evidence

FIELDS = 'address name unit blocks countable verified_blocks calls mismatches mutated explained'.split()


def row(unit='a', address='0x10', name='OnReset', countable=10, verified=0, explained=0, **extra):
    return dict(address=address, name=name, unit=unit, blocks=countable, countable=countable,
                verified_blocks=verified, calls=1, mismatches=0, mutated=0, explained=explained, **extra)


def write(path, rows):
    with path.open('w', newline='', encoding='utf-8') as file:
        writer = csv.DictWriter(file, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(rows)


class ReportsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.state = Path(self.temp.name)
        (self.state / 'history').mkdir()
        self.reviewed = self.state / 'reviewed.txt'
        self.reviewed.write_text('')

    def test_history_matches_unit_and_address_despite_duplicate_names(self):
        a, b = row('a', verified=9), row('b', verified=2)
        write(self.state / 'report.csv', [a, b])
        write(self.state / 'history/01-f1.csv', [b, a])
        data = treemap.build_data(self.state, 'latest', self.reviewed)
        self.assertEqual(data['snaps'][0]['v'], [9, 0, 1, 2, 0, 1])

    def test_changed_historical_denominator_is_preserved(self):
        write(self.state / 'report.csv', [row(countable=10, verified=5)])
        write(self.state / 'history/01-f1.csv', [row(countable=5, verified=5)])
        data = treemap.build_data(self.state, 'latest', self.reviewed)
        self.assertEqual(data['snaps'][0]['countable'], [5])
        self.assertEqual(data['snaps'][0]['blocks'], [5])
        self.assertEqual(data['fns'][0][10], 10)

    def test_ambiguous_review_requires_unit(self):
        a, b = row('a'), row('b')
        a['mutated'] = b['mutated'] = 1
        write(self.state / 'report.csv', [a, b])
        self.reviewed.write_text('OnReset # evidence\n')
        with self.assertRaises(ValueError):
            treemap.build_data(self.state, 'latest', self.reviewed)
        self.reviewed.write_text('a:OnReset # evidence\n')
        data = treemap.build_data(self.state, 'latest', self.reviewed)
        self.assertEqual([r[7] for r in data['fns']], [1, 0])
        self.assertEqual(data['snaps'][-1]['v'][2::3], [1, 5])

    def test_natural_round_order_including_closing_snapshot(self):
        paths = ['28-f7.10.csv', '28-f7.csv', '28-f7.9.csv']
        self.assertEqual(sorted(paths, key=reports.history_key),
                         ['28-f7.9.csv', '28-f7.10.csv', '28-f7.csv'])

    def test_invalid_and_duplicate_evidence_fails(self):
        path = self.state / 'report.csv'
        write(path, [row(verified=8, explained=3)])
        with self.assertRaises(ValueError):
            reports.read_report(path)
        invalid = row()
        invalid['blocks'] = 9
        write(path, [invalid])
        with self.assertRaises(ValueError):
            reports.read_report(path)
        write(path, [row(), row()])
        with self.assertRaises(ValueError):
            reports.read_report(path)

    def test_unknown_historical_function_is_not_silently_dropped(self):
        write(self.state / 'report.csv', [row()])
        write(self.state / 'history/01-f1.csv', [row('other')])
        with self.assertRaises(ValueError):
            treemap.build_data(self.state, 'latest', self.reviewed)

    def test_lever_gains_keep_same_named_functions_separate(self):
        (self.state / 'levers.tsv').write_text('')
        (self.state / 'levers-notes.tsv').write_text('')
        write(self.state / 'history/01-f1.csv', [row('a', verified=10), row('b')])
        write(self.state / 'report.csv', [row('a', verified=10), row('b', verified=3)])
        data = levers.build_data(self.state, 'f1')
        self.assertEqual(data['gainedFunctions'], 1)
        self.assertEqual(data['gains'][0]['unit'], 'b')
        self.assertEqual(data['gains'][0]['gained'], 3)
        with self.assertRaises(ValueError):
            levers.build_data(self.state, 'missing')

    def test_cost_uses_last_completed_run_summary(self):
        path = self.state / 'run.txt'
        path.write_text('2 fields, 3 M instructions\n10 fields, 8 M instructions\n')
        self.assertEqual(levers.cost(path), 0.008)

    def test_small_run_cost_is_not_rounded_to_zero(self):
        (self.state / 'levers.tsv').write_text('today\tfuzz\trun\tknown.bin\tresult\t\t\n')
        (self.state / 'levers-notes.tsv').write_text('')
        (self.state / 'known.bin').write_bytes(bytes(8))
        (self.state / 'result.bin').write_bytes((1).to_bytes(8, 'little'))
        (self.state / 'result.txt').write_text('2 fields, 3 M instructions\n')
        write(self.state / 'history/01-f1.csv', [row()])
        write(self.state / 'report.csv', [row()])
        run = levers.build_data(self.state, 'f1')['runs'][0]
        self.assertEqual(run['cost'], 0.003)
        self.assertAlmostEqual(run['new'] / run['cost'], 1000 / 3)

    def test_ledger_names_are_qualified_and_reasons_preserved(self):
        path = self.state / 'gaps.txt'
        path.write_text('# Shared call graph rationale\na:OnReset+* fail # <handler> & caller\n'
                        'b:OnReset+0x4 dead # no path\nExcluded+* hw # substituted\n')
        data = evidence.ledger(path, [['OnReset', 0], ['OnReset', 1]], ['a', 'b'])
        self.assertEqual(data['byFunction'], {'0': [0], '1': [1]})
        self.assertEqual(len(data['entries']), 3)
        self.assertEqual(data['entries'][0]['reason'], '<handler> & caller')
        self.assertEqual(data['groups'][0], 'Shared call graph rationale')
        self.assertEqual(data['entries'][0]['line'], 2)
        self.assertEqual(data['entries'][1]['group'], 0)

    def test_ambiguous_or_invalid_ledger_claim_fails(self):
        path = self.state / 'gaps.txt'
        for text in ('OnReset+* fail', 'a:OnReset+* madeup', 'a:OnReset+wrong dead'):
            path.write_text(text)
            with self.subTest(text=text), self.assertRaises(ValueError):
                evidence.ledger(path, [['OnReset', 0], ['OnReset', 1]], ['a', 'b'])


if __name__ == '__main__':
    unittest.main()
