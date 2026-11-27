"""Read-only cgroup v2 telemetry for the ignored-test shards; logs contain only fixed fields."""
from contextlib import ExitStack, contextmanager, suppress
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory, TemporaryFile
from time import monotonic_ns, sleep
from typing import NamedTuple, TextIO

CPU_FIELDS = ("usage_usec", "user_usec", "system_usec", "nr_periods", "nr_throttled", "throttled_usec")
MEMORY_FIELDS = ("current", "anon", "file")
SAMPLE_INTERVAL_SECONDS = 1
END_CPU_READ_SECONDS = 1
COUNTER_DIGITS = 20  # decimal digits in a u64
SNAPSHOT_FIELDS = 2 + len(MEMORY_FIELDS) + 2 * len(CPU_FIELDS)
SNAPSHOT_BYTES = SNAPSHOT_FIELDS * (COUNTER_DIGITS + 1)  # fields plus separators and final newline
Counter = int | None


class PhaseSnapshot(NamedTuple):
    attempts: int
    maxima: tuple[Counter, ...]
    span_ms: Counter
    first_cpu: tuple[Counter, ...]
    last_cpu: tuple[Counter, ...]


NO_CPU = (None,) * len(CPU_FIELDS)
NO_SNAPSHOT = PhaseSnapshot(0, (None,) * len(MEMORY_FIELDS), None, NO_CPU, NO_CPU)


