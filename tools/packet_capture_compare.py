#!/usr/bin/env python3
"""Compare two per-connection play packet capture directories (SUNRISE_PACKET_CAPTURE)."""
from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IDS_PATH = ROOT / "crates/sunrise-protocol/src/ids.rs"
HEX_DIGEST_RE = re.compile(r"^[0-9a-f]{32}$")
INT_RE = re.compile(r"^-?[0-9]+$")

U32_MAX = (1 << 32) - 1
I32_MAX = (1 << 31) - 1
I64_MIN = -(1 << 63)
I64_MAX = (1 << 63) - 1
U64_MAX = (1 << 64) - 1

# Clientbound play packets whose wire payloads can carry wall-clock values.
WALL_CLOCK_PACKETS: tuple[tuple[str, int, str], ...] = (
    ("debug_sample", 30, "server tick timing samples (dedicated_server_tick_time)"),
    ("keep_alive", 45, "elapsed milliseconds since join sent by the network thread"),
    ("pong_response", 63, "echoes the client PingRequest wall-clock timestamp"),
    ("player_chat", 66, "epoch-millisecond chat message timestamp (also carries message/sender)"),
    (
        "player_info_update",
        71,
        "smoothed keep-alive RTT in UPDATE_LATENCY (also carries roster/gamemode/session)",
    ),
    (
        "update_advancements",
        133,
        "epoch-millisecond criterion grant timestamps (also carries criteria/completion)",
    ),
)


@dataclass(frozen=True)
class PacketRecord:
    seq: int
    tick: int
    packet_id: int
    length: int
    digest: str


@dataclass(frozen=True)
class ConnectionCapture:
    run_id: str
    player: str
    conn: int
    ordinal: int
    path: Path
    packets: list[PacketRecord]


def sanitize_filename(name: str) -> str:
    clean = "".join(
        c if (c.isascii() and (c.isalnum() or c in "_-")) else "_" for c in name
    )
    return clean if clean else "player"


def load_play_clientbound_names(path: Path = IDS_PATH) -> list[str]:
    """Return play::clientbound::NAMES indexed by packet id."""
    if not path.is_file():
        return []
    text = path.read_text(encoding="utf-8")
    play_block = re.search(r"pub mod play \{(.*?)\n\}", text, re.S)
    if not play_block:
        return []
    cb_block = re.search(
        r"pub mod clientbound \{.*?NAMES: &\[&str\] = &\[(.*?)\];",
        play_block.group(1),
        re.S,
    )
    if not cb_block:
        return []
    return re.findall(r'"([^"]+)"', cb_block.group(1))


def packet_label(packet_id: int, names: list[str]) -> str:
    if 0 <= packet_id < len(names):
        return f"{packet_id} ({names[packet_id]})"
    return f"{packet_id} (unknown)"


def _parse_int(
    text: str,
    field: str,
    path: Path,
    lineno: int,
    *,
    min_value: int,
    max_value: int,
) -> int:
    if not INT_RE.match(text):
        raise ValueError(f"{path}:{lineno}: invalid {field}: {text!r}")
    value = int(text, 10)
    if value < min_value or value > max_value:
        raise ValueError(
            f"{path}:{lineno}: {field} out of range ({min_value}..={max_value}): {value}"
        )
    return value


