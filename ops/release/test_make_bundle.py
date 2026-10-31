#!/usr/bin/env python3
"""Tests for `ops/release/make_bundle.sh` (REL-2 S2-T3).

The bundle is what an air-gapped host installs, so the checksums it
carries are the only thing that says "these are the bytes the release
published". These tests build a bundle and check that every checksum in
it matches the file it names.

`docker save` is stood in for: the test puts a script named `docker` on
the PATH that writes a file instead of saving an image. The bundle's own
logic — what it collects, what it hashes, what it names — is what is
under test here; that a real image can be saved and loaded is proven by
the release pipeline's own dry run, which saves the four published
images into a real bundle.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
BUNDLE_SCRIPT = Path(__file__).resolve().parent / "make_bundle.sh"

DIGEST = "sha256:" + "a" * 64
IMAGE = f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}"

FAKE_DOCKER = """#!/usr/bin/env bash
# A stand-in for `docker save`: it writes a file that stands for the
# image, so the bundle's logic can be tested without a daemon.
set -euo pipefail
if [ "${1:-}" != "save" ]; then
    echo "fake docker: only `save` is implemented" >&2
    exit 1
fi
shift
output=""
reference=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o|--output) output="$2"; shift 2 ;;
        *) reference="$1"; shift ;;
    esac
done
printf 'a stand-in for the image tar of %s\\n' "$reference" > "$output"
"""


class BundleTest(unittest.TestCase):
    """What the bundle script writes."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.bin / "docker").write_text(FAKE_DOCKER, encoding="utf-8")
        (self.bin / "docker").chmod(0o755)

        self.compose = self.root / "docker-compose.yml"
        self.compose.write_text("services:\n  lakehouse-api:\n    image: x\n", encoding="utf-8")

        self.sbom_dir = self.root / "sboms"
        self.sbom_dir.mkdir()
        (self.sbom_dir / "rantai-lakehouse-api.cdx.json").write_text(
            json.dumps({"bomFormat": "CycloneDX"}), encoding="utf-8"
        )

        self.signature = self.root / "release.json.sig"
        self.signature.write_text("a signature\n", encoding="utf-8")

        self.manifest = self.root / "release.json"
        self.manifest.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "version": "0.2.0",
                    "gitSha": "0123456789abcdef0123456789abcdef01234567",
                    "createdAt": "2026-11-01T12:00:00Z",
                    "minFrom": "0.0.0",
                    "stops": [],
                    "images": {"lakehouse-api": IMAGE},
                    "files": {"docker-compose.yml": ""},
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        self.write_manifest_with_the_compose_checksum()

        self.out = self.root / "out"

    def tearDown(self) -> None:
        self._directory.cleanup()

    def write_manifest_with_the_compose_checksum(self) -> None:
        """The manifest, with the Compose file's real checksum in it."""
        import hashlib

        data = json.loads(self.manifest.read_text(encoding="utf-8"))
        data["files"]["docker-compose.yml"] = hashlib.sha256(
            self.compose.read_bytes()
        ).hexdigest()
        self.manifest.write_text(json.dumps(data, indent=2), encoding="utf-8")

    def run_bundle(self) -> subprocess.CompletedProcess[str]:
        environment = dict(os.environ)
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        return subprocess.run(
            [
                "bash",
                str(BUNDLE_SCRIPT),
                "--manifest",
                str(self.manifest),
                "--compose",
                str(self.compose),
                "--signature",
                str(self.signature),
                "--sbom-dir",
                str(self.sbom_dir),
                "--out",
                str(self.out),
            ],
            capture_output=True,
            text=True,
            env=environment,
            check=False,
        )

    def extract(self) -> Path:
        bundle = self.out / "rantai-lakehouse-0.2.0-linux-x86_64.tar.zst"
        target = self.root / "extracted"
        target.mkdir()
        subprocess.run(
            ["tar", "--zstd", "-xf", str(bundle), "-C", str(target)],
            check=True,
            capture_output=True,
        )
        return target

    def test_the_bundle_is_written_and_its_size_is_printed(self) -> None:
        result = self.run_bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.out / "rantai-lakehouse-0.2.0-linux-x86_64.tar.zst").exists())
        self.assertIn("bytes", result.stdout)

    def test_the_bundle_holds_the_release_files(self) -> None:
        self.assertEqual(self.run_bundle().returncode, 0)
        extracted = self.extract()
        for name in (
            "docker-compose.yml",
            "release.json",
            "release.json.sig",
            "SHA256SUMS",
        ):
            self.assertTrue((extracted / name).exists(), name)
        self.assertTrue((extracted / "images" / "rantai-lakehouse-api.tar").exists())
        self.assertTrue((extracted / "sbom" / "rantai-lakehouse-api.cdx.json").exists())

    def test_every_checksum_in_the_bundle_matches_the_file_it_names(self) -> None:
        self.assertEqual(self.run_bundle().returncode, 0)
        extracted = self.extract()
        result = subprocess.run(
            ["sha256sum", "-c", "SHA256SUMS"],
            cwd=extracted,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("FAILED", result.stdout)

    def test_a_compose_file_that_is_not_the_one_the_manifest_names_is_refused(self) -> None:
        self.compose.write_text("services:\n  changed: {}\n", encoding="utf-8")
        result = self.run_bundle()
        self.assertEqual(result.returncode, 1)
        self.assertIn("checksum", result.stderr)
        self.assertFalse((self.out / "rantai-lakehouse-0.2.0-linux-x86_64.tar.zst").exists())

    def test_an_image_without_an_sbom_is_refused(self) -> None:
        (self.sbom_dir / "rantai-lakehouse-api.cdx.json").unlink()
        result = self.run_bundle()
        self.assertEqual(result.returncode, 1)
        self.assertIn("SBOM", result.stderr)

    def test_a_missing_signature_is_refused(self) -> None:
        self.signature.unlink()
        result = self.run_bundle()
        self.assertEqual(result.returncode, 1)
        self.assertIn("signature", result.stderr)


@unittest.skipUnless(shutil.which("zstd"), "zstd is needed to build a bundle")
class BundleToolTest(unittest.TestCase):
    """The tools the bundle needs."""

    def test_the_tools_the_bundle_needs_are_named_in_its_usage(self) -> None:
        text = BUNDLE_SCRIPT.read_text(encoding="utf-8")
        for tool in ("docker", "jq", "zstd", "tar", "sha256sum"):
            self.assertIn(tool, text)


if __name__ == "__main__":
    unittest.main()
