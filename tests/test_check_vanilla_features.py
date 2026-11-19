"""The vanilla-features guard on a small Cargo workspace laid out like the game's."""
import importlib.util
import json
import os
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("check_vanilla_features", ROOT / "tools/check_vanilla_features.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

LIB = '[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n{rest}'


def workspace(root: Path, game_audio: str, client_features: str, client_deps: str = "", game_features: str = ""):
    crates = {
        "sunrise-game": f'[dependencies]\nsunrise-client = {{ path = "../sunrise-client", default-features = false }}\n'
                        f'[features]\ndefault = ["audio", "friends"]\naudio = {game_audio}\nfriends = ["sunrise-client/friends"]\n'
                        f'{game_features}',
        "sunrise-client": '[dependencies]\nsunrise-friends = { path = "../sunrise-friends", optional = true }\n'
                          f'sunrise-telemetry = {{ path = "../sunrise-telemetry" }}\n{client_deps}'
                          f'[features]\naudio = []\nfriends = ["dep:sunrise-friends"]\n{client_features}',
        "sunrise-friends": "",
        "sunrise-telemetry": '[dependencies]\nsunrise-addresses = { path = "../sunrise-addresses" }\n',
        "sunrise-addresses": "",
        "sunrise-reconnect": "",
        "sunrise-official": '[dependencies]\nsunrise-game = { path = "../sunrise-game" }\n'
                            '[features]\ndefault = ["sunrise-extras"]\nsunrise-extras = ["sunrise-game/friends"]\n',
    }
    for name, rest in crates.items():
        (root / "crates" / name / "src").mkdir(parents=True)
        (root / "crates" / name / "Cargo.toml").write_text(LIB.format(name=name, rest=rest))
        (root / "crates" / name / "src/lib.rs").write_text("")
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n')
    (root / "tools").mkdir()
    layers = {"sunrise-game": "platform", "sunrise-client": "engine", "sunrise-friends": "sunrise",
              "sunrise-official": "sunrise", "sunrise-telemetry": "sunrise", "sunrise-addresses": "sunrise",
              "sunrise-reconnect": "sunrise"}
    allowed = ["sunrise-client -> sunrise-telemetry", "sunrise-client -> sunrise-friends"]
    (root / "tools/layers.json").write_text(json.dumps({"crates": layers, "allowed": allowed}))
    cargo = shlex.split(os.environ.get("CARGO_COMMAND", "cargo"))
    subprocess.run([*cargo, "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "add", "-A"], cwd=root, check=True)


class VanillaFeatures(unittest.TestCase):
    def test_audio_turning_on_friends_or_a_new_sunrise_crate_fails_and_the_split_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            good = Path(tmp) / "good"
            workspace(good, '["sunrise-client/audio"]', "")
            self.assertEqual(check.problems(good), [])
            root_only = Path(tmp) / "root-only"
            workspace(root_only, '["sunrise-client/audio", "extension-points"]', "", game_features="extension-points = []\n")
            self.assertEqual(check.problems(root_only), ['sunrise-game has feature "extension-points"'])
            self.assertEqual(check.feature_problems("", {"sunrise-game"}), [
                "sunrise-client is missing from the plan", "sunrise-game is missing from the plan"])
            bad = Path(tmp) / "bad"
            workspace(bad, '["sunrise-client/audio", "sunrise-client/friends"]', 'sunrise-extras = ["friends"]\n',
                      'sunrise-reconnect = { path = "../sunrise-reconnect" }\n')
            self.assertEqual(check.problems(bad), [
                'sunrise-client has feature "friends"',
                "sunrise-friends is in the graph",
                "sunrise-reconnect is in the graph",
                "crates/sunrise-client/Cargo.toml declares the sunrise-extras feature",
            ])


if __name__ == "__main__":
    unittest.main()