def parse_capture_file(path: Path) -> ConnectionCapture:
    raw_bytes = path.read_bytes()
    if not raw_bytes:
        raise ValueError(f"{path}: empty capture file")
    if not raw_bytes.endswith(b"\n"):
        raise ValueError(f"{path}: truncated file (missing trailing newline)")
    try:
        text = raw_bytes.decode("utf-8")
    except UnicodeDecodeError as err:
        raise ValueError(f"{path}: invalid UTF-8: {err}") from err

    lines = text.splitlines()
    if len(lines) < 2:
        raise ValueError(f"{path}: incomplete capture file (missing end marker)")

    header = lines[0].split("\t")
    if len(header) != 5 or header[0] != "# v2":
        raise ValueError(f"{path}:1: invalid capture header: {lines[0]!r}")
    run_id = header[1]
    conn = _parse_int(header[2], "conn", path, 1, min_value=0, max_value=U32_MAX)
    ordinal = _parse_int(header[3], "ordinal", path, 1, min_value=1, max_value=U32_MAX)
    player = header[4]
    if not run_id or not player:
        raise ValueError(f"{path}:1: invalid capture header fields: {lines[0]!r}")

    packets: list[PacketRecord] = []
    ended = False
    for lineno, line in enumerate(lines[1:], start=2):
        if ended:
            raise ValueError(f"{path}:{lineno}: trailing data after end marker")
        if line.startswith("# end\t"):
            parts = line.split("\t")
            if len(parts) < 3:
                raise ValueError(f"{path}:{lineno}: malformed end marker: {line!r}")
            declared_count = _parse_int(
                parts[1], "end count", path, lineno, min_value=0, max_value=U64_MAX
            )
            status = parts[2]
            if status != "ok":
                reason = parts[3] if len(parts) > 3 else status
                raise ValueError(f"{path}:{lineno}: incomplete capture ({reason})")
            if len(parts) != 3:
                raise ValueError(f"{path}:{lineno}: malformed ok end marker: {line!r}")
            if declared_count != len(packets):
                raise ValueError(
                    f"{path}:{lineno}: end marker count {declared_count} != {len(packets)} rows"
                )
            ended = True
            continue
        if line.startswith("#") or not line:
            raise ValueError(f"{path}:{lineno}: unexpected line in capture body: {line!r}")
        parts = line.split("\t")
        if len(parts) != 5:
            raise ValueError(
                f"{path}:{lineno}: expected 5 tab-separated fields, got {len(parts)}"
            )
        seq = _parse_int(parts[0], "seq", path, lineno, min_value=0, max_value=U64_MAX)
        tick = _parse_int(parts[1], "tick", path, lineno, min_value=I64_MIN, max_value=I64_MAX)
        packet_id = _parse_int(
            parts[2], "packet_id", path, lineno, min_value=0, max_value=I32_MAX
        )
        length = _parse_int(parts[3], "length", path, lineno, min_value=0, max_value=U64_MAX)
        digest = parts[4]
        if seq != len(packets):
            raise ValueError(
                f"{path}:{lineno}: noncontiguous sequence {seq} (expected {len(packets)})"
            )
        if not HEX_DIGEST_RE.match(digest):
            raise ValueError(f"{path}:{lineno}: invalid digest {digest!r}")
        packets.append(
            PacketRecord(
                seq=seq,
                tick=tick,
                packet_id=packet_id,
                length=length,
                digest=digest,
            )
        )

    if not ended:
        raise ValueError(f"{path}: truncated capture (missing # end marker)")

    return ConnectionCapture(
        run_id=run_id,
        player=player,
        conn=conn,
        ordinal=ordinal,
        path=path,
        packets=packets,
    )


