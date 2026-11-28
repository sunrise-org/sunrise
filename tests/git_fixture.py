"""Git setup for repositories created by tests, never for the project checkout."""
import os
import subprocess


def isolated_git_env(**overrides: str) -> dict[str, str]:
    """Keep fixture Git subprocesses independent of the caller's config and checkout."""
    return {**{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1", **overrides}


def git_in(folder):
    """Return a Git command bound to the fixture directory."""
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=folder, text=True, stderr=subprocess.PIPE,
                                       env=isolated_git_env()).strip()
    return git


def merge_pull_request(root, base, tracked, main_change, pr_change):
    """Leave HEAD at CI's test merge of a pull request into a main that moved on since `base`.

    The pull request branches from `base` and makes `pr_change`; main then makes `main_change`; HEAD is
    `git merge --no-ff` of the pull request into that main, so its first parent is main as it is now. `tracked` are
    the paths (relative to `root`) the commits take. Returns (the pull request's commit, main's commit)."""
    git = git_in(root)
    identity = ("-c", "user.name=Test", "-c", "user.email=test@example.invalid")
    git("checkout", "-q", "--detach", base)
    git("checkout", "-q", "-b", "pull-request")
    pr_change()
    git("add", "-A", "--", *tracked)
    git(*identity, "commit", "-q", "--allow-empty", "-m", "pull request")
    pr = git("rev-parse", "HEAD")
    git("checkout", "-q", "--detach", base)
    main_change()
    git("add", "-A", "--", *tracked)
    git(*identity, "commit", "-q", "--allow-empty", "-m", "main moved on")
    main = git("rev-parse", "HEAD")
    git(*identity, "merge", "--no-ff", "-q", "-m", "Merge the pull request", "pull-request")
    return pr, main


def decoy_git_env(folder):
    """Simulate a caller with its own clean, disposable checkout selected."""
    folder.mkdir()
    git = git_in(folder)
    git("init", "-q", "-b", "main")
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-qm", "decoy", "--allow-empty")
    return isolated_git_env(GIT_DIR=str(folder / ".git"), GIT_WORK_TREE=str(folder))
