#!/usr/bin/env python3
"""Tests for `ops/release/install.sh` (REL-2 S3-T4).

The script is the one command an operator types
(`curl -fsSL https://get.rantai.dev/lakehouse | bash`), so two things
matter and are checked here:

- it refuses a host it does not support, with a message that says why;
- it checks the downloaded binary against the release's own checksums
  BEFORE it runs it.

The script's downloads are stood in for: a fake `curl` serves files
from a directory, and the binary it serves is a script that records
the arguments it was given. The script's own logic — the refusal, the
checksum check, the hand-over to `lakehousectl install` — is what is
under test.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

RELEASE_DIR = Path(__file__).resolve().parent
INSTALL_SCRIPT = RELEASE_DIR / "install.sh"
BINARY = "lakehousectl-linux-x86_64"


class InstallScriptTest(unittest.TestCase):
    """The script an operator runs."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)

        # The release the fake `curl` serves: the binary, the
        # checksums, and the JSON that names the newest release.
        self.served = self.root / "served"
        self.served.mkdir()
        self.binary = self.served / BINARY
        self.binary.write_text(
            "#!/bin/sh\necho \"lakehousectl $@\" > \"$RECORD\"\n", encoding="utf-8"
        )
        self.checksums = self.served / "SHA256SUMS"
        self.checksums.write_text(
            f"{hashlib.sha256(self.binary.read_bytes()).hexdigest()}  {BINARY}\n",
            encoding="utf-8",
        )
        self.latest = self.served / "latest.json"
        self.latest.write_text(json.dumps({"tag_name": "v0.2.0"}), encoding="utf-8")

        # The tools the script calls, stood in for.
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.write_tool("curl", self.fake_curl())
        self.write_tool("uname", self.fake_uname("Linux", "x86_64"))
        self.record = self.root / "record"

    def tearDown(self) -> None:
        self._directory.cleanup()

    def write_tool(self, name: str, body: str) -> None:
        path = self.bin / name
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)

    def fake_curl(self) -> str:
        return """#!/usr/bin/env bash
# A stand-in for curl: it serves the files of the release the test
# prepared, and it records what it was asked for.
set -euo pipefail
url="${!#}"
output=""
previous=""
for argument in "$@"; do
    if [ "$previous" = "--output" ] || [ "$previous" = "-o" ]; then output="$argument"; fi
    previous="$argument"
done
echo "curl $url" >> "${CURL_LOG:-/dev/null}"
case "$url" in
    *latest*) cat "${SERVED}/latest.json" ;;
    *SHA256SUMS*) cat "${SERVED}/SHA256SUMS" ;;
    *) cat "${SERVED}/lakehousectl-linux-x86_64" ;;
esac > "${output:-/dev/stdout}"
"""

    def fake_uname(self, system: str, machine: str) -> str:
        return f"""#!/usr/bin/env bash
case "${{1:-}}" in
    -s) echo "{system}" ;;
    -m) echo "{machine}" ;;
    *) echo "{system}" ;;
esac
"""

    def run_script(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        environment = dict(os.environ)
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        environment["SERVED"] = str(self.served)
        environment["CURL_LOG"] = str(self.root / "curl.log")
        environment["RECORD"] = str(self.record)
        return subprocess.run(
            ["bash", str(INSTALL_SCRIPT), *arguments],
            capture_output=True,
            text=True,
            env=environment,
            check=False,
        )

    def test_the_script_runs_the_installer_with_its_arguments(self) -> None:
        result = self.run_script("--dir", "/opt/lakehouse")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.record.exists(), "the installer was not run")
        self.assertEqual(
            self.record.read_text(encoding="utf-8").strip(),
            "lakehousectl install --dir /opt/lakehouse",
        )

    def test_the_script_passes_the_version_on(self) -> None:
        result = self.run_script("--version", "0.2.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "--version 0.2.0",
            self.record.read_text(encoding="utf-8"),
        )

    def test_a_host_that_is_not_x86_64_is_refused(self) -> None:
        self.write_tool("uname", self.fake_uname("Linux", "aarch64"))
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("x86_64", result.stderr)
        self.assertFalse(self.record.exists(), "the installer was run anyway")

    def test_a_host_that_is_not_linux_is_refused(self) -> None:
        self.write_tool("uname", self.fake_uname("Darwin", "x86_64"))
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("Linux", result.stderr)
        self.assertFalse(self.record.exists(), "the installer was run anyway")

    def test_a_binary_that_does_not_match_the_checksum_is_refused(self) -> None:
        """The download is checked before it runs.

        A tampered download would be a tampered installer, and the
        installer is what verifies the release.
        """
        self.binary.write_text("#!/bin/sh\necho tampered\n", encoding="utf-8")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("checksum", result.stderr)
        self.assertFalse(self.record.exists(), "the installer was run anyway")


if __name__ == "__main__":
    unittest.main()
