"""Disposable git history for tests of the gates that take `--base`, isolated from the outer git configuration."""
import os
from pathlib import Path
import subprocess

OPTIONS = ("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false",
           "-c", "tag.gpgsign=false", "-c", f"core.hooksPath={os.devnull}")


def clean_env(**extra):
    """The process environment without the caller's `GIT_*` variables (`GIT_DIR`, `GIT_WORK_TREE`,
    `GIT_CONFIG_PARAMETERS`...), and with the user's and the system's git configuration (signing, hooks, identity)
    switched off, so a disposable repository never sees the checkout it runs in."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    env.update(extra)
    return env


def git(root, *arguments, check=True):
    return subprocess.run(["git", *OPTIONS, *arguments], cwd=root, env=clean_env(),
                          capture_output=True, text=True, encoding="utf-8", check=check)


def merge_pull_request(root, base, tracked, main_change, pr_change):
    """Leave HEAD at CI's test merge of a pull request into a main that moved on since `base`.

    The pull request branches from `base` and makes `pr_change`; main then makes `main_change`; HEAD is
    `git merge --no-ff` of the pull request into that main, so its first parent is main as it is now. `tracked` are
    the paths (relative to `root`) the commits take. Returns (the pull request's commit, main's commit)."""
    git(root, "checkout", "-q", "--detach", base)
    git(root, "checkout", "-q", "-b", "pull-request")
    pr_change()
    git(root, "add", "-A", "--", *tracked)
    git(root, "commit", "-q", "--allow-empty", "-m", "pull request")
    pr = git(root, "rev-parse", "HEAD").stdout.strip()
    git(root, "checkout", "-q", "--detach", base)
    main_change()
    git(root, "add", "-A", "--", *tracked)
    git(root, "commit", "-q", "--allow-empty", "-m", "main moved on")
    main = git(root, "rev-parse", "HEAD").stdout.strip()
    git(root, "merge", "--no-ff", "-q", "-m", "Merge the pull request", "pull-request")
    return pr, main


def write(root, path, text):
    (Path(root) / path).parent.mkdir(parents=True, exist_ok=True)
    (Path(root) / path).write_text(text, encoding="utf-8", newline="\n")