def load_directory(
    directory: Path,
    *,
    expected_run_id: str | None = None,
) -> dict[tuple[str, int], ConnectionCapture]:
    """Validate manifest.tsv and every *.pkt file in `directory`, keyed by (player, ordinal)."""
    if not directory.is_dir():
        raise ValueError(f"not a directory: {directory}")
    manifest_path = directory / "manifest.tsv"
    if not manifest_path.is_file():
        raise ValueError(f"{directory}: incomplete run (missing manifest.tsv)")

    raw_bytes = manifest_path.read_bytes()
    if not raw_bytes or not raw_bytes.endswith(b"\n"):
        raise ValueError(f"{manifest_path}: truncated manifest (missing trailing newline)")
    try:
        lines = raw_bytes.decode("utf-8").splitlines()
    except UnicodeDecodeError as err:
        raise ValueError(f"{manifest_path}: invalid UTF-8: {err}") from err
    if len(lines) < 2:
        raise ValueError(f"{manifest_path}: incomplete manifest (missing # end marker)")

    header = lines[0].split("\t")
    if len(header) != 3 or header[0] != "# manifest v2":
        raise ValueError(f"{manifest_path}:1: invalid manifest header: {lines[0]!r}")
    run_id, run_status = header[1], header[2]
    if not run_id:
        raise ValueError(f"{manifest_path}:1: empty run id")
    if expected_run_id is not None and run_id != expected_run_id:
        raise ValueError(
            f"{manifest_path}:1: run id {run_id!r} does not match expected {expected_run_id!r}"
        )
    if run_status != "ok":
        raise ValueError(f"{manifest_path}:1: incomplete capture run (status={run_status})")

    claim_path = directory / ".capture-claim"
    if claim_path.exists():
        claim_raw = claim_path.read_bytes()
        if not claim_raw or not claim_raw.endswith(b"\n"):
            raise ValueError(f"{claim_path}: truncated capture claim file")
        try:
            claim_id = claim_raw.decode("utf-8").strip()
        except UnicodeDecodeError as err:
            raise ValueError(f"{claim_path}: invalid UTF-8: {err}") from err
        if claim_id != run_id:
            raise ValueError(
                f"{claim_path}: claim run id {claim_id!r} != manifest run id {run_id!r}"
            )

    captures: dict[tuple[str, int], ConnectionCapture] = {}
    expected_files: set[str] = set()
    seen_conns: set[int] = set()
    ordinals_by_player: dict[str, list[int]] = defaultdict(list)
    ended = False
    for lineno, line in enumerate(lines[1:], start=2):
        if ended:
            raise ValueError(f"{manifest_path}:{lineno}: trailing data after manifest end marker")
        if line.startswith("# end\t"):
            parts = line.split("\t")
            if len(parts) != 3:
                raise ValueError(f"{manifest_path}:{lineno}: malformed manifest end marker: {line!r}")
            declared_sessions = _parse_int(
                parts[1], "session count", manifest_path, lineno, min_value=0, max_value=U64_MAX
            )
            end_status = parts[2]
            if end_status != "ok":
                raise ValueError(
                    f"{manifest_path}:{lineno}: incomplete manifest end status ({end_status})"
                )
            if declared_sessions != len(captures):
                raise ValueError(
                    f"{manifest_path}:{lineno}: manifest end session count "
                    f"{declared_sessions} != {len(captures)} rows"
                )
            ended = True
            continue
        if line.startswith("#") or not line:
            raise ValueError(f"{manifest_path}:{lineno}: unexpected line in manifest: {line!r}")
        parts = line.split("\t")
        if len(parts) != 6:
            raise ValueError(
                f"{manifest_path}:{lineno}: expected 6 tab-separated fields, got {len(parts)}"
            )
        conn = _parse_int(parts[0], "conn", manifest_path, lineno, min_value=0, max_value=U32_MAX)
        ordinal = _parse_int(
            parts[1], "ordinal", manifest_path, lineno, min_value=1, max_value=U32_MAX
        )
        player, filename, status = parts[2], parts[3], parts[4]
        declared_count = _parse_int(
            parts[5], "count", manifest_path, lineno, min_value=0, max_value=U64_MAX
        )
        if not player:
            raise ValueError(f"{manifest_path}:{lineno}: empty player name")
        if status != "ok":
            raise ValueError(
                f"{manifest_path}:{lineno}: session {player} (conn={conn}) incomplete ({status})"
            )
        if conn in seen_conns:
            raise ValueError(f"{manifest_path}:{lineno}: duplicate conn id {conn}")
        seen_conns.add(conn)

        expected_filename = f"{sanitize_filename(player)}-{conn}.pkt"
        if filename != expected_filename:
            raise ValueError(
                f"{manifest_path}:{lineno}: invalid session filename {filename!r} "
                f"(expected {expected_filename!r})"
            )
        if filename in expected_files:
            raise ValueError(f"{manifest_path}:{lineno}: duplicate filename {filename!r}")
        expected_files.add(filename)

        pkt_path = directory / filename
        if not pkt_path.is_file():
            raise ValueError(f"{manifest_path}:{lineno}: missing session file {filename}")
        cap = parse_capture_file(pkt_path)
        if (
            cap.run_id != run_id
            or cap.conn != conn
            or cap.ordinal != ordinal
            or cap.player != player
        ):
            raise ValueError(
                f"{pkt_path}: header ({cap.run_id}, conn={cap.conn}, ord={cap.ordinal}, "
                f"player={cap.player!r}) does not match manifest ({run_id}, conn={conn}, "
                f"ord={ordinal}, player={player!r})"
            )
        if len(cap.packets) != declared_count:
            raise ValueError(
                f"{pkt_path}: packet count {len(cap.packets)} != manifest count {declared_count}"
            )
        key = (player, ordinal)
        if key in captures:
            raise ValueError(f"{manifest_path}:{lineno}: duplicate session key {key!r}")
        captures[key] = cap
        ordinals_by_player[player].append(ordinal)

    if not ended:
        raise ValueError(f"{manifest_path}: truncated manifest (missing # end marker)")

    for player, ordinals in ordinals_by_player.items():
        ordered = sorted(ordinals)
        if ordered != list(range(1, len(ordered) + 1)):
            raise ValueError(
                f"{manifest_path}: noncontiguous ordinals for {player!r}: {ordered}"
            )

    actual_files = {p.name for p in directory.iterdir() if p.name.endswith(".pkt")}
    extra_files = sorted(actual_files - expected_files)
    if extra_files:
        raise ValueError(
            f"{directory}: unlisted .pkt file(s) not in manifest: {', '.join(extra_files)}"
        )

    return captures


