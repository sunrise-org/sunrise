"""tools/pack_plan_trace.py's report reading: what fails a pack, its table rows, the cases a run reached, and its
paths and pack list."""

import os
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import pack_plan_trace as tool  # noqa: E402


def sized(blocks='ok', edits='ok', errors=(), guards=(), size=(854, 480)):
    return {
        'size': list(size),
        'frames': [{'seq': 1, 'state': 0, 'blocks': 'ok', 'errors': [], 'guards': []},
                   {'seq': 2, 'state': 0, 'blocks': blocks, 'errors': list(errors), 'guards': list(guards)}],
        'states': [{'passes': 7, 'clears': 3, 'blocks': 2, 'edits': edits}],
        'allocated': 9000, 'plannable': 600, 'dead_bytes': 512, 'removed': [], 'folded': [], 'dead': [],
        'savings': {'events': 1, 'clears_folded': 2, 'clears_dropped': 3, 'bytes': 4096},
    }


def report(*unresolved, narrowed=None):
    return {'refused': [], 'unresolved': {'sizes': list(unresolved)},
            'narrowed': {'sizes': list(narrowed if narrowed is not None else unresolved)}}


class Failures(unittest.TestCase):
    def test_a_clean_report_fails_nothing(self):
        self.assertEqual(tool.failures(report(sized())), [])

    def test_each_failed_check_is_named(self):
        bad = report(sized(blocks='Outside { block: 1 }'),
                     narrowed=[sized(edits='Differs(3)', errors=['a write'], size=(427, 240))])
        bad['refused'] = ['frame-1.trace: bad']
        self.assertEqual(tool.failures(bad), [
            'refused frame-1.trace: bad',
            'unresolved: frame 2: blocks Outside { block: 1 }',
            'narrowed: frame 2: a write',
            'narrowed: 427x240 state 0: edits Differs(3)',
        ])

    def test_a_run_without_frames_fails(self):
        self.assertEqual(tool.failures({'error': 'no frame was recorded'}), ['no frame was recorded'])


class Rows(unittest.TestCase):
    def test_a_row_per_variant_and_size_with_its_numbers(self):
        rows = tool.row('Pack', report(sized(), sized(size=(427, 240)), narrowed=[sized(blocks='Outside')]))
        self.assertEqual(rows, [
            '| Pack | unresolved | 854×480 | 2/1 | ok | 7 | 3 | 1 | 2 | 3 | 512 of 600 | 9,000 |',
            '| Pack | unresolved | 427×240 | 2/1 | ok | 7 | 3 | 1 | 2 | 3 | 512 of 600 | 9,000 |',
            '| Pack | narrowed | 854×480 | 2/1 | FAILED | 7 | 3 | 1 | 2 | 3 | 512 of 600 | 9,000 |',
        ])

    def test_a_variant_that_could_not_be_measured_shows_dashes(self):
        rows = tool.row('Pack', {'unresolved': {'error': 'x'}, 'narrowed': {'error': 'x'}})
        self.assertEqual(rows[0], '| Pack | unresolved | – | – | – | – | – | – | – | – | – | – |')


class Cases(unittest.TestCase):
    def test_a_case_is_reached_by_its_guard_value_in_any_frame_of_any_pack(self):
        one = report(sized(guards=[['shadow cache hit', True], ['mod draws', False]]))
        two = report(sized(guards=[['a world callback drew', False],
                                   ['first deferred size-skipped, a later one ran', True]]))
        self.assertEqual(tool.reached([one, two]), {'shadow-cache-hit', 'mod-draws-absent', 'callback-drew-nothing',
                                                    'deferred-skipped-then-ran'})
        self.assertEqual(tool.reached([report(sized(guards=[['shadow cache hit', False]]))]), set())


class Paths(unittest.TestCase):
    def test_relative_paths_are_made_absolute_from_the_callers_folder(self):
        with tempfile.TemporaryDirectory() as folder:
            here = os.getcwd()
            os.chdir(folder)
            try:
                args = tool.resolved(tool.argparse.Namespace(packs='p', bin_dir=Path('b'), out=Path('o/x'),
                                                             resources=Path('../r')))
            finally:
                os.chdir(here)
            base = Path(folder).resolve()
            self.assertEqual((args.packs, args.bin_dir, args.out, args.resources),
                             (base / 'p', base / 'b', base / 'o/x', base.parent / 'r'))


class Packs(unittest.TestCase):
    def test_zips_and_folders_then_the_fixtures(self):
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder)
            (base / 'b.zip').write_bytes(b'')
            (base / 'a').mkdir()
            (base / 'notes.txt').write_text('not a pack')
            found = tool.packs(base, True)
        self.assertEqual([p.name for p in found[:2]], ['a', 'b.zip'])
        self.assertEqual([p.name for p in found[2:]],
                         ['frame_identity_terrain', 'frame_identity_world', 'frame_plan_cases'])


if __name__ == '__main__':
    unittest.main()
