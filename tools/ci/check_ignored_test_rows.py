#!/usr/bin/env python3
"""verify-static's guard for tools/ci/ignored-tests/: read every policy file as the ignored-test readers do.

Fails on a malformed row, a test listed in two files, a folder with no rows or with a file other than `*.txt`
and README.md, and a file at the retired single-list path. The shared reader validates both `needs`
and `needs-plain` fixture rows; neither is a known failure. Every `daily` row must name a package of a workspace (the game's) and
one of its test targets, and some leg of the daily run's matrix must run it: its package's shard, or the `daily-rows`
leg (tools/ci/ignored_shards.py), so no moved test runs nowhere. A `windows` row instead names a Windows-only integration
test in the game workspace and runs in the Windows job. Builds only the source-inventory helper, not the game or SDK.

Every `#[ignore]`d test that no daily shard runs (rowless_problems) has a row of some mark: `daily`, a known
failure, `needs`, `manual`, ...; otherwise it would run nowhere unseen. In a shard's own targets a rowless
`#[ignore]` is fine: the shard runs it with `--include-ignored`, and it must pass.

A test that reaches `prepared_test_root::prepared_root` (resolved by name through the helpers, aliases and re-exports
of its own target, see crates/sunrise-game-discovery/README.md) must be a daily test; with `--base COMMIT`, no
function may newly reach the older readers of the prepared root (`SUNRISE_TEST_RESOURCES`, `SUNRISE_RESOURCES`,
`SUNRISE_MESH_TEST_RESOURCES`, `SUNRISE_FISHING_RESOURCES`, `.local/sunrise-client`).
`--pull-request-merge` (CI's pull-request checkout) compares with the merge's first parent, as check_addresses.py does.
The base is scanned only for the targets a change can affect, from a partial archive of its tree.

In addition to the legacy physical-source checks below, the canonical inventory checks package, target and
full logical test path.

The legacy rows and physical-source checks agree both ways (read with tools/ci/rust_tests.py): every function marked
`#[ignore = "daily"]` in a workspace package's sources has a daily row of that package whose test path ends in its
name, and every daily row names a function of that name marked so in its package's sources. A source file compiled
into two targets (sunrise-client-render's gfx/sunrise_fan.rs, also built by its `sunrise_shader` test) has one
attribute and a row per target; both rows find it. The limit: rows carry no file and the source has no module path
(a `#[path]` or a game area's derived module can place a file anywhere), so the match is by package and function
name. Two marked functions of one name in one package are satisfied by one row naming either; the daily run, which
runs each row by its exact path, still fails a row whose test does not run.
"""
import argparse
import re
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import merge_base
from tools.ci import ignored_inventory, ignored_shards, ignored_tests, rust_tests, vendored_yaml

yaml = vendored_yaml.load()

ROOT = Path(__file__).resolve().parents[2]
WINDOWS_WORKFLOW = ".github/workflows/windows-runtime-leases.yml"
OWN_SDK_WORKFLOW = ".github/workflows/book-quickstart.yml"
# Prepared test data: the one required way to get it, and what only a legacy reader spells. Resolved over the
# tests' own sources by sunrise-game-discovery's `--references` (crates/sunrise-game-discovery/src/references.rs).
BOUNDARY = "crates/sunrise-core/src/prepared_test_root.rs::prepared_root"
LEGACY_LITERALS = (
    "SUNRISE_TEST_RESOURCES",
    "SUNRISE_RESOURCES",
    "SUNRISE_MESH_TEST_RESOURCES",
    "SUNRISE_FISHING_RESOURCES",
)
LEGACY_FRAGMENTS = (".local/sunrise-client",)
# Fragments of the vocabulary's exports that discovery includes into a target: a change there can reach any of them.
EXPORTS = "crates/sunrise-mod-vocab/src/exports/"
ADD_ROW = ("add its row: `python3 tools/ci/move_to_daily.py apply LIST` (its docstring has the list format), or write "
           "`PACKAGE TEST TARGET daily REASON` into tools/ci/ignored-tests/daily-PACKAGE-TOPIC.txt")