def resolve_ignores(specs: list[str], names: list[str]) -> set[int]:
    name_to_id = {name: idx for idx, name in enumerate(names)}
    ignored: set[int] = set()
    for spec in specs:
        for raw in spec.split(","):
            token = raw.strip()
            if not token:
                continue
            if INT_RE.match(token):
                ignored.add(int(token, 10))
            elif token in name_to_id:
                ignored.add(name_to_id[token])
            else:
                raise ValueError(f"unknown play clientbound packet name or id: {token!r}")
    return ignored


def _fmt_pair(a: str, b: str) -> str:
    return a if a == b else f"{a} vs {b}"


def compare_connection(
    label: str,
    a_packets: list[PacketRecord],
    b_packets: list[PacketRecord],
    ignored_ids: set[int],
    names: list[str],
) -> tuple[str, bool, int]:
    a_filt = [p for p in a_packets if p.packet_id not in ignored_ids]
    b_filt = [p for p in b_packets if p.packet_id not in ignored_ids]
    a_skipped = len(a_packets) - len(a_filt)
    b_skipped = len(b_packets) - len(b_filt)
    total_skipped = a_skipped + b_skipped

    for ra, rb in zip(a_filt, b_filt):
        if (ra.tick, ra.packet_id, ra.length, ra.digest) != (
            rb.tick,
            rb.packet_id,
            rb.length,
            rb.digest,
        ) or (not ignored_ids and ra.seq != rb.seq):
            seq_s = _fmt_pair(str(ra.seq), str(rb.seq))
            tick_s = _fmt_pair(str(ra.tick), str(rb.tick))
            id_s = _fmt_pair(packet_label(ra.packet_id, names), packet_label(rb.packet_id, names))
            return (
                f"{label}: DIFFERS seq={seq_s} tick={tick_s} id={id_s} "
                f"len={ra.length} vs {rb.length} hash={ra.digest} vs {rb.digest}",
                False,
                total_skipped,
            )

    if len(a_filt) != len(b_filt):
        if len(a_filt) > len(b_filt):
            extra = a_filt[len(b_filt)]
            side = "A"
            lens = f"{extra.length} vs <eof>"
            hashes = f"{extra.digest} vs <eof>"
        else:
            extra = b_filt[len(a_filt)]
            side = "B"
            lens = f"<eof> vs {extra.length}"
            hashes = f"<eof> vs {extra.digest}"
        return (
            f"{label}: DIFFERS count={len(a_filt)} vs {len(b_filt)} "
            f"(first extra in {side}: seq={extra.seq} tick={extra.tick} "
            f"id={packet_label(extra.packet_id, names)} len={lens} hash={hashes})",
            False,
            total_skipped,
        )

    if ignored_ids:
        return (
            f"{label}: retained packets identical ({len(a_filt)} kept, "
            f"{a_skipped}/{b_skipped} ignored)",
            True,
            total_skipped,
        )
    return f"{label}: identical ({len(a_filt)} packets)", True, 0


