"""launcher-bridge.yml: sign_release.sh signs a launcher release for both tags, and verify_launcher_release.sh accepts only what was signed."""
import base64
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests.os_support import BASH, needs_command, needs_ed25519_openssl, needs_runner_shell

ROOT = Path(__file__).resolve().parents[1]
SIGN = ROOT / "tools/ci/sign_release.sh"
VERIFY = ROOT / "tools/ci/verify_launcher_release.sh"
# The PATH sign_release.sh runs with, and so the one whose OpenSSL must generate Ed25519 keys.
SCRIPT_PATH = "/usr/bin:/bin"
OPENSSL = shutil.which("openssl", path=SCRIPT_PATH)
ASSETS = ["sunrise-launcher-linux-x86_64.tar.gz", "sunrise-launcher-setup.exe"]


@needs_runner_shell
@needs_command("sha256sum")
@needs_ed25519_openssl(SCRIPT_PATH)
class LauncherBridge(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.work)
        self.dist = self.work / "dist"
        self.dist.mkdir()
        for name in ASSETS:
            (self.dist / name).write_bytes(name.encode())

    def sign(self, tag="launcher@1.1.0", launcher_for="v1.1.0"):
        files = [str(self.dist / name) for name in ASSETS]
        env = {"PATH": SCRIPT_PATH, "EPHEMERAL": "yes", "LAUNCHER_FOR": launcher_for}
        result = subprocess.run([BASH, SIGN, tag, *files], env=env, capture_output=True, text=True)
        if result.returncode == 0:
            self.keys = self.work / "keys.txt"
            shutil.move(self.dist / "ephemeral.pub", self.keys)
        return result.returncode == 0

    def accepts(self, *args) -> bool:
        return subprocess.run([BASH, VERIFY, self.dist, *map(str, args)], capture_output=True).returncode == 0

    def test_both_signatures_check_with_the_key_that_made_them(self):
        self.assertTrue(self.sign())
        self.assertTrue(self.accepts("launcher@1.1.0", self.keys, self.dist / "SHA256SUMS.sig"))
        self.assertTrue(self.accepts("v1.1.0", self.keys, self.dist / "SHA256SUMS.launcher.sig"))

    def test_a_signature_never_passes_for_the_other_tag_or_another_version(self):
        self.assertTrue(self.sign())
        self.assertFalse(self.accepts("v1.1.0", self.keys, self.dist / "SHA256SUMS.sig"))
        self.assertFalse(self.accepts("launcher@1.1.0", self.keys, self.dist / "SHA256SUMS.launcher.sig"))
        self.assertFalse(self.accepts("v1.2.0", self.keys, self.dist / "SHA256SUMS.launcher.sig"))

    def test_an_unlisted_or_changed_file_is_refused(self):
        self.assertTrue(self.sign())
        (self.dist / "sunrise-launcher-extra.zip").write_bytes(b"unsigned")
        self.assertFalse(self.accepts())
        (self.dist / "sunrise-launcher-extra.zip").unlink()
        self.assertTrue(self.accepts())
        (self.dist / ASSETS[0]).write_bytes(b"changed")
        self.assertFalse(self.accepts())

    def test_the_launcher_tag_must_be_the_bridged_version(self):
        self.assertFalse(self.sign(launcher_for="v1.2.0"))
        self.assertFalse(self.sign(tag="nightly", launcher_for="v1.1.0"))
        self.assertFalse(self.sign(launcher_for="v1.1.0-rc1"))

    def test_a_release_key_from_the_environment_signs_both(self):
        der = subprocess.run([OPENSSL, "genpkey", "-algorithm", "ed25519", "-outform", "DER"], check=True, capture_output=True).stdout
        public = subprocess.run([OPENSSL, "pkey", "-inform", "DER", "-pubout", "-outform", "DER"], input=der, check=True, capture_output=True).stdout[-32:]
        keys = self.work / "release-keys.txt"
        keys.write_text(base64.b64encode(public).decode() + "\n")
        env = {"PATH": SCRIPT_PATH, "LAUNCHER_FOR": "v1.1.0", "RELEASE_SIGNING_KEY": base64.b64encode(der).decode()}
        files = [str(self.dist / name) for name in ASSETS]
        subprocess.run([BASH, SIGN, "launcher@1.1.0", *files], env=env, check=True, capture_output=True)
        self.assertFalse((self.dist / "ephemeral.pub").exists())
        self.assertTrue(self.accepts("launcher@1.1.0", keys, self.dist / "SHA256SUMS.sig"))
        self.assertTrue(self.accepts("v1.1.0", keys, self.dist / "SHA256SUMS.launcher.sig"))

    def test_windows_openssl_only_gets_a_key_file_for_ephemeral_signing(self):
        # Simulate Git Bash with a native OpenSSL that cannot reopen its /dev/fd paths.
        bash_env = self.work / "windows-openssl.sh"
        bash_env.write_text(r"""uname() { echo MINGW64_NT-10.0; }
windows_openssl() {
  if [[ "$1" == pkeyutl ]]; then
    local i next key_path
    for ((i=1; i<$#; i++)); do
      [[ "${!i}" == -inkey ]] || continue
      next=$((i + 1))
      key_path=${!next}
      printf '%s\n' "$key_path" > "$KEY_PATH_LOG"
      [[ "$key_path" != /dev/fd/3 ]] || return 42
      [[ -f "$key_path" ]] || return 43
      break
    done
  fi
  command openssl "$@"
}
""")
        key_path_log = self.work / "inkey.txt"
        env = {"PATH": SCRIPT_PATH, "BASH_ENV": str(bash_env), "OPENSSL": "windows_openssl",
               "KEY_PATH_LOG": str(key_path_log)}
        tag = "launcher@1.1.0"
        cmd = [BASH, SIGN, tag, *(self.dist / name for name in ASSETS)]
        signature = self.dist / "SHA256SUMS.sig"
        subprocess.run(cmd, env=dict(env, EPHEMERAL="yes"), check=True, capture_output=True)
        key_path = Path(key_path_log.read_text().strip())
        self.assertEqual(key_path.name, "ephemeral.der")
        self.assertFalse(key_path.exists())
        keys = self.work / "ephemeral.pub"
        (self.dist / "ephemeral.pub").rename(keys)
        self.assertTrue(self.accepts(tag, keys, signature))

        der = subprocess.run([OPENSSL, "genpkey", "-algorithm", "ed25519", "-outform", "DER"], check=True, capture_output=True).stdout
        refused = subprocess.run(cmd, env=dict(env, RELEASE_SIGNING_KEY=base64.b64encode(der).decode()), capture_output=True)
        self.assertEqual(refused.returncode, 42, refused.stderr)
        self.assertEqual(key_path_log.read_text().strip(), "/dev/fd/3")
        self.assertFalse(signature.exists())

    def test_without_a_listed_key_nothing_passes(self):
        self.assertTrue(self.sign())
        empty = self.work / "empty.txt"
        empty.write_text("# no key\n")
        self.assertFalse(self.accepts("v1.1.0", empty, self.dist / "SHA256SUMS.launcher.sig"))


if __name__ == "__main__":
    unittest.main()