def daily_problems(rows, targets, matrix):
    """Why each daily row would not run in the daily matrix `matrix` (shard names), given the workspace's targets."""
    problems = []
    groups = ignored_tests.daily_rows_groups(rows)
    windows_wired = not any(row.reason == ignored_tests.WINDOWS_REASON for row in rows) or windows_daily_runs(ROOT)
    for row in rows:
        where = f"{row.file}: {row.package} {row.test}"
        if row.package not in targets:
            problems.append(f"{where}: {row.package} is not a workspace package")
        elif row.reference not in targets[row.package]:
            problems.append(f"{where}: {row.package} has no target {row.reference}")
        elif row.reason == ignored_tests.WINDOWS_REASON:
            if not windows_wired:
                problems.append(f"{where}: daily.yml must call {WINDOWS_WORKFLOW}'s runtime-leases job and run "
                                "ignored_tests.py --windows-rows --run there")
        elif row.reason == ignored_tests.OWN_SDK_REASON:
            if not own_sdk_daily_runs(ROOT, row):
                problems.append(f"{where}: daily.yml must call {OWN_SDK_WORKFLOW} with own_sdk and run this "
                                "integration test by exact name there")
        elif ignored_tests.shard_runs(row.package, row.reference):
            if not any(ignored_tests.parse_shard(shard)[0] == row.package for shard in matrix):
                problems.append(f"{where}: no leg runs {row.package}'s shard")
        elif (leg := ignored_shards.daily_rows_leg(row.package)) not in matrix or \
                row.test not in groups.get((row.package, row.reference), ()):
            problems.append(f"{where}: no leg runs it (the {leg} leg is missing)")
    return problems


def own_sdk_daily_runs(root, row):
    """Recognize the daily call and exact test step that run with the calling run's SDK."""
    if not row.reference.startswith("test:"):
        return False
    daily_path = root / ".github/workflows/daily.yml"
    quickstart_path = root / OWN_SDK_WORKFLOW
    if not daily_path.is_file() or not quickstart_path.is_file():
        return False
    daily = yaml.load(daily_path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader) or {}
    call = daily.get("jobs", {}).get("book-quickstart", {})
    # The call's own condition (skips, leg selection) is the daily's routing, not this row's concern.
    needs = call.get("needs") or []
    if (call.get("uses") != f"./{OWN_SDK_WORKFLOW}"
            or not {"resolve", "sdk"} <= set([needs] if isinstance(needs, str) else needs)
            or call.get("with", {}).get("own_sdk") != "true"
            or call.get("with", {}).get("commit") != "${{ needs.resolve.outputs.commit }}"
            or re.sub(r"^\$\{\{(.*)\}\}$", r"\1", call.get("if", "").strip()).strip() == "false"):
        return False
    enabled = (None, "true", "${{ true }}")
    quickstart = yaml.load(quickstart_path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader) or {}
    job = quickstart.get("jobs", {}).get("quickstart", {})
    command = ["cargo", "test", "--locked", "-p", row.package, "--test", row.reference[5:],
               "--", "--ignored", "--exact", row.test]
    return job.get("if") in enabled and any(
        step.get("if") in ("inputs.own_sdk", "${{ inputs.own_sdk }}") and step.get("run", "").split() == command
        for step in job.get("steps", ()))


