"""The image Railway builds for the telemetry server copies only the crates it names.

The service is its own workspace with its own Cargo.lock. When one of its crates gains a path dependency on
another crate of the repository, the image must copy that crate too, and Railway must rebuild when it changes:
otherwise the next deploy fails (`no matching package named ...`) while the old deployment keeps running, and nobody
sees it (2026-11-09: it failed after sunrise-addresses and sunrise-release-sign joined it).
"""

import json
import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SERVICES = ["sunrise-telemetry-server"]


def all_crates() -> set[str]:
    return {path.parent.name for path in (ROOT / "crates").glob("*/Cargo.toml")}


def path_packages(lock: Path) -> set[str]:
    """The packages a lockfile builds from a path: the ones without a `source`."""
    return {package["name"] for package in tomllib.loads(lock.read_text())["package"] if "source" not in package}


def copies_every_crate(dockerfile: Path) -> bool:
    return re.search(r"^COPY crates crates\s*$", dockerfile.read_text(), re.MULTILINE) is not None


def copied_crates(dockerfile: Path) -> set[str]:
    if copies_every_crate(dockerfile):
        return all_crates()
    return set(re.findall(r"^COPY crates/([a-z0-9-]+) crates/\1\s*$", dockerfile.read_text(), re.MULTILINE))


def watched_crates(railway: Path) -> set[str]:
    patterns = json.loads(railway.read_text())["build"]["watchPatterns"]
    if "/crates/**" in patterns:
        return all_crates()
    return {match.group(1) for pattern in patterns if (match := re.fullmatch(r"/crates/([a-z0-9-]+)/\*\*", pattern))}


class ServiceImageTests(unittest.TestCase):
    def test_each_image_copies_every_path_crate_of_its_lockfile(self):
        for service in SERVICES:
            crate = ROOT / "crates" / service
            needed = path_packages(crate / "Cargo.lock")
            with self.subTest(service=service):
                self.assertIn(service, needed)
                for name in needed:
                    self.assertTrue((ROOT / "crates" / name / "Cargo.toml").is_file(), f"{name} is not crates/{name}")
                missing = sorted(needed - copied_crates(crate / "Dockerfile"))
                self.assertEqual(missing, [], f"{service}/Dockerfile must COPY these crates")

    def test_railway_rebuilds_when_any_of_those_crates_changes(self):
        for service in SERVICES:
            crate = ROOT / "crates" / service
            with self.subTest(service=service):
                missing = sorted(path_packages(crate / "Cargo.lock") - watched_crates(crate / "railway.json"))
                self.assertEqual(missing, [], f"{service}/railway.json watchPatterns must list these crates")

    def test_the_checks_read_real_dockerfile_lines(self):
        # A COPY of a crate is recognised, a COPY of anything else is not, and a lockfile's registry packages
        # are not path packages.
        sample = ROOT / "crates" / "sunrise-telemetry-server" / "Dockerfile"
        self.assertIn("sunrise-telemetry-server", copied_crates(sample))
        self.assertNotIn("Cargo.toml", copied_crates(sample))
        self.assertNotIn("serde", path_packages(ROOT / "crates" / "sunrise-telemetry-server" / "Cargo.lock"))
        # A Dockerfile that names its crates one by one copies only those.
        self.assertFalse(copies_every_crate(sample))
        self.assertNotIn("sunrise-client", copied_crates(sample))


if __name__ == "__main__":
    unittest.main()