def compare_dirs(
    dir_a: Path,
    dir_b: Path,
    ignored_ids: set[int] | None = None,
    names: list[str] | None = None,
    *,
    run_id_a: str | None = None,
    run_id_b: str | None = None,
) -> tuple[list[str], bool]:
    ignored = ignored_ids or set()
    pkt_names = load_play_clientbound_names() if names is None else names
    a_map = load_directory(dir_a, expected_run_id=run_id_a)
    b_map = load_directory(dir_b, expected_run_id=run_id_b)

    keys = sorted(set(a_map) | set(b_map))
    if not keys:
        return ["no packet recordings found", "DIFFERENT"], False

    ordinals_per_player: dict[str, set[int]] = defaultdict(set)
    for player, ordinal in keys:
        ordinals_per_player[player].add(ordinal)

    report: list[str] = []
    all_same = True
    total_ignored = 0
    for player, ordinal in keys:
        multi = len(ordinals_per_player[player]) > 1 or ordinal > 1
        label = f"{player} #{ordinal}" if multi else player
        cap_a = a_map.get((player, ordinal))
        cap_b = b_map.get((player, ordinal))
        if cap_a is None:
            all_same = False
            report.append(f"{label}: DIFFERS only in B ({len(cap_b.packets)} packets)")
        elif cap_b is None:
            all_same = False
            report.append(f"{label}: DIFFERS only in A ({len(cap_a.packets)} packets)")
        else:
            line, same, skipped = compare_connection(
                label, cap_a.packets, cap_b.packets, ignored, pkt_names
            )
            report.append(line)
            all_same = all_same and same
            total_ignored += skipped

    if all_same:
        if ignored:
            report.append(
                f"IDENTICAL (retained only; {total_ignored} ignored packet(s) skipped)"
            )
        else:
            report.append("IDENTICAL")
    else:
        report.append("DIFFERENT")
    return report, all_same


def build_parser() -> argparse.ArgumentParser:
    wall_lines = "\n".join(
        f"  {name:<20} (id {pid:>3}): {desc}" for name, pid, desc in WALL_CLOCK_PACKETS
    )
    epilog = (
        "Play clientbound packets carrying wall-clock values (none ignored by default):\n"
        f"{wall_lines}\n\n"
        "Notes:\n"
        "  - --ignore skips entire packet IDs; a filtered match proves equality only of\n"
        "    retained records (not skipped fields such as roster, chat, or advancements).\n"
        "  - Exclude clock-dependent commands (e.g. /debug stop) and network-driven chunk\n"
        "    batching from parity workloads rather than broadly ignoring chat or chunks.\n\n"
        "Example:\n"
        "  packet_capture_compare.py run_a run_b --ignore keep_alive,pong_response"
    )
    parser = argparse.ArgumentParser(
        description=(
            "Compare two per-connection outgoing play packet capture directories "
            "(matched by player name and connection ordinal) and print the first "
            "difference per connection."
        ),
        epilog=epilog,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("a", type=Path, help="first capture directory")
    parser.add_argument("b", type=Path, help="second capture directory")
    parser.add_argument(
        "--run-id-a",
        default=None,
        metavar="ID",
        help="require manifest run id of directory A to equal ID",
    )
    parser.add_argument(
        "--run-id-b",
        default=None,
        metavar="ID",
        help="require manifest run id of directory B to equal ID",
    )
    parser.add_argument(
        "--ignore",
        action="append",
        default=[],
        metavar="PACKET",
        help=(
            "play clientbound packet name or numeric id to ignore (repeatable or "
            "comma-separated; off by default)"
        ),
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        names = load_play_clientbound_names()
        ignored = resolve_ignores(args.ignore, names)
        report, same = compare_dirs(
            args.a,
            args.b,
            ignored_ids=ignored,
            names=names,
            run_id_a=args.run_id_a,
            run_id_b=args.run_id_b,
        )
    except (ValueError, OSError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    print("\n".join(report))
    return 0 if same else 1


if __name__ == "__main__":
    sys.exit(main())