def windows_daily_runs(root):
    """Verify the daily's named call and the enabled step whose verdict the summary reads."""
    daily_path = root / ".github/workflows/daily.yml"
    windows_path = root / WINDOWS_WORKFLOW
    if not daily_path.is_file() or not windows_path.is_file():
        return False
    # BaseLoader preserves Actions' unquoted `on` key rather than treating it as a YAML 1.1 boolean.
    daily = yaml.load(daily_path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader) or {}
    windows = yaml.load(windows_path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader) or {}
    events = windows.get("on") or ()
    if not (events == "workflow_call" if isinstance(events, str) else "workflow_call" in events):
        return False
    call = (daily.get("jobs") or {}).get("windows-runtime-leases") or {}
    if call.get("uses") != f"./{WINDOWS_WORKFLOW}":
        return False
    # Recognize the daily's non-skipped resolve path and the commit it passes; do not guess whether arbitrary
    # expressions are true. Likewise the called job either is unconditional or accepts a nonempty commit.
    enabled = (None, "true", "${{ true }}")
    # A full daily (no break-glass leg selection, `legs` empty) always runs it.
    if call.get("if") not in (*enabled, "needs.resolve.outputs.skip == 'false'",
                              "needs.resolve.outputs.skip == 'false' && (needs.resolve.outputs.legs == '' || "
                              "contains(fromJSON(needs.resolve.outputs.legs), 'windows-runtime-leases'))"):
        return False
    job = (windows.get("jobs") or {}).get("runtime-leases") or {}
    if (job.get("if") not in enabled and
            (job.get("if") != "inputs.commit != '' || github.event_name == 'workflow_dispatch'" or
             call.get("with", {}).get("commit") != "${{ needs.resolve.outputs.commit }}")):
        return False
    if job.get("runs-on") != "windows-latest" or job.get("needs"):
        return False
    default_cwd = next((cwd for scope in (job, windows)
                        if (cwd := scope.get("defaults", {}).get("run", {}).get("working-directory"))), ".")
    return any(step.get("if") in enabled and step.get("working-directory", default_cwd) == "." and re.fullmatch(
        r"python3?\s+tools/ci/ignored_tests\.py\s+--windows-rows\s+--run\s+--report\s+\S+",
        step.get("run", "").strip()) for step in job.get("steps", ()))


def windows_target_problems(rows, root=ROOT):
    """Only an actual game-workspace tests/NAME.rs crate root gated by Windows can leave the Linux legs."""
    windows = [row for row in rows if row.reason == ignored_tests.WINDOWS_REASON]
    if not windows:
        return []
    members = ignored_shards.workspace_members()
    roots = {(package, target): path for package, target, path, _ in ignored_inventory.target_roots(root)}
    problems = []
    for row in windows:
        where = f"{row.file}: {row.package} {row.test}"
        if row.package not in members:
            continue  # daily_problems already diagnoses unknown packages
        member, workspace = members[row.package]
        if workspace != ".":
            problems.append(f"{where}: `windows` is only supported in the game workspace, not {workspace}")
        elif not row.reference.startswith("test:"):
            problems.append(f"{where}: `windows` requires a Cargo integration test target (test:NAME)")
        else:
            expected = (Path(member) / "tests" / f"{row.reference.removeprefix('test:')}.rs").as_posix()
            path = root / expected
            if roots.get((row.package, row.reference)) != expected or not path.is_file():
                problems.append(f"{where}: `windows` requires the Cargo target root to be {expected}")
            elif not re.match(r"\s*#!\s*\[\s*cfg\s*\(\s*windows\s*\)\s*\]",
                              rust_tests.blank(path.read_text(encoding="utf-8-sig"))):
                problems.append(f"{where}: `windows` requires {expected} to start with #![cfg(windows)] "
                                "after any comments")
    return problems


def source_packages():
    """Each member's source folder, in every workspace -> the packages whose sources it holds
    (the deepest folder wins, and crates/sunrise-client also builds sunrise-client-lib:
    tools/ci/ignored_shards.py SHARED_SOURCES)."""
    return {member: set(ignored_shards.SHARED_SOURCES.get(Path(member).name, (name,))) | {name}
            for name, (member, _) in ignored_shards.workspace_members().items()}


def target_kind(member, path):
    """The kind of target a member's source file builds into, as a daily row's TARGET begins: `test` (tests/),
    `bin` (src/main.rs, src/bin/), `example` or `bench`, else `lib`."""
    inside = path[len(member) + 1:]
    for prefix, kind in (("tests/", "test"), ("src/bin/", "bin"), ("examples/", "example"), ("benches/", "bench")):
        if inside.startswith(prefix):
            return kind
    return "bin" if inside == "src/main.rs" else "lib"


def ignored_functions(root=ROOT, reachable=None):
    """(path, line, function name, packages, target kind, daily) of every unconditionally `#[ignore]`d test
    in tracked workspace sources, restricted to `reachable` relative paths when supplied."""
    owners = source_packages()
    listing = subprocess.run(["git", "ls-files", "-z", "--", "*.rs"], cwd=root, capture_output=True, text=True,
                             check=True)
    found = []
    for path in filter(None, listing.stdout.split("\0")):
        if reachable is not None and path not in reachable:
            continue
        folder = max((member for member in owners if path.startswith(member + "/")), key=len, default=None)
        if folder is None:
            continue
        source = (root / path).read_text(errors="replace")
        if "ignore" not in source:
            continue
        for function in rust_tests.functions(source, rust_tests.DAILY_KEEP):
            if function.is_test and function.is_ignored:
                found.append((path, function.line, function.name, owners[folder], target_kind(folder, path),
                              function.is_daily))
    return found


