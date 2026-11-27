"""Synthetic failure payloads only; no game or prepared-data execution."""
import contextlib
import ctypes
import io
import os
import struct
import subprocess
import time
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

from tools.ci import ignored_tests as ci


class Budgets(unittest.TestCase):
    def render(self, output):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics(output)
        return log.getvalue()

    def test_public_client_excerpt_preserves_reason_despite_all_withheld_counter(self):
        # Public job 130479644435, lines 326–330 (timestamps removed).
        # Raw panic locations/payloads are unavailable; input below is synthetic,
        # not a reconstruction of private output.
        fixture = Path(__file__).with_name('fixtures') / 'client_failure_public_excerpt.txt'
        excerpt = fixture.read_text()
        name = excerpt.splitlines()[0].removeprefix('  failed test: ')
        log = self.render(f"test {name} ... FAILED\n"
                          f"thread '{name}' panicked at private.rs:1:1:\n"
                          "assertion `left == right` failed: PRIVATE\n"
                          ' left: "PRIVATE"\n right: "PRIVATE"\n')
        self.assertEqual(log.split('failure diagnostics: ')[0],
                         '\n'.join(excerpt.splitlines()[:3]) + '\n')
        self.assertIn('1 withheld context/identity', log)
        self.assertIn('1 classified panic reasons; 0 identities withheld; 1 locations withheld; '
                      '1 payloads withheld (expected privacy filtering)', log)
        self.assertNotIn('PRIVATE', log)

    def test_generated_absolute_location_uses_only_this_checkout_root(self):
        root = Path(ci.__file__).resolve().parents[2]
        path = 'crates/sunrise-client/src/cut_layout/game/administration/commands/incoming_tests/knot_identity.rs'
        location = f'{root / path}:176:1'
        log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {location}:\n"
                          "assertion failed: PRIVATE\n")
        self.assertIn(f'panicked at {path}:176:1', log)
        self.assertNotIn(str(root), log)
        self.assertNotIn('PRIVATE', log)
        for rejected in (f'/another-checkout/{path}:176:1',
                         f'{root}-other/{path}:176:1',
                         f'{root}/../{root.name}/{path}:176:1',
                         f'{root}/{path}:999999:1',
                         f'{root}/{path}:176:0',
                         f'{root}/crates/sunrise-client/src/not_tracked.rs:1:1',
                         f'{root}/.local/private.rs:1:1'):
            with self.subTest(location=rejected):
                log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {rejected}:\n"
                                  "assertion failed: PRIVATE\n")
                self.assertIn('panic location withheld', log)
                self.assertNotIn('  panicked at ', log)
                self.assertNotIn(str(root), log)

    def test_relative_and_cross_crate_raw_panic_headers(self):
        path = 'crates/sunrise-client/src/title/tests.rs'
        for raw_path in (path, 'crates/sunrise-client-lib/../sunrise-client/src/title/tests.rs'):
            # Keep real path validation, but do not tie this fixture to the contents of line 109.
            with patch.object(ci, 'source_lines', return_value=[''] * 108 + ['assert!(ready);']):
                log = self.render(f"test game::x::y ... FAILED\n"
                                  f"thread 'game::x::y' (12345) panicked at {raw_path}:109:5:\n"
                                  "assertion failed: PRIVATE\n")
            self.assertEqual(log,
                '  failed test: game::x::y\n'
                f'  panicked at {path}:109:5\n'
                '  panic reason: assertion condition failed; payload withheld (resource privacy)\n'
                '  panic source: assert!(ready);\n'
                'failure diagnostics: 1 shown; 0 failing tests not shown (total byte budget 128000); '
                '0 records shortened (per-test byte budget 500); 0 missing context; '
                '1 withheld context/identity; 1 classified panic reasons; 0 identities withheld; '
                '0 locations withheld; 1 payloads withheld (expected privacy filtering); '
                '0 lines skipped (line/count bounds; failure count unknown there); '
                '0 contexts skipped (retention bound)\n')
        log = self.render("test game::x::y ... FAILED\n"
                          "thread 'game::x::y' (12345) panicked at "
                          "crates/sunrise-client/src/game/administration/commands/terrain_cmds/loot_slot_tests.rs:123:9:\n"
                          "assertion failed: PRIVATE\n")
        # Requested example is not a tracked file in this checkout: fail closed.
        self.assertIn('panic location withheld', log)

    def test_payload_filtering_does_not_imply_location_or_identity_loss(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['', '']):
            log = self.render("test safe ... FAILED\n"
                              "thread 'safe' panicked at crates/example.rs:2:3:\n"
                              "called `Option::unwrap()` on a `None` value\n")
        self.assertIn('panicked at crates/example.rs:2:3', log)
        self.assertIn('Option::unwrap on None', log)
        self.assertIn('1 classified panic reasons; 0 identities withheld; 0 locations withheld; '
                      '1 payloads withheld', log)

    def test_a_test_that_catches_its_own_panic_is_shown_by_the_panic_it_failed_on(self):
        source = ['panic!("a listener fails");', '', 'assert_eq!(posts, 0, "aborted pass fired post callbacks");']
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), patch.dict(ci._panic_locations, clear=True):
            log = self.render(
                "test guarded ... FAILED\n\nfailures:\n\n---- guarded stdout ----\n"
                "thread 'guarded' panicked at crates/example.rs:1:1:\na listener fails\n"
                "---- the second phase ----\n"
                "thread 'guarded' panicked at crates/example.rs:3:1:\n"
                "assertion `left == right` failed: aborted pass fired post callbacks\n  left: 1\n right: 0\n"
            )
        self.assertIn('panicked at crates/example.rs:3:1', log)
        self.assertIn('message: aborted pass fired post callbacks', log)
        self.assertNotIn('crates/example.rs:1:1', log)
        self.assertNotIn('a listener fails', log)

    def test_outside_a_failed_tests_capture_the_first_panic_stays(self):
        source = ['assert!(original);', 'panic!("cleanup");']
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), patch.dict(ci._panic_locations, clear=True):
            # A fatal run: no capture header, the process dies after a cleanup panic on the same thread.
            log = self.render(
                "thread 'dying' panicked at crates/example.rs:1:1:\nassertion failed: original\n"
                "thread 'dying' panicked at crates/example.rs:2:1:\ncleanup\n"
                "test dying ... FAILED\n"
            )
        self.assertIn('panicked at crates/example.rs:1:1', log)
        self.assertNotIn('crates/example.rs:2:1', log)

    def test_a_repeated_name_that_panics_twice_later_keeps_the_first_targets_context(self):
        source = ['assert!(first);', 'panic!("a listener fails");', 'assert!(second);']
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), patch.dict(ci._panic_locations, clear=True):
            # Two targets' captures of one name, separated by nothing but the second's header.
            log = self.render(
                "test same ... FAILED\ntest same ... FAILED\n\nfailures:\n\n"
                "---- same stdout ----\nthread 'same' panicked at crates/example.rs:1:1:\nassertion failed: first\n"
                "---- same stdout ----\nthread 'same' panicked at crates/example.rs:2:1:\na listener fails\n"
                "thread 'same' panicked at crates/example.rs:3:1:\nassertion failed: second\n"
            )
        self.assertEqual(log.count('panicked at crates/example.rs:1:1'), 2)
        self.assertNotIn('crates/example.rs:3:1', log)

    CHILD_SOURCE = ['assert_eq!(slots, 1, "child fixture failed");', 'panic!("child test process failed");']
    CHILD_REPORT = ("running 1 test\ntest foo ... FAILED\n\nfailures:\n\n---- foo stdout ----\n"
                    "thread 'foo' panicked at crates/example.rs:1:1:\nchild fixture failed\n\nfailures:\n    foo\n\n"
                    "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n")
    PARENT_START = ("     Running unittests src/lib.rs (target/debug/deps/example)\nrunning 1 test\ntest foo ... FAILED\n"
                    "\nfailures:\n\n---- foo stdout ----\n")

    def render_child(self, output):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=self.CHILD_SOURCE), \
                patch.dict(ci._panic_locations, clear=True):
            log = self.render(output)
            return log, dict(ci._panic_locations)

    def test_a_childs_panic_printed_before_its_parents_wrapper_panic_is_the_one_shown(self):
        log, locations = self.render_child(
            self.PARENT_START + self.CHILD_REPORT +
            "thread 'foo' panicked at crates/example.rs:2:1:\nchild test process failed\n")
        self.assertIn('  panicked at crates/example.rs:1:1\n', log)
        self.assertIn('  panic reason: custom panic; message: child fixture failed\n', log)
        self.assertIn('  panic source: assert_eq!(slots, 1, "child fixture failed");\n', log)
        self.assertNotIn('crates/example.rs:2:1', log)
        self.assertNotIn('child test process failed', log)
        self.assertEqual(log.count('  failed test: foo\n'), 1)
        self.assertEqual(log.count('panicked at crates/example.rs:1:1'), 1)
        self.assertEqual(locations, {'foo': 'crates/example.rs:1:1'})

    def test_a_childs_panic_inside_its_parents_panic_message_replaces_the_wrapper(self):
        child = ("running 1 test\ntest foo ... FAILED\n\nfailures:\n\n---- foo stdout ----\n"
                 "thread 'foo' panicked at crates/example.rs:1:1:\n"
                 "assertion `left == right` failed: PRIVATE 987654\n  left: 987654\n right: 1\n\n"
                 "failures:\n    foo\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n")
        log, locations = self.render_child(
            self.PARENT_START + "thread 'foo' panicked at crates/example.rs:2:1:\nfoo failed in its own process:\n" + child)
        self.assertIn('  panicked at crates/example.rs:1:1\n', log)
        self.assertIn('  panic reason: assertion equality failed; payload withheld (resource privacy)\n', log)
        self.assertIn('  panic source: assert_eq!(slots, 1, "child fixture failed");\n', log)
        self.assertNotIn('crates/example.rs:2:1', log)
        for withheld in ('PRIVATE', '987654', 'its own process'):
            self.assertNotIn(withheld, log)
        self.assertEqual(log.count('  failed test: foo\n'), 1)
        self.assertEqual(log.count('panicked at crates/example.rs:1:1'), 1)
        self.assertEqual(locations, {'foo': 'crates/example.rs:1:1'})

    def test_a_child_that_exits_without_panicking_shows_its_parents_static_message(self):
        log, locations = self.render_child(
            self.PARENT_START + "thread 'foo' panicked at crates/example.rs:2:1:\nchild test process failed\n")
        self.assertIn('  panicked at crates/example.rs:2:1\n', log)
        self.assertIn('  panic reason: custom panic; message: child test process failed\n', log)
        self.assertNotIn('payload withheld', log)
        self.assertEqual(locations, {'foo': 'crates/example.rs:2:1'})

    def test_a_childs_report_does_not_override_the_first_target_or_a_caught_panic(self):
        source = ['assert!(first);', 'panic!("a listener fails");', 'assert!(second);']
        first = ("     Running unittests src/lib.rs (a)\nrunning 1 test\ntest same ... FAILED\n\nfailures:\n\n"
                 "---- same stdout ----\nthread 'same' panicked at crates/example.rs:1:1:\nassertion failed: first\n"
                 "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n")
        second = ("     Running tests/b.rs (b)\nrunning 1 test\ntest same ... FAILED\n\nfailures:\n\n"
                  "---- same stdout ----\nthread 'same' panicked at crates/example.rs:3:1:\nassertion failed: second\n"
                  "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n")
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source):
            log = self.render(first + second)
            # Inside a child's own report, the last panic of its capture is still the one it failed on.
            caught = self.render(
                self.PARENT_START + "thread 'foo' panicked at crates/example.rs:3:1:\nwrapper\n"
                "running 1 test\ntest foo ... FAILED\n\n---- foo stdout ----\n"
                "thread 'foo' panicked at crates/example.rs:2:1:\na listener fails\n"
                "thread 'foo' panicked at crates/example.rs:1:1:\nassertion failed: first\n")
        self.assertEqual(log.count('panicked at crates/example.rs:1:1'), 2)
        self.assertNotIn('crates/example.rs:3:1', log)
        self.assertEqual(caught.count('  failed test: foo\n'), 1)
        self.assertIn('panicked at crates/example.rs:1:1', caught)
        self.assertNotIn('crates/example.rs:2:1', caught)
        self.assertNotIn('crates/example.rs:3:1', caught)

    def test_a_later_repeated_name_keeps_its_first_context_after_an_earlier_tests_child_run(self):
        source = ['assert!(first);', 'panic!("child test process failed");', 'assert!(second);']
        later = ("test same ... FAILED\ntest same ... FAILED\n\nfailures:\n\n"
                 "---- same stdout ----\nthread 'same' panicked at crates/example.rs:1:1:\nassertion failed: first\n"
                 "---- same stdout ----\nthread 'same' panicked at crates/example.rs:3:1:\nassertion failed: second\n")
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), \
                patch.dict(ci._panic_locations, clear=True):
            # foo's child run ends with its summary; no `running N tests` precedes the later repeated captures.
            log = self.render(self.PARENT_START + self.CHILD_REPORT +
                              "thread 'foo' panicked at crates/example.rs:2:1:\nchild test process failed\n" + later)
            locations = dict(ci._panic_locations)
        self.assertEqual(locations, {'foo': 'crates/example.rs:1:1', 'same': 'crates/example.rs:1:1'})
        self.assertEqual(log.count('  failed test: foo\n'), 1)
        self.assertEqual(log.count('  failed test: same\n'), 2)
        self.assertEqual(log.count('panicked at crates/example.rs:1:1'), 3)
        self.assertNotIn('crates/example.rs:2:1', log)
        self.assertNotIn('crates/example.rs:3:1', log)

    def test_vulkan_driver_files_alone_marks_a_vulkan_driver_present(self):
        with patch.dict(os.environ, {'VK_DRIVER_FILES': '/driver/icd.json'}), \
                patch.object(ci, 'ICD_DIRS', ()):
            os.environ.pop('VK_ICD_FILENAMES', None)
            self.assertTrue(ci.has_vulkan())
        with patch.dict(os.environ, clear=True), patch.object(ci, 'ICD_DIRS', ()):
            self.assertFalse(ci.has_vulkan())

    def test_a_shown_panic_message_allows_bounded_rustc_codes_not_diagnostic_text(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['assert!(ok, "fixture did not compile");']):
            log = self.render(
                "thread 'generated' panicked at crates/example.rs:1:1:\nfixture did not compile\n"
                "error[E0433]: PRIVATE\n  --> /hidden/path.rs:2:1\nerror[E0433]: duplicate\n"
                "error[E0308]: mismatched types\nerror[E0425]: missing name\nerror[E0599]: beyond the bound\n"
                "test generated ... FAILED\n"
                "thread 'other' panicked at crates/example.rs:1:1:\nunrecognized panic\n"
                "error[E0307]: unverified message\ntest other ... FAILED\n"
            )
        self.assertIn('compiler: rustc E0433, E0308, E0425 (diagnostic text withheld)', log)
        for withheld in ('PRIVATE', '/hidden/path.rs', 'mismatched types', 'E0599', 'E0307'):
            self.assertNotIn(withheld, log)

    def test_a_panic_that_is_rustc_output_shows_its_codes_not_its_text(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['assert!(output.status.success(), "{}", stderr);']), \
                patch.dict(ci._panic_locations, clear=True):
            log = self.render(
                "thread 'generated' panicked at crates/example.rs:1:1:\n"
                "error[E0425]: cannot find function `PRIVATE` in module `super::runtime`\n"
                "  --> /hidden/path.rs:2:1\nerror[E0609]: no field `SECRET`\nerror[E0425]: duplicate\n"
                "test generated ... FAILED\n"
            )
        self.assertIn('panic reason: rustc could not compile a test program; payload withheld', log)
        self.assertIn('compiler: rustc E0425, E0609 (diagnostic text withheld)', log)
        for withheld in ('PRIVATE', 'SECRET', '/hidden/path.rs', 'cannot find function'):
            self.assertNotIn(withheld, log)

    def test_payloads_cannot_become_locations_or_numeric_operands(self):
        path = 'crates/sunrise-client/src/title/tests.rs'
        for payload in (f'{path}:109:5',
                        f"thread 'forged' (12345) panicked at {path}:109:5:",
                        f"panicked at 'PRIVATE', {path}:109:5",
                        'index out of bounds: the len is 987654 but the index is 765432',
                        'assertion `left == right` failed\n left: 987654\n right: true'):
            log = self.render("test safe ... FAILED\n"
                              "thread 'safe' panicked at private.rs:1:1:\n" + payload + '\n')
            self.assertNotIn(path, log)
            self.assertNotIn('forged', log)
            self.assertNotIn('PRIVATE', log)
            self.assertNotIn('987654', log)
            self.assertNotIn('765432', log)
            self.assertNotIn('right: true', log)

    def test_raw_location_negative_matrix(self):
        path = 'crates/sunrise-client/src/title/tests.rs'
        locations = [f'crates/bad{control}/../sunrise-client/src/title/tests.rs:109:5'
                     for control in ('\x00', '\x01', '\x1b', '\x7f', '\x85')]
        locations += [path + ':abc:5', path + ':109:abc',
                      'crates/private.txt:1:1', 'crates/symlink.rs:1:1',
                      'crates/sunrise-client/src/not_tracked.rs:1:1']
        for location in locations:
            with self.subTest(location=location):
                log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {location}:\nPRIVATE\n")
                self.assertNotIn('  panicked at ', log)
                self.assertNotIn('PRIVATE', log)

    def test_a_location_in_a_large_tracked_source_of_any_package_is_reported(self):
        # sunrise-client-lib's sources live under crates/sunrise-client/ through its manifest, and motion.rs is
        # over 256 KB: its failure's location was withheld (#4028) while smaller sunrise-client files were shown.
        ci.tracked_rust_paths.cache_clear()
        ci.source_lines.cache_clear()
        # The integration split leaves motion.rs at 5,793 lines; 5000 is still inside it.
        for location in ('crates/sunrise-sim/src/motion.rs:5000:9',
                         'crates/sunrise-client-lib/../sunrise-sim/src/motion.rs:5000:9'):
            log = self.render("test motion::tests::resident_fluid_heights_are_measured_from_the_feet ... FAILED\n"
                              "thread 'motion::tests::resident_fluid_heights_are_measured_from_the_feet' panicked at "
                              f"{location}:\nassertion `left == right` failed\n left: 1\n right: 2\n")
            self.assertIn('  panicked at crates/sunrise-sim/src/motion.rs:5000:9', log)
            self.assertIn('0 locations withheld', log)
        # Past the read bound a tracked source still stays withheld.
        ci.tracked_rust_paths.cache_clear()
        with patch.object(ci, 'SOURCE_BYTES', 1000):
            log = self.render("test t ... FAILED\nthread 't' panicked at crates/sunrise-sim/src/motion.rs:1:1:\n"
                              "assertion failed: x\n")
        self.assertIn('panic location withheld', log)
        ci.tracked_rust_paths.cache_clear()
        ci.source_lines.cache_clear()

    def test_non_regular_git_entry_never_read_as_source(self):
        import subprocess
        for mode, kind in (('120000', 'blob'), ('040000', 'tree')):
            ci.source_lines.cache_clear()
            with patch.object(ci, 'tracked_rust_paths', return_value={'crates/symlink.rs'}), \
                    patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess(
                        [], 0, f'{mode} {kind} abc 42\tcrates/symlink.rs\n')) as git:
                log = self.render("test safe ... FAILED\nthread 'safe' panicked at crates/symlink.rs:1:1:\nPRIVATE\n")
            self.assertNotIn('  panicked at ', log)
            self.assertNotIn('PRIVATE', log)
            self.assertEqual(git.call_count, 1)
        ci.source_lines.cache_clear()

    def test_120_failures_all_have_identity_and_first_safe_context(self):
        output = ''.join(f"test suite::case_{i} ... FAILED\n"
                         f"thread 'suite::case_{i}' panicked at .local/private.rs:1:1:\n"
                         "assertion `left == right` failed: POISON\n left: 96\n right: 93\n"
                         for i in range(120))
        log = self.render(output)
        for i in range(120):
            self.assertIn(f'failed test: suite::case_{i}\n', log)
        self.assertEqual(log.count('assertion equality failed'), 120)
        self.assertNotIn('assertion left:', log)
        self.assertNotIn('assertion right:', log)
        self.assertIn('120 shown; 0 failing tests not shown', log)
        self.assertNotIn('POISON', log)
        self.assertNotIn('.local', log)

    def test_total_budget_counts_occurrences_including_duplicate_names(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 200):
            log = self.render('test repeated ... FAILED\n' * 120)
        payload, metadata = log.split('failure diagnostics: ')
        shown = payload.count('failed test:')
        self.assertLessEqual(len(payload.encode()), 200)
        self.assertIn(f'{shown} shown; {120 - shown} failing tests not shown', metadata)
        self.assertIn('total byte budget 200', metadata)

    def test_multibyte_identity_and_per_test_budget(self):
        log = self.render('test ' + '界' * 290 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertIn('shortened: per-test byte budget', payload)

    def test_missing_and_poisoned_context_are_explicit(self):
        log = self.render("test absent ... FAILED\ntest missing ... FAILED\n"
                          "thread 'missing' panicked at .local/private.rs:1:1:\n"
                          "test poisoned ... FAILED\n"
                          "thread 'poisoned' panicked at .local/private.rs:1:1:\n"
                          "secret token POISON\n")
        self.assertIn('panic context missing', log)
        self.assertIn('panic reason missing', log)
        self.assertIn('custom panic; payload withheld', log)
        self.assertNotIn('POISON', log)

    def test_forged_and_mixed_identity_lines_do_not_publish_tokens(self):
        log = self.render('test safe::case ... FAILED\n'
                          'test ghp_PRIVATE ... FAILED\n'
                          'test safe::ghp_PRIVATE ... FAILED\n'
                          'test safe::case/PRIVATE ... FAILED\n')
        self.assertIn('failed test: safe::case\n', log)
        self.assertEqual(log.count('failed test: identity withheld'), 3)
        self.assertNotIn('PRIVATE', log)
        self.assertIn('4 shown; 0 failing tests not shown', log)

    def test_true_utf8_boundary(self):
        # 15 prefix bytes + 600 UTF-8 identity bytes; clipping crosses a codepoint.
        log = self.render('test ' + '界' * 200 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertIn('界', payload)
        self.assertNotIn('\\u', payload)
        self.assertNotIn('\ufffd', payload)
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertGreater(len(payload.encode()), 497)

    def test_duplicate_names_use_first_context_for_each_occurrence(self):
        log = self.render("test same ... FAILED\nthread 'same' panicked at private.rs:1:1:\n"
                          "attempt to divide by zero\n"
                          "test same ... FAILED\nthread 'same' panicked at private.rs:2:1:\n"
                          "attempt to add with overflow\n")
        self.assertEqual(log.count('failed test: same'), 2)
        self.assertEqual(log.count('attempt to divide by zero'), 2)
        self.assertNotIn('attempt to add with overflow', log)

    def test_exact_total_edge_uses_independent_record_size(self):
        record = '  failed test: edge\n  panic context missing from captured output\n'
        for budget, expected in ((len(record.encode()) * 2, 2),
                                 (len(record.encode()) * 2 - 1, 1)):
            with patch.object(ci, 'FAILURE_TOTAL_BYTES', budget):
                log = self.render('test edge ... FAILED\n' * 3)
            self.assertIn(f'{expected} shown; {3 - expected} failing tests not shown', log)

    def test_flood_bounds_and_tracked_source_lookup_is_once(self):
        output = ''.join(f"test c{i} ... FAILED\nthread 'c{i}' panicked at crates/x.rs:1:1:\n"
                         "attempt to divide by zero\n" for i in range(20))
        ci.tracked_rust_paths.cache_clear()
        import subprocess
        with patch.object(ci, 'DIAGNOSTIC_LINES', 30), patch.object(ci, 'DIAGNOSTIC_CONTEXTS', 2), \
                patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '')) as git:
            log = self.render(output)
        self.assertEqual(git.call_count, 1)
        self.assertIn('30 lines skipped', log)
        self.assertIn('8 contexts skipped', log)
        ci.tracked_rust_paths.cache_clear()
        with patch.object(ci, 'DIAGNOSTIC_LINE_CHARS', 50):
            log = self.render('x' * 100 + '\ntest ok ... FAILED\n')
        self.assertIn('1 lines skipped', log)

    def test_policy_annotations_share_one_budget_across_runs(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 100), \
                patch.object(ci, 'known_failures', return_value={}), \
                patch.object(ci, 'run', return_value={'ghp_PRIVATE': False, 'safe': False}), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ci.main(['one', 'two']), 1)
        self.assertNotIn('PRIVATE', log.getvalue())
        annotations = ''.join(line + '\n' for line in log.getvalue().splitlines() if line.startswith('::error::'))
        self.assertLessEqual(len(annotations.encode()), 100)
        self.assertIn('problems not shown (shared total byte budget)', log.getvalue())


