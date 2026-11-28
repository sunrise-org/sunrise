"""Pure-Python unit tests for tools/packet_capture_compare.py."""
from __future__ import annotations

import hashlib
import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import packet_capture_compare as cmp_tool  # noqa: E402


def digest_for(raw: bytes) -> str:
    """Deterministic 32-hex-char test digest derived from raw packet bytes."""
    return hashlib.blake2b(raw, digest_size=16).hexdigest()


def write_run(
    directory: Path,
    sessions: list[tuple[int, int, str, list[tuple[int, int, int, int, str]]]],
    *,
    run_id: str = "100-1",
    run_status: str = "ok",
    session_statuses: dict[tuple[int, int], str] | None = None,
    write_manifest: bool = True,
    manifest_end: str | None = "__default__",
    footer_override: dict[tuple[int, int], str | None] | None = None,
    filename_override: dict[tuple[int, int], str] | None = None,
    claim_id: str | None = "__default__",
    trailing_newline: bool = True,
) -> None:
    """Write a complete (or intentionally broken) capture directory."""
    directory.mkdir(parents=True, exist_ok=True)
    statuses = session_statuses or {}
    footers = footer_override or {}
    filenames = filename_override or {}
    manifest_lines = [f"# manifest v2\t{run_id}\t{run_status}"]
    for conn, ordinal, player, rows in sessions:
        status = statuses.get((conn, ordinal), "ok")
        if status == "incomplete:open_failed":
            manifest_lines.append(f"{conn}\t{ordinal}\t{player}\t-\t{status}\t0")
            continue
        filename = filenames.get(
            (conn, ordinal), f"{cmp_tool.sanitize_filename(player)}-{conn}.pkt"
        )
        lines = [f"# v2\t{run_id}\t{conn}\t{ordinal}\t{player}"]
        for seq, tick, pkt_id, length, digest in rows:
            lines.append(f"{seq}\t{tick}\t{pkt_id}\t{length}\t{digest}")
        if (conn, ordinal) in footers:
            custom = footers[(conn, ordinal)]
            if custom is not None:
                lines.append(custom)
        else:
            lines.append(f"# end\t{len(rows)}\tok")
        body = "\n".join(lines) + ("\n" if trailing_newline else "")
        (directory / filename).write_text(body, encoding="utf-8")
        manifest_lines.append(f"{conn}\t{ordinal}\t{player}\t{filename}\t{status}\t{len(rows)}")
    if claim_id == "__default__":
        (directory / ".capture-claim").write_text(f"{run_id}\n", encoding="utf-8")
    elif claim_id is not None:
        (directory / ".capture-claim").write_text(f"{claim_id}\n", encoding="utf-8")
    if write_manifest:
        if manifest_end == "__default__":
            manifest_lines.append(f"# end\t{len(sessions)}\t{run_status}")
        elif manifest_end is not None:
            manifest_lines.append(manifest_end)
        (directory / "manifest.tsv").write_text("\n".join(manifest_lines) + "\n", encoding="utf-8")