def daily_functions(root=ROOT):
    """(path, line, function name, packages) of every function marked `#[ignore = "daily"]` in tracked sources."""
    return [(path, line, name, packages) for path, line, name, packages, _, daily in ignored_functions(root) if daily]


def rowless_problems(rows, functions):
    """Ignored tests no daily shard runs and no row of any mark names: they run nowhere, and nothing says so.

    A package's shard runs every test of the targets it builds with `--include-ignored`, so there a rowless
    `#[ignore]` still runs daily and must pass; anywhere else (another package, or a shard package's target its shard
    does not build) it needs a row. Matched by package and function name."""
    named = {(row.package, row.test.rsplit("::", 1)[-1]) for row in rows}
    problems = []
    for path, line, name, packages, kind, _ in functions:
        if any(ignored_tests.shard_runs(package, "lib" if kind == "lib" else f"{kind}:") for package in packages):
            continue
        if not any((package, name) in named for package in packages):
            problems.append(f"{path}:{line}: `{name}` is #[ignore]d in {' or '.join(sorted(packages))}, which no daily "
                            "shard runs, and no row names it, so it runs nowhere: mark it #[ignore = \"daily\"] with its "
                            f"row to run it daily ({ADD_ROW}), or write a `manual` row in "
                            "tools/ci/ignored-tests/manual-PACKAGE.txt saying who runs it and why")
    return problems


def source_problems(rows, functions):
    """Marked functions with no row, and rows with no marked function (matched by package and function name)."""
    problems = []
    named = {(row.package, row.test.rsplit("::", 1)[-1]) for row in rows}
    for path, line, name, packages in functions:
        if not any((package, name) in named for package in packages):
            problems.append(f"{path}:{line}: `{name}` is #[ignore = \"daily\"] but no daily row of "
                            f"{' or '.join(sorted(packages))} names it, so the daily run may never run it: {ADD_ROW}")
    marked = {(package, name) for _, _, name, packages in functions for package in packages}
    for row in rows:
        if (row.package, row.test.rsplit("::", 1)[-1]) not in marked:
            problems.append(f"{row.file}: {row.package} {row.test}: no function `{row.test.rsplit('::', 1)[-1]}` "
                            f"in {row.package}'s sources is #[ignore = \"daily\"]: restore the attribute "
                            "(tools/ci/move_to_daily.py apply) or remove the row")
    return problems


def prepared_problems(tests, records):
    """A test that reaches `prepared_test_root::prepared_root` is a daily test, and no path to it may stay unresolved.

    `tests` are the inventory's; `records` the discovery tool's reference records. Matched by the complete
    identity (package, target, name), the one the daily rows select by."""
    daily = {(test.package, test.target, test.name) for test in tests if test.reason == "daily"}
    problems = []
    for record in records:
        if record["seed"] != "boundary":
            continue
        where = f"{record['path']}:{record['line']}: {record['package']} {record['name']}"
        if record["kind"] == "unresolved":
            problems.append(f"{where}: `{record['reference']}` may reach {BOUNDARY.rpartition('::')[2]} and the "
                            "static check cannot follow it (a glob or path it cannot resolve, a method whose name "
                            "is shared); call the function by a path the source spells out")
        elif record["test"] and (record["package"], record["target"], record["name"]) not in daily:
            problems.append(f"{where}: reaches {BOUNDARY.rpartition('::')[2]} ({record['chain']}) so it needs "
                            f'prepared data, but is not a daily test: mark it #[ignore = "daily"] with its row '
                            f"({ADD_ROW}), or make it synthetic")
    return problems


def legacy_identity(record):
    """Where legacy prepared-data access sits: the Cargo target, the file, the function and, for a reference the
    tool could not resolve, what it spells. Not the chain, which a refactor may change. The target is part of it: a
    source file already in debt that another target builds does not inherit the first target's exemption."""
    return (record["package"], record["target"], record["path"], record["name"], record["kind"],
            record.get("reference", ""))