def _read(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError:
        return b""


def _counter(value: bytes) -> Counter:
    return int(value) if len(value) <= COUNTER_DIGITS and value.isdigit() else None


def _stat(path: Path, fields: tuple[str, ...]) -> dict[str, Counter]:
    keys = {field.encode("ascii"): field for field in fields}
    counters = dict.fromkeys(fields)
    for line in _read(path).splitlines():
        parts = line.split()
        if len(parts) == 2 and (field := keys.get(parts[0])) is not None:
            counters[field] = _counter(parts[1])
    return counters


def _metric_fields(fields: dict[str, Counter]) -> str:
    return " ".join(f"{key}={value if value is not None else 'unavailable'}" for key, value in fields.items())


def _sample_phase(cgroup: Path, snapshot: Path) -> None:
    staging = snapshot.with_suffix(".pending")
    attempts = 0
    maxima = dict.fromkeys(MEMORY_FIELDS)
    first_at, first_cpu = None, NO_CPU
    while True:
        values = _stat(cgroup / "memory.stat", ("anon", "file"))
        values["current"] = _counter(_read(cgroup / "memory.current").strip())
        cpu = tuple(_stat(cgroup / "cpu.stat", CPU_FIELDS).values())
        at = monotonic_ns()
        for field, value in values.items():
            if value is not None and (maxima[field] is None or value > maxima[field]):
                maxima[field] = value
        if first_at is None:
            first_at, first_cpu = at, cpu
        span_ms = (at - first_at) // 1_000_000 if attempts else None
        attempts += 1
        fields = (attempts, *maxima.values(), span_ms, *first_cpu, *cpu)
        # Never truncate the published file or create a directory: the parent can
        # read either generation and can remove the scratch path while we sample.
        staging.write_bytes((" ".join("-" if value is None else str(value) for value in fields) + "\n").encode("ascii"))
        staging.replace(snapshot)
        sleep(SAMPLE_INTERVAL_SECONDS)


def _read_snapshot(snapshot: Path) -> PhaseSnapshot:
    try:
        with snapshot.open("rb", buffering=0) as observation:
            raw = observation.read(SNAPSHOT_BYTES + 1)
    except OSError:
        return NO_SNAPSHOT
    if len(raw) > SNAPSHOT_BYTES or not raw.endswith(b"\n"):
        return NO_SNAPSHOT
    fields = raw[:-1].split(b" ")
    if len(fields) != SNAPSHOT_FIELDS:
        return NO_SNAPSHOT
    values = []
    for field in fields:
        value = _counter(field)
        if value is None and field != b"-":
            return NO_SNAPSHOT
        values.append(value)
    span = 1 + len(MEMORY_FIELDS)
    if not (attempts := values[0]) or (values[span] is None) != (attempts == 1):
        return NO_SNAPSHOT
    return PhaseSnapshot(attempts, tuple(values[1:span]), values[span],
                         tuple(values[span + 1:-len(CPU_FIELDS)]), tuple(values[-len(CPU_FIELDS):]))


def _start_observer(*arguments: str, stdout: int | TextIO = subprocess.DEVNULL) -> subprocess.Popen:
    # No shard, Cargo, or workflow pipe belongs to a reader that might outlive its caller.
    return subprocess.Popen([sys.executable, __file__, *arguments],
                            stdin=subprocess.DEVNULL, stdout=stdout, stderr=subprocess.DEVNULL, close_fds=True)


@contextmanager
def sample_command(cgroup: Path, batch: str, phase: str):
    """Sample the job cgroup around one command (batch and phase are supplied by the shard runner).

    The maxima are independent observations, not additive or isolated to the command's processes.
    File cache and other memory still charged from earlier phases count too. memory.peak is not used.
    CPU deltas cover the span between completed snapshots, not the command's exact boundaries.
    The attempt count includes only completed snapshots, even if a counter in one is unavailable.
    """
    reader = None
    reading = NO_SNAPSHOT
    with ExitStack() as cleanup:
        try:
            with suppress(OSError):
                scratch = Path(cleanup.enter_context(TemporaryDirectory(ignore_cleanup_errors=True)))
                live = scratch / "live"
                live.mkdir()
                snapshot = live / "latest"
                reader = _start_observer("--sample-phase", str(cgroup), str(snapshot))
            yield
        finally:
            if reader is not None:
                try:
                    reading = _read_snapshot(snapshot)
                finally:
                    # Do not wait, including through Popen.__exit__: a cgroup read
                    # may still be outstanding after the kill request.
                    with suppress(OSError):
                        reader.kill()
                    with suppress(OSError):
                        # Cut off the writer's path before deleting its files. The outer
                        # cleanup also catches a file operation already in flight here.
                        shutil.rmtree(live.rename(scratch / "retired"), ignore_errors=True)
            samples, maxima, span_ms, first_cpu, last_cpu = reading
            metrics = {"memory_sample_attempts": samples, "sample_interval_seconds": SAMPLE_INTERVAL_SECONDS,
                       "cpu_sample_span_ms": span_ms,
                       **{f"memory_{field}_observed_max_bytes": value for field, value in zip(MEMORY_FIELDS, maxima)}}
            for field, before, after in zip(CPU_FIELDS, first_cpu, last_cpu):
                metrics[f"cpu_{field}_delta"] = (after - before if span_ms is not None and before is not None
                                                  and after is not None and after >= before else None)
            with suppress(OSError):
                # Telemetry must not replace a subprocess error with a logging error.
                print(f"data-leg cgroup-phase batch={batch} phase={phase} scope=job_cgroup_during_command "
                      + _metric_fields(metrics), flush=True)


def _observe_job_cpu(cgroup: str) -> str:
    # The reader gets no workflow pipes. A kernel read that outlives the deadline
    # cannot delay step exit via either a pipe or a wait (including Popen.__exit__).
    try:
        with TemporaryFile(mode="w+t", encoding="ascii") as observation:
            reader = _start_observer("--read-cpu", cgroup, stdout=observation)
            try:
                if reader.wait(timeout=END_CPU_READ_SECONDS) == 0:
                    observation.seek(0)
                    return observation.read()
            finally:
                if reader.returncode is None:
                    reader.kill()
    except (OSError, subprocess.TimeoutExpired):
        pass
    return _metric_fields(dict.fromkeys(CPU_FIELDS))


if __name__ == "__main__":
    if sys.argv[1] == "--sample-phase":
        _sample_phase(Path(sys.argv[2]), Path(sys.argv[3]))
    elif sys.argv[1] == "--read-cpu":
        sys.stdout.write(_metric_fields(_stat(Path(sys.argv[2]) / "cpu.stat", CPU_FIELDS)))
    else:
        print("data-leg cgroup-cpu scope=whole_job_cgroup_cumulative " + _observe_job_cpu(sys.argv[1]))
