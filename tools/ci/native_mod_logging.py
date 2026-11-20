#!/usr/bin/env python3
"""Run two API-only native SDK mods on the client and dedicated server and check latest.log."""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tools/ci/fn-replace-check/mod_logging"
IDS = ("fnr_log_alpha", "fnr_log_beta")
MARKER = "mod-logger fixture: "


def check_side(args, scratch, sources, side):
    env = dict(os.environ, FNR_SIDE=side, FNR_ROUTE="prebuilt-api",
               FNR_MODS="alpha beta", FNR_MOD_ROOT=str(sources))
    env.pop("FNR_KEEP", None)
    subprocess.run(
        ["bash", ROOT / "tools/ci/fn_replace_check.sh", args.launcher, args.sdk_base, scratch],
        env=env, cwd=ROOT, check=True,
    )
    run = scratch / "run"
    run.mkdir()
    homes = {
        "SUNRISE_HOME": scratch / "home",
        "SUNRISE_LAUNCHER_HOME": scratch / "launcher",
        "XDG_DATA_HOME": scratch / "data",
        "XDG_CONFIG_HOME": scratch / "config",
    }
    env.update({key: str(path) for key, path in homes.items()})
    env.update(SUNRISE_NO_AUDIO="1", SUNRISE_LAUNCHER_NO_SELF_UPDATE="1")
    cmd = [scratch / "engine/sunrise-engine", "--resources", args.resources]
    if side == "server":
        (run / "eula.txt").write_text("eula=true\n")
        (run / "server.properties").write_text(
            "server-port=0\nserver-ip=127.0.0.1\nonline-mode=false\n"
            "view-distance=2\nsimulation-distance=2\nlevel-type=minecraft:flat\n"
        )
        cmd += ["--dedicated", "--nogui"]
        latest = run / "logs/latest.log"
        thread = "Server thread"
        runtime = ("before rollback", "after rollback")
    else:
        cmd += ["--offscreen"]
        latest = homes["SUNRISE_HOME"] / "logs/latest.log"
        thread = "Render thread"
        runtime = ("client first", "client second")
    expected = [f"[{thread}/INFO]: [{mod}] {MARKER}registered 17" for mod in IDS]
    expected += [f"[{thread}/{level}]: [{mod}] {MARKER}{message}"
                 for mod in IDS for level, message in zip(("WARN", "ERROR"), runtime, strict=True)]
    stdout = scratch / "game-output.txt"
    with stdout.open("w") as output, subprocess.Popen(
        cmd, cwd=run, env=env, stdout=output, stderr=subprocess.STDOUT, stdin=subprocess.PIPE,
    ) as game:
        try:
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline and game.poll() is None:
                if latest.is_file() and latest.read_text(errors="replace").count(MARKER) >= len(expected):
                    break
                time.sleep(0.05)
            else:
                raise AssertionError(f"{side}: fixture did not finish; see {stdout}")
            if side == "server":
                game.communicate(b"stop\n", timeout=30)
                assert game.returncode == 0, f"dedicated server shutdown failed; see {stdout}"
        finally:
            if game.poll() is None:
                game.kill()
    log = latest.read_text(errors="replace")
    actual = [re.sub(r"^\[\d\d:\d\d:\d\d\] ", "", line)
              for line in log.splitlines() if MARKER in line]
    assert actual == expected, f"{side}: expected {expected!r}; got {actual!r}"
    for mod in IDS:
        assert f"mod {mod} loading warning" not in log
    assert MARKER not in stdout.read_text(), f"{side}: diagnostic leaked to console"
    print(f"{side}: SDK mod log IDs, levels, messages and order matched", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("launcher", type=lambda path: Path(path).resolve())
    parser.add_argument("sdk_base", type=lambda path: Path(path).resolve())
    parser.add_argument("resources", type=lambda path: Path(path).resolve())
    args = parser.parse_args()
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    target.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="native-logging-", dir=target))
    print(f"Native logging fixture scratch: {scratch}", flush=True)
    sources = scratch / "sources"
    shutil.copytree(FIXTURE, sources / "alpha")
    shutil.copytree(FIXTURE, sources / "beta")
    manifest = sources / "beta/sunrise-mod.toml"
    manifest.write_text(
        manifest.read_text().replace('id = "fnr_log_alpha"', 'id = "fnr_log_beta"')
        + '\n[dependencies]\nfnr_log_alpha = "^0.1"\n'
    )
    cargo = sources / "beta/Cargo.toml"
    cargo.write_text(cargo.read_text().replace('"fnr-logging-fixture"', '"fnr-logging-fixture-peer"'))
    for side in ("client", "server"):
        check_side(args, scratch / side, sources, side)
    shutil.rmtree(scratch)


if __name__ == "__main__":
    main()