class PacketCaptureCompareTests(unittest.TestCase):
    def test_equal_recordings_match_by_player_name_and_ordinal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            alice_rows = [
                (0, 1, 50, 83, digest_for(b"\x32hello")),
                (1, 2, 46, 4096, digest_for(b"\x2echunk")),
            ]
            bob_rows = [
                (0, 1, 50, 83, digest_for(b"\x32bob")),
            ]
            # Different run IDs and connection IDs; matching uses (player, ordinal).
            write_run(a, [(1, 1, "Alice", alice_rows), (2, 1, "Bob", bob_rows)], run_id="100-1")
            write_run(b, [(9, 1, "Alice", alice_rows), (4, 1, "Bob", bob_rows)], run_id="200-1")

            out = io.StringIO()
            with redirect_stdout(out):
                rc = cmp_tool.main([str(a), str(b), "--run-id-a", "100-1", "--run-id-b", "200-1"])
            self.assertEqual(rc, 0)
            text = out.getvalue()
            self.assertIn("Alice: identical (2 packets)", text)
            self.assertIn("Bob: identical (1 packets)", text)
            self.assertTrue(text.strip().endswith("IDENTICAL"))

            # Mismatched expected run id fails validation cleanly.
            err = io.StringIO()
            with redirect_stderr(err):
                rc_bad = cmp_tool.main([str(a), str(b), "--run-id-a", "999-1"])
            self.assertEqual(rc_bad, 2)
            self.assertIn("does not match expected", err.getvalue())

    def test_reconnects_match_by_ordinal_and_detect_omitted_reconnect(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b, c = Path(tmp) / "a", Path(tmp) / "b", Path(tmp) / "c"
            s1 = [(0, 1, 50, 10, digest_for(b"s1"))]
            s2 = [(0, 20, 50, 12, digest_for(b"s2"))]
            write_run(a, [(1, 1, "Alice", s1), (4, 2, "Alice", s2)], run_id="1-1")
            write_run(b, [(2, 1, "Alice", s1), (7, 2, "Alice", s2)], run_id="2-1")
            write_run(c, [(2, 1, "Alice", s1)], run_id="3-1")

            out_ok = io.StringIO()
            with redirect_stdout(out_ok):
                rc_ok = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc_ok, 0)
            self.assertIn("Alice #1: identical (1 packets)", out_ok.getvalue())
            self.assertIn("Alice #2: identical (1 packets)", out_ok.getvalue())

            out_missing = io.StringIO()
            with redirect_stdout(out_missing):
                rc_missing = cmp_tool.main([str(a), str(c)])
            self.assertEqual(rc_missing, 1)
            self.assertIn("Alice #2: DIFFERS only in A", out_missing.getvalue())

    def test_one_byte_different_reports_first_mismatch_details(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            payload_a = bytearray(b"\x08block_update_payload")
            payload_b = bytearray(payload_a)
            payload_b[-1] ^= 0x01
            hash_a = digest_for(bytes(payload_a))
            hash_b = digest_for(bytes(payload_b))
            self.assertNotEqual(hash_a, hash_b)
            write_run(
                a,
                [
                    (
                        1,
                        1,
                        "Alice",
                        [
                            (0, 10, 50, 80, digest_for(b"login")),
                            (1, 11, 8, len(payload_a), hash_a),
                            (2, 12, 46, 512, digest_for(b"c1")),
                        ],
                    )
                ],
            )
            write_run(
                b,
                [
                    (
                        1,
                        1,
                        "Alice",
                        [
                            (0, 10, 50, 80, digest_for(b"login")),
                            (1, 11, 8, len(payload_b), hash_b),
                            (2, 12, 46, 999, digest_for(b"c2")),
                        ],
                    )
                ],
            )

            out = io.StringIO()
            with redirect_stdout(out):
                rc = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc, 1)
            text = out.getvalue()
            self.assertIn("Alice: DIFFERS", text)
            self.assertIn("seq=1", text)
            self.assertIn("tick=11", text)
            self.assertIn("id=8 (block_update)", text)
            self.assertIn(f"len={len(payload_a)} vs {len(payload_b)}", text)
            self.assertIn(f"hash={hash_a} vs {hash_b}", text)
            self.assertNotIn("seq=2", text)
            self.assertTrue(text.strip().endswith("DIFFERENT"))

    def test_reordered_packets_fail_with_both_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            ha, hb = digest_for(b"blk"), digest_for(b"time")
            write_run(a, [(1, 1, "Alice", [(0, 5, 8, 10, ha), (1, 5, 115, 24, hb)])])
            write_run(b, [(1, 1, "Alice", [(0, 5, 115, 24, hb), (1, 5, 8, 10, ha)])])

            out = io.StringIO()
            with redirect_stdout(out):
                rc = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc, 1)
            text = out.getvalue()
            self.assertIn("seq=0", text)
            self.assertIn("tick=5", text)
            self.assertIn("8 (block_update) vs 115 (set_time)", text)
            self.assertIn("len=10 vs 24", text)
            self.assertIn(f"hash={ha} vs {hb}", text)

    def test_missing_connection_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            h = digest_for(b"p")
            write_run(a, [(1, 1, "Alice", [(0, 1, 50, 10, h)]), (2, 1, "Bob", [(0, 1, 50, 10, h)])])
            write_run(
                b, [(1, 1, "Alice", [(0, 1, 50, 10, h)]), (3, 1, "Carol", [(0, 1, 50, 10, h)])]
            )

            out = io.StringIO()
            with redirect_stdout(out):
                rc = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc, 1)
            text = out.getvalue()
            self.assertIn("Alice: identical (1 packets)", text)
            self.assertIn("Bob: DIFFERS only in A", text)
            self.assertIn("Carol: DIFFERS only in B", text)

    def test_unequal_counts_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            h1, h2 = digest_for(b"1"), digest_for(b"2")
            write_run(a, [(1, 1, "Alice", [(0, 1, 50, 10, h1)])])
            write_run(b, [(1, 1, "Alice", [(0, 1, 50, 10, h1), (1, 2, 8, 14, h2)])])

            out = io.StringIO()
            with redirect_stdout(out):
                rc = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc, 1)
            text = out.getvalue()
            self.assertIn("count=1 vs 2", text)
            self.assertIn("seq=1", text)
            self.assertIn("tick=2", text)
            self.assertIn("8 (block_update)", text)

    def test_ignored_id_off_by_default_labels_filtered_pass_and_catches_following_mutation(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b, c = Path(tmp) / "a", Path(tmp) / "b", Path(tmp) / "c"
            h_login = digest_for(b"login")
            h_ka = digest_for(b"ka1")
            h_blk = digest_for(b"blk")
            h_blk_mut = digest_for(b"blk!")
            write_run(
                a,
                [
                    (
                        1,
                        1,
                        "Alice",
                        [
                            (0, 1, 50, 80, h_login),
                            (1, 1, 45, 9, h_ka),
                            (2, 2, 8, 12, h_blk),
                        ],
                    )
                ],
            )
            write_run(
                b,
                [
                    (
                        1,
                        1,
                        "Alice",
                        [
                            (0, 1, 50, 80, h_login),
                            (1, 2, 8, 12, h_blk),
                        ],
                    )
                ],
            )
            write_run(
                c,
                [
                    (
                        1,
                        1,
                        "Alice",
                        [
                            (0, 1, 50, 80, h_login),
                            (1, 2, 8, 12, h_blk_mut),
                        ],
                    )
                ],
            )

            # Default: keep_alive is NOT ignored.
            out_default = io.StringIO()
            with redirect_stdout(out_default):
                rc_default = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc_default, 1)
            self.assertIn("45 (keep_alive)", out_default.getvalue())

            # With --ignore keep_alive: retained pass is clearly labelled as filtered.
            out_named = io.StringIO()
            with redirect_stdout(out_named):
                rc_named = cmp_tool.main([str(a), str(b), "--ignore", "keep_alive"])
            self.assertEqual(rc_named, 0)
            self.assertIn("retained packets identical (2 kept, 1/0 ignored)", out_named.getvalue())
            self.assertIn(
                "IDENTICAL (retained only; 1 ignored packet(s) skipped)", out_named.getvalue()
            )

            # Numeric packet id works too.
            out_num = io.StringIO()
            with redirect_stdout(out_num):
                rc_num = cmp_tool.main([str(a), str(b), "--ignore", "45"])
            self.assertEqual(rc_num, 0)

            # Mutation immediately after ignored row fails and reports both raw sequence numbers.
            out_mut = io.StringIO()
            with redirect_stdout(out_mut):
                rc_mut = cmp_tool.main([str(a), str(c), "--ignore", "keep_alive"])
            self.assertEqual(rc_mut, 1)
            self.assertIn("seq=2 vs 1", out_mut.getvalue())
            self.assertIn("id=8 (block_update)", out_mut.getvalue())
            self.assertIn(f"hash={h_blk} vs {h_blk_mut}", out_mut.getvalue())

    def test_tick_difference_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            h = digest_for(b"blk")
            write_run(a, [(1, 1, "Alice", [(0, 4, 8, 12, h)])])
            write_run(b, [(1, 1, "Alice", [(0, 5, 8, 12, h)])])

            out = io.StringIO()
            with redirect_stdout(out):
                rc = cmp_tool.main([str(a), str(b)])
            self.assertEqual(rc, 1)
            self.assertIn("tick=4 vs 5", out.getvalue())

    def test_equally_truncated_and_malformed_recordings_are_rejected(self) -> None:
        h = digest_for(b"ok")
        cases: list[tuple[str, dict]] = [
            (
                "matched_prefix_without_footer",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "footer_override": {(1, 1): None},
                },
            ),
            (
                "header_only_without_footer",
                {
                    "sessions": [(1, 1, "Alice", [])],
                    "footer_override": {(1, 1): None},
                },
            ),
            (
                "incomplete_footer_marker",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "footer_override": {(1, 1): "# end\t1\tincomplete\tunflushed_tail"},
                },
            ),
            (
                "footer_count_mismatch",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "footer_override": {(1, 1): "# end\t2\tok"},
                },
            ),
            (
                "missing_manifest",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "write_manifest": False,
                },
            ),
            (
                "missing_manifest_end_marker",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "manifest_end": None,
                },
            ),
            (
                "manifest_end_count_mismatch",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "manifest_end": "# end\t2\tok",
                },
            ),
            (
                "claim_run_id_mismatch",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "claim_id": "999-9",
                },
            ),
            (
                "failed_open_session_in_manifest",
                {
                    "sessions": [(1, 1, "Alice", [])],
                    "run_status": "incomplete",
                    "session_statuses": {(1, 1): "incomplete:open_failed"},
                },
            ),
            (
                "missing_trailing_newline",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "trailing_newline": False,
                },
            ),
            (
                "partial_digest",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, "deadbeef")])],
                },
            ),
            (
                "noncontiguous_sequence",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h), (2, 1, 8, 10, h)])],
                },
            ),
            (
                "reused_conn_id_under_different_players",
                {
                    "sessions": [
                        (1, 1, "Alice", [(0, 1, 50, 10, h)]),
                        (1, 1, "Bob", [(0, 1, 50, 10, h)]),
                    ],
                },
            ),
            (
                "missing_initial_ordinal",
                {
                    "sessions": [(1, 2, "Alice", [(0, 1, 50, 10, h)])],
                },
            ),
            (
                "missing_intermediate_ordinal",
                {
                    "sessions": [
                        (1, 1, "Alice", [(0, 1, 50, 10, h)]),
                        (2, 3, "Alice", [(0, 1, 50, 10, h)]),
                    ],
                },
            ),
            (
                "filename_without_pkt_suffix",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 50, 10, h)])],
                    "filename_override": {(1, 1): "Alice-1.txt"},
                },
            ),
            (
                "conn_exceeds_u32",
                {
                    "sessions": [(1 << 32, 1, "Alice", [(0, 1, 50, 10, h)])],
                },
            ),
            (
                "packet_id_exceeds_i32",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1, 1 << 31, 10, h)])],
                },
            ),
            (
                "tick_exceeds_i64",
                {
                    "sessions": [(1, 1, "Alice", [(0, 1 << 63, 50, 10, h)])],
                },
            ),
        ]
        for name, kwargs in cases:
            with self.subTest(case=name), tempfile.TemporaryDirectory() as tmp:
                a, b = Path(tmp) / "a", Path(tmp) / "b"
                # Write the exact same broken recording on both sides: must NEVER compare equal.
                write_run(a, **kwargs)
                write_run(b, **kwargs)
                err = io.StringIO()
                with redirect_stderr(err):
                    rc = cmp_tool.main([str(a), str(b)])
                self.assertEqual(rc, 2, f"{name} should fail validation, stderr={err.getvalue()!r}")

    def test_unreadable_file_returns_input_error_exit_code(self) -> None:
        if os.geteuid() == 0:
            self.skipTest("chmod 000 does not block root reads")
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            h = digest_for(b"ok")
            write_run(a, [(1, 1, "Alice", [(0, 1, 50, 10, h)])])
            write_run(b, [(1, 1, "Alice", [(0, 1, 50, 10, h)])])
            pkt = a / "Alice-1.pkt"
            pkt.chmod(0)
            try:
                err = io.StringIO()
                with redirect_stderr(err):
                    rc = cmp_tool.main([str(a), str(b)])
                self.assertEqual(rc, 2)
                self.assertIn("error:", err.getvalue())
            finally:
                pkt.chmod(0o600)

    def test_unknown_ignore_name_errors(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            write_run(a, [(1, 1, "Alice", [])])
            write_run(b, [(1, 1, "Alice", [])])
            err = io.StringIO()
            with redirect_stderr(err):
                rc = cmp_tool.main([str(a), str(b), "--ignore", "not_a_real_packet"])
            self.assertEqual(rc, 2)
            self.assertIn("not_a_real_packet", err.getvalue())

    def test_wall_clock_packets_match_checked_in_protocol_ids_and_help(self) -> None:
        names = cmp_tool.load_play_clientbound_names()
        self.assertGreater(len(names), 100)
        help_text = cmp_tool.build_parser().format_help()
        self.assertIn("none ignored by default", help_text)
        for name, pid, _ in cmp_tool.WALL_CLOCK_PACKETS:
            self.assertEqual(
                names[pid],
                name,
                f"WALL_CLOCK_PACKETS id {pid} does not match protocol table entry {names[pid]!r}",
            )
            self.assertIn(name, help_text)
            self.assertIn(str(pid), help_text)


if __name__ == "__main__":
    unittest.main()