class VulkanAdapters(unittest.TestCase):
    """The one-line adapter log: synthetic Vulkan data and loaders, no driver needed."""

    @staticmethod
    def properties(name=b"llvmpipe\x07(LLVM 15.0.7,   256 bits)\xc3\xa9"):
        buffer = bytearray(824)
        struct.pack_into("<I", buffer, 0, (1 << 22) | (3 << 12) | 296)
        struct.pack_into("<I", buffer, 16, 4)
        buffer[20:20 + len(name)] = name
        struct.pack_into("<I", buffer, 300, 16384)
        struct.pack_into("<I", buffer, 320, 65536)
        struct.pack_into("<I", buffer, 324, 128 * 1024 * 1024)
        struct.pack_into("<I", buffer, 372, 16)
        return bytes(buffer)

    def loader(self, adapters, created=0):
        """A stand-in for the system loader that reports `adapters` (VkPhysicalDeviceProperties bytes)."""
        loader = MagicMock()
        loader.vkCreateInstance.return_value = created

        def enumerate_devices(_, count, devices):
            if devices is None:
                count._obj.value = len(adapters)
            else:
                for index in range(count._obj.value):
                    devices[index] = index + 1
            return 0

        def get_properties(device, buffer):
            ctypes.memmove(buffer, adapters[device - 1], len(adapters[device - 1]))

        loader.vkEnumeratePhysicalDevices.side_effect = enumerate_devices
        loader.vkGetPhysicalDeviceProperties.side_effect = get_properties
        return loader

    def test_an_adapter_is_one_sanitized_line_with_its_storage_limits(self):
        loader = self.loader([self.properties()])
        with patch.object(ci.ctypes, 'CDLL', return_value=loader):
            lines = ci._probe_vulkan_adapters()
        self.assertEqual(lines, [
            'vulkan adapter[0]: name=llvmpipe (LLVM 15.0.7, 256 bits) type=cpu api=1.3.296 '
            'max_storage_buffer_binding_size=134217728 (128 MiB) max_storage_buffers_per_shader_stage=16 '
            'max_texture_dimension_2d=16384'])
        self.assertTrue(all(32 <= ord(char) <= 126 for char in lines[0]))
        loader.vkDestroyInstance.assert_called_once()

    def test_at_most_four_adapters_are_shown_and_an_empty_name_is_unknown(self):
        loader = self.loader([self.properties(b"\x01\x02")] * 6)
        with patch.object(ci.ctypes, 'CDLL', return_value=loader):
            lines = ci._probe_vulkan_adapters()
        self.assertEqual(len(lines), 4)
        self.assertTrue(lines[3].startswith('vulkan adapter[3]: name=unknown type=cpu '))
        self.assertEqual(ci.vulkan_adapter_line(0, self.properties(b'x' * 200)).split(' type=')[0],
                         'vulkan adapter[0]: name=' + 'x' * 80)

    def test_failures_report_a_fixed_tag_and_never_raw_error_text(self):
        secret = '/home/runner/PRIVATE/libvulkan.so'
        failing = self.loader([])
        failing.vkEnumeratePhysicalDevices.side_effect = RuntimeError(secret)
        cases = (
            ('loader-missing', patch.object(ci.ctypes, 'CDLL', side_effect=OSError(secret)), False),
            ('instance-init-failed', patch.object(ci.ctypes, 'CDLL', return_value=self.loader([], created=-9)), False),
            ('no-physical-devices', patch.object(ci.ctypes, 'CDLL', return_value=self.loader([])), True),
            ('query-failed', patch.object(ci.ctypes, 'CDLL', return_value=failing), True),
        )
        for tag, loading, destroyed in cases:
            with self.subTest(tag=tag), loading as load:
                self.assertEqual(ci._probe_vulkan_adapters(), [f'vulkan adapter: unavailable ({tag})'])
                if destroyed:
                    load.return_value.vkDestroyInstance.assert_called_once()
                elif tag == 'instance-init-failed':
                    load.return_value.vkDestroyInstance.assert_not_called()

    def test_a_failing_instance_cleanup_never_raises_or_shows_its_error_text(self):
        secret = '/home/runner/PRIVATE/libvulkan.so'
        healthy = self.loader([self.properties()])
        failing = self.loader([])
        failing.vkEnumeratePhysicalDevices.side_effect = RuntimeError(secret)
        for loader, expected in (
                (healthy, ci.vulkan_adapter_line(0, self.properties())),
                (failing, 'vulkan adapter: unavailable (query-failed)')):
            loader.vkDestroyInstance.side_effect = RuntimeError(secret)
            with self.subTest(expected=expected[:30]), patch.object(ci.ctypes, 'CDLL', return_value=loader):
                lines = ci._probe_vulkan_adapters()
            self.assertEqual(lines, [expected])
            self.assertNotIn(secret, ''.join(lines))
            loader.vkDestroyInstance.assert_called_once()

    def test_a_probe_that_crashes_by_signal_reports_a_fixed_tag_and_none_of_its_output(self):
        secret = 'PRIVATE-DRIVER-OUTPUT'
        die = "os.kill(os.getpid(), signal.SIGKILL)" if os.name == 'posix' else "os._exit(1)"
        script = f"import os, signal, sys; print({secret!r}); sys.stdout.flush(); {die}"
        with patch.object(ci, 'VULKAN_PROBE_SCRIPT', script):
            lines = ci.vulkan_adapter_summaries()
        self.assertEqual(lines, ['vulkan adapter: unavailable (probe-crashed)'])
        self.assertNotIn(secret, ''.join(lines))

    def test_a_probe_that_hangs_is_stopped_at_the_timeout(self):
        with patch.object(ci, 'VULKAN_PROBE_SCRIPT', 'import time; time.sleep(30)'), \
                patch.object(ci, 'VULKAN_PROBE_TIMEOUT', 0.1):
            self.assertEqual(ci.vulkan_adapter_summaries(), ['vulkan adapter: unavailable (probe-timed-out)'])

    def probe_prints(self, text):
        with patch.object(ci, 'VULKAN_PROBE_SCRIPT', f'print({text!r})'):
            return ci.vulkan_adapter_summaries()

    def test_only_adapter_lines_survive_a_drivers_stdout_noise(self):
        adapter = ci.vulkan_adapter_line(0, self.properties())
        self.assertEqual(self.probe_prints('PRIVATE driver noise\n' + adapter + '\nmore noise'), [adapter])
        # The lines of the most adapters the probe reports (every index it can print) all match.
        every = [ci.vulkan_adapter_line(index, self.properties(name)) for index in range(ci.VULKAN_MAX_ADAPTERS)
                 for name in (b'x', b'x' * 200, b'\x01\x02', b'a\x07b\xc3\xa9')]
        self.assertTrue(all(ci.VULKAN_ADAPTER_LINE.match(line) for line in every))
        self.assertEqual(self.probe_prints('\n'.join(every[::4] * 2)), every[::4])

    def test_a_child_with_nothing_recognized_is_a_failed_query(self):
        self.assertEqual(self.probe_prints('PRIVATE only noise\nvulkan adapter: unavailable (PRIVATE)'),
                         ['vulkan adapter: unavailable (query-failed)'])
        self.assertEqual(self.probe_prints(''), ['vulkan adapter: unavailable (query-failed)'])

    def test_a_known_unavailable_tag_from_the_child_is_passed_on(self):
        for tag in sorted(ci.VULKAN_UNAVAILABLE_TAGS):
            line = f'vulkan adapter: unavailable ({tag})'
            self.assertEqual(self.probe_prints('noise\n' + line), [line])

    def test_a_probe_that_cannot_start_shows_no_error_text(self):
        secret = '/home/runner/PRIVATE/python'
        with patch.object(ci.subprocess, 'Popen', side_effect=OSError(secret)):
            lines = ci.vulkan_adapter_summaries()
        self.assertEqual(lines, ['vulkan adapter: unavailable (query-failed)'])
        self.assertNotIn(secret, ''.join(lines))

    def test_a_noisy_probe_is_capped_killed_and_reaped(self):
        secret = 'PRIVATE-NOISE'
        script = ("import sys\n"
                  "try:\n"
                  "    while True:\n"
                  f"        sys.stdout.buffer.write({secret!r}.encode() * 1024); sys.stdout.buffer.flush()\n"
                  "except OSError:\n"
                  "    pass\n")
        started, real = [], subprocess.Popen

        def spy(*args, **kwargs):
            started.append(real(*args, **kwargs))
            return started[-1]

        begun = time.monotonic()
        with patch.object(ci, 'VULKAN_PROBE_SCRIPT', script), patch.object(ci.subprocess, 'Popen', side_effect=spy):
            lines = ci.vulkan_adapter_summaries()
        # Far under the timeout: the cap, not the clock, ended it.
        self.assertLess(time.monotonic() - begun, ci.VULKAN_PROBE_TIMEOUT / 2)
        self.assertEqual(lines, ['vulkan adapter: unavailable (probe-output-capped)'])
        self.assertNotIn(secret, ''.join(lines))
        self.assertEqual(len(started), 1)
        self.assertIsNotNone(started[0].returncode)

    def test_the_api_major_version_ignores_the_variant_bits(self):
        properties = bytearray(self.properties())
        struct.pack_into("<I", properties, 0, (7 << 29) | (1 << 22) | (3 << 12) | 296)
        self.assertIn(' api=1.3.296 ', ci.vulkan_adapter_line(0, bytes(properties)))

    def test_main_logs_the_adapters_only_where_a_vulkan_driver_is_present(self):
        for vulkan in (True, False):
            with self.subTest(vulkan=vulkan), patch.object(ci, 'has_vulkan', return_value=vulkan), \
                    patch.object(ci, 'vulkan_adapter_summaries', return_value=['vulkan adapter: one line']) as probe, \
                    patch.object(ci, 'known_failures', return_value={}), \
                    patch.object(ci, 'run', return_value={'fine': True}), \
                    contextlib.redirect_stdout(io.StringIO()) as log:
                self.assertEqual(ci.main(['example']), 0)
            self.assertEqual('vulkan adapter: one line\n' in log.getvalue(), vulkan)
            self.assertEqual(probe.called, vulkan)


