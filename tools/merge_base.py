"""The pull request's merge base, shared by every gate that takes --base.

`pull_request_base` picks the ref a gate compares HEAD with (the first parent of CI's pull-request merge).
check_layers.py, check_addresses.py and tools/hotspots/ use it.
"""

from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, encoding="utf-8", check=True, cwd=ROOT).stdout


def lines(text):
    return text.count("\n") + (1 if text and not text.endswith("\n") else 0)


def text_at(rev, path, root=None):
    """`path`'s text at `rev`, or None when `rev` has no such file. A file that is there but can't be read (a partial
    clone's blob whose download failed) stops the check after a second try: an unread baseline is never taken for a
    missing one, which would skip the comparison."""
    root = root or ROOT
    listed = subprocess.run(["git", "ls-tree", "--name-only", rev, "--", path], cwd=root, capture_output=True,
                            encoding="utf-8")
    if listed.returncode != 0:
        raise SystemExit(f"can't list {path} at {rev}: {listed.stderr.strip()}")
    if path not in listed.stdout.splitlines():
        return None
    for _ in range(2):
        shown = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=root, capture_output=True, encoding="utf-8")
        if shown.returncode == 0:
            return shown.stdout
    raise SystemExit(f"{path} is at {rev} but can't be read: {shown.stderr.strip()}")


PULL_REQUEST_MERGE_HELP = ("HEAD is the pull request merged into its base branch (CI's pull_request checkout): "
                           "compare with that merge's first parent, the base branch as it is now")


def pull_request_base(base, merged, root=None):
    """(the ref to compare HEAD with, error). With `merged`, HEAD merges the pull request into a descendant of `base`
    (the event's base, where the branch last met main): main has changed since, and the merge holds those changes too,
    so only the merge's own change, against its first parent, is the pull request's. Shared by the gates that take
    --base (check_addresses.py, check_layers.py, check_launcher_mirrors.py, check_port_map_paths.py
    and tools/hotspots/)."""
    if not merged:
        return base, None
    root = root or ROOT  # read at the call, as the two-argument callers always had it
    listed = subprocess.run(["git", "rev-list", "--parents", "-n", "1", "HEAD"], cwd=root, capture_output=True,
                            encoding="utf-8")
    parents = listed.stdout.split()[1:] if listed.returncode == 0 else []
    if len(parents) != 2 or subprocess.run(["git", "merge-base", "--is-ancestor", base, parents[0]],
                                           cwd=root, capture_output=True).returncode != 0:
        return base, "--pull-request-merge needs HEAD to merge the pull request into a descendant of --base."
    return parents[0], None


def branch_base(base, root=None):
    """Where HEAD's branch left `base`, the commit a gate reads its baseline file at. A `base` git can't read, or one
    that shares no history with HEAD, stops the check: an unread baseline is never taken for an empty one."""
    root = root or ROOT
    known = subprocess.run(["git", "cat-file", "-e", f"{base}^{{commit}}"], cwd=root, capture_output=True,
                           encoding="utf-8")
    if known.returncode != 0:
        raise SystemExit(f"git cannot read {base}, so the baseline at the base cannot be read: {known.stderr.strip()}")
    found = subprocess.run(["git", "merge-base", base, "HEAD"], cwd=root, capture_output=True, encoding="utf-8")
    if found.returncode != 0:
        raise SystemExit(f"{base} and HEAD have no merge base, so the baseline at the base cannot be read: "
                         f"{found.stderr.strip()}")
    return found.stdout.strip()