def legacy_problems(head, base, renames):
    """Legacy access (`SUNRISE_TEST_RESOURCES`, `SUNRISE_RESOURCES`, `SUNRISE_MESH_TEST_RESOURCES`,
    `SUNRISE_FISHING_RESOURCES`, a path into `.local/sunrise-client`) that the base did not have.

    Existing readers stay until their lane migrates; a function that newly reaches one, or a reader that moves to
    another file, is refused: new tests get prepared data only from the boundary. `renames` maps a base path to
    its path now (`git diff -M`), so a moved file keeps its debt."""
    known = {(package, target, renames.get(path, path), *rest)
             for package, target, path, *rest in (legacy_identity(record) for record in base
                                                  if record["seed"] == "legacy")}
    problems = []
    for record in head:
        if record["seed"] != "legacy" or legacy_identity(record) in known:
            continue
        what = (f"calls `{record['reference']}`, which may read the prepared root" if record["kind"] == "unresolved"
                else f"reaches a read of the prepared root ({record['chain']})")
        problems.append(f"{record['path']}:{record['line']}: {record['package']} {record['name']} {what}, and the "
                        f"base has no such access there: get prepared data from {BOUNDARY} and mark the test "
                        f'#[ignore = "daily"] with its row ({ADD_ROW})')
    return problems


