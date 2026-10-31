#!/usr/bin/env python3
"""Tests for `ops/release/make_manifest.py` (REL-2 S2-T3).

No network, no registry: the manifest is written from files the test
writes itself. The manifest is the file an operator's `lakehousectl`
verifies everything through, so its shape is checked here as carefully as
the numbers in it.
"""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import make_manifest  # noqa: E402

DIGEST = "sha256:" + "a" * 64
IMAGES = {
    "lakehouse-api": f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}",
    "console": f"ghcr.io/rantai-dev/rantai-lakehouse-console:0.2.0@{DIGEST}",
}


class ManifestTest(unittest.TestCase):
    """The manifest the script writes."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.compose = self.root / "docker-compose.yml"
        self.compose.write_text("services:\n  lakehouse-api:\n    image: x\n", encoding="utf-8")
        self.upgrade_path = self.root / "upgrade-path.json"
        self.upgrade_path.write_text('{"releases": {}}\n', encoding="utf-8")

    def tearDown(self) -> None:
        self._directory.cleanup()

    def build(self, **overrides) -> dict[str, object]:
        arguments = {
            "version": "0.2.0",
            "git_sha": "0123456789abcdef0123456789abcdef01234567",
            "images": IMAGES,
            "compose_path": self.compose,
            "created_at": "2026-11-01T12:00:00Z",
            "upgrade_path": self.upgrade_path,
        }
        arguments.update(overrides)
        return make_manifest.build_manifest(**arguments)  # type: ignore[arg-type]

    def test_the_manifest_holds_every_field_the_release_needs(self) -> None:
        manifest = self.build()
        self.assertEqual(
            list(manifest),
            [
                "schemaVersion",
                "version",
                "gitSha",
                "createdAt",
                "minFrom",
                "stops",
                "images",
                "files",
            ],
        )
        self.assertEqual(manifest["schemaVersion"], 1)
        self.assertEqual(manifest["version"], "0.2.0")
        self.assertEqual(manifest["createdAt"], "2026-11-01T12:00:00Z")
        self.assertEqual(manifest["stops"], [])

    def test_the_manifest_names_every_published_image(self) -> None:
        manifest = self.build()
        self.assertEqual(manifest["images"], IMAGES)

    def test_the_manifest_holds_the_compose_file_checksum(self) -> None:
        manifest = self.build()
        expected = hashlib.sha256(self.compose.read_bytes()).hexdigest()
        self.assertEqual(manifest["files"], {"docker-compose.yml": expected})

    def test_the_default_min_from_is_two_minor_versions_back(self) -> None:
        self.assertEqual(self.build()["minFrom"], "0.0.0")
        self.assertEqual(self.build(version="0.3.0")["minFrom"], "0.1.0")
        self.assertEqual(self.build(version="0.4.1")["minFrom"], "0.2.0")

    def test_an_upgrade_path_entry_overrides_the_min_from_and_names_a_stop(self) -> None:
        self.upgrade_path.write_text(
            json.dumps({"releases": {"0.2.0": {"minFrom": "0.1.0", "stops": ["0.1.5"]}}}),
            encoding="utf-8",
        )
        manifest = self.build()
        self.assertEqual(manifest["minFrom"], "0.1.0")
        self.assertEqual(manifest["stops"], ["0.1.5"])

    def test_an_upgrade_path_entry_for_another_release_changes_nothing(self) -> None:
        self.upgrade_path.write_text(
            json.dumps({"releases": {"0.9.0": {"stops": ["0.8.0"]}}}),
            encoding="utf-8",
        )
        manifest = self.build()
        self.assertEqual(manifest["minFrom"], "0.0.0")
        self.assertEqual(manifest["stops"], [])

    def test_an_image_without_a_digest_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(images={"lakehouse-api": "ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0"})

    def test_a_version_that_is_not_a_version_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(version="0.2")

    def test_a_git_sha_that_is_not_a_sha_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(git_sha="not-a-sha")

    def test_a_manifest_without_an_image_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(images={})

    def test_the_written_manifest_is_json_with_a_trailing_newline(self) -> None:
        manifest = self.build()
        out = self.root / "release.json"
        make_manifest.write_manifest(manifest, out)
        text = out.read_text(encoding="utf-8")
        self.assertTrue(text.endswith("\n"))
        self.assertEqual(json.loads(text)["version"], "0.2.0")


class CheckedInUpgradePathTest(unittest.TestCase):
    """The upgrade path the repository ships."""

    def test_the_checked_in_upgrade_path_is_readable(self) -> None:
        releases = make_manifest.read_upgrade_path(make_manifest.UPGRADE_PATH)
        self.assertIsInstance(releases, dict)


if __name__ == "__main__":
    unittest.main()