class PublicText(unittest.TestCase):
    """What a failure may show beyond its reason: only text that occurs verbatim in the tracked source around it."""
    SOURCE = ['fn placement() {',
              '    let slots = menu_slots(&rules);',
              '    assert_eq!(',
              '        slots,',
              '        (3, Placement::Hotbar, true),',
              '        "installed rules move the mod menu"',
              '    );',
              '    assert!(slots.0 > 1, "at least {} slots", slots.0);',
              '}']

    def render(self, payload, line=3):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=self.SOURCE), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics("test menu::placement ... FAILED\n"
                                   f"thread 'menu::placement' panicked at crates/example.rs:{line}:5:\n" + payload)
        return log.getvalue()

    def test_the_statement_a_literal_message_and_a_written_operand_are_shown(self):
        log = self.render("assertion `left == right` failed: installed rules move the mod menu\n"
                          "  left: (2, Placement::Inventory, true)\n"
                          " right: (3, Placement::Hotbar, true)\n")
        self.assertIn('  panic reason: assertion equality failed; message: installed rules move the mod menu\n', log)
        self.assertIn('  panic source: assert_eq!( slots, (3, Placement::Hotbar, true), '
                      '"installed rules move the mod menu" );\n', log)
        self.assertIn('  right: (3, Placement::Hotbar, true)\n', log)
        # The computed side is not in the source: it may come from prepared data.
        self.assertIn('  left: withheld (not text of the tracked source)\n', log)
        self.assertNotIn('Inventory', log)

    def test_a_message_with_a_computed_value_and_untracked_sources_stay_withheld(self):
        log = self.render("at least 987654 slots\n", line=8)
        self.assertIn('custom panic; payload withheld (resource privacy)', log)
        self.assertNotIn('987654', log)
        with patch.object(ci, 'tracked_rust_paths', return_value=set()), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics("test t ... FAILED\nthread 't' panicked at crates/example.rs:3:5:\n"
                                   "assertion `left == right` failed: installed rules move the mod menu\n"
                                   " right: (3, Placement::Hotbar, true)\n")
        self.assertNotIn('installed rules', log.getvalue())
        self.assertNotIn('Hotbar', log.getvalue())