def changes(base):
    """(renames, changed paths) between `base` and the working tree, from one `git diff -M --name-status -z`.

    `renames` maps a base path to its path now. The changed paths hold every added, modified, deleted, copied and
    renamed file, a renamed or copied one under both its paths."""
    diff = subprocess.run(["git", "diff", "-M", "--name-status", "-z", base], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout.split("\0")
    renames, changed = {}, set()
    fields = iter(filter(None, diff))
    for status in fields:
        if status.startswith(("R", "C")):
            old, new = next(fields), next(fields)
            changed.update((old, new))
            if status.startswith("R"):
                renames[old] = new
        else:
            changed.add(next(fields))
    return renames, changed


def top_directory(path):
    """The crate (or top-level) folder a file lives in: `crates/NAME` or the first component."""
    parts = path.split("/")
    if len(parts) < 2:
        return path
    return "/".join(parts[:2]) if parts[0] == "crates" and len(parts) > 2 else parts[0]


def affected_targets(legacy_targets, head_target_files, changed_paths):
    """The targets whose legacy access a change can have altered: all of them when a manifest or the vocabulary's
    export fragments changed (they decide which targets exist and what they include), none when no Rust file
    changed, else those with a changed file (or whose files the head's scan did not list)."""
    if any(path == "Cargo.toml" or path.endswith("/Cargo.toml") or path.startswith(EXPORTS) for path in changed_paths):
        return set(legacy_targets)
    if not any(path.endswith(".rs") for path in changed_paths):
        return set()
    return {target for target in legacy_targets
            if target not in head_target_files or not set(head_target_files[target]).isdisjoint(changed_paths)}


def base_records(base, head_records, head_target_files, changed_paths):
    """The legacy references the tree at commit `base` had.

    A target none of whose files changed has what the head has, so only the targets `affected_targets` names are
    scanned in `base`, with the head's tool over an archive of just their crates (and the manifests that say which
    targets exist). Listing the tree fetches no blobs, and a change with no Rust or manifest file archives nothing.
    A target `base` lacks has no records there."""
    head_legacy = tuple(record for record in head_records if record["seed"] == "legacy")
    legacy_targets = {(record["package"], record["target"]) for record in head_legacy}
    if not legacy_targets:
        return ()
    affected = affected_targets(legacy_targets, head_target_files, changed_paths)
    kept = tuple(record for record in head_legacy if (record["package"], record["target"]) not in affected)
    if not affected:
        return kept
    listing = subprocess.run(["git", "ls-tree", "-r", "--name-only", "-z", base], cwd=ROOT, capture_output=True,
                             text=True, check=True).stdout
    paths = set(filter(None, listing.split("\0")))
    folders = {top_directory(path) for path in paths}
    needed = {path for path in paths if path == "Cargo.toml" or path.endswith("/Cargo.toml")}
    if any(path.startswith(EXPORTS) for path in paths):
        needed.add(EXPORTS.rstrip("/"))
    # The crates of the affected targets' files, and of every changed Rust file (a moved one under its old path).
    wanted = {path for target in affected for path in head_target_files.get(target, ())}
    wanted.update(path for path in changed_paths if path.endswith(".rs"))
    needed.update(top_directory(path) for path in wanted if top_directory(path) in folders)
    with tempfile.TemporaryDirectory() as directory:
        with subprocess.Popen(["git", "archive", "--format=tar", base, "--", *sorted(needed)], cwd=ROOT,
                              stdout=subprocess.PIPE) as archive:
            try:
                with tarfile.open(fileobj=archive.stdout, mode="r|") as tar:
                    tar.extractall(directory, **({"filter": "tar"} if hasattr(tarfile, "tar_filter") else {}))
                # The reader stops at the end-of-archive blocks, short of the zero padding git archive still writes;
                # drain it, or the child blocks on a full pipe while `wait` blocks on the child.
                while archive.stdout.read(65536):
                    pass
            except tarfile.TarError:
                # A failed `git archive` is the cause of a stream that does not parse: report it, not the symptom.
                archive.stdout.close()
                if archive.wait():
                    raise SystemExit(f"git archive {base} failed")
                raise
            archive.stdout.close()
            if archive.wait():
                raise SystemExit(f"git archive {base} failed")
        extracted = Path(directory)
        roots = ignored_inventory.target_roots(extracted, tested=True)
        scannable = affected & {(package, target) for package, target, path, _ in roots if (extracted / path).is_file()}
        scanned = ignored_inventory.references(extracted, BOUNDARY, LEGACY_LITERALS, LEGACY_FRAGMENTS,
                                               targets=scannable) if scannable else ()
    return kept + tuple(record for record in scanned if record["seed"] == "legacy")


def baseline_problems(references, head_target_files, base, merged):
    """`legacy_problems` against `base`, which with `merged` is the first parent of CI's pull-request merge
    (tools/merge_base.py), the base branch as it is now."""
    base, error = merge_base.pull_request_base(base, merged)
    if error:
        return [error]
    renames, changed = changes(base)
    return legacy_problems(references, base_records(base, references, head_target_files, changed), renames)


def parser():
    result = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    result.add_argument("--base", help="a commit whose legacy prepared-data access a change may not add to; "
                                       "without it only the routing check runs")
    result.add_argument("--pull-request-merge", action="store_true", help=merge_base.PULL_REQUEST_MERGE_HELP)
    return result


if __name__ == "__main__":
    arguments = parser().parse_args()
    rows = ignored_tests.listed_entries()
    print(f"{len(rows)} rows in {len({row.file for row in rows})} files of {ignored_tests.POLICY_PATH}/")
    daily = [row for row in rows if row.mark == "daily"]
    problems = daily_problems(daily, ignored_shards.workspace_targets(),
                              [*ignored_shards.expand(ignored_shards.SHARDS), *ignored_shards.DAILY_ROWS_LEGS.values()])
    problems += windows_target_problems(daily)
    reachable = set(ignored_inventory.discovery(ROOT, ("--files",)))
    ignored = ignored_functions(reachable=reachable)
    functions = [(path, line, name, packages) for path, line, name, packages, _, daily in ignored if daily]
    problems += source_problems(daily, functions)
    problems += rowless_problems(rows, ignored)
    inventory = ignored_inventory.tests()
    problems += ignored_inventory.problems(rows, inventory, ignored_inventory.exceptions())
    references, head_target_files = ignored_inventory.references_with_files(ROOT, BOUNDARY, LEGACY_LITERALS,
                                                                            LEGACY_FRAGMENTS)
    problems += prepared_problems(inventory, references)
    if arguments.base:
        problems += baseline_problems(references, head_target_files, arguments.base, arguments.pull_request_merge)
    for problem in problems:
        print(f"::error::{problem}")
    print(f"{len(daily)} daily rows, {len(functions)} #[ignore = \"daily\"] functions, {len(ignored)} ignored tests; "
          f"{len(problems)} problems")
    sys.exit(1 if problems else 0)
