#!/usr/bin/env python3
"""Tests for `ops/release/make_manifest.py` (REL-2 S2-T3).

No network, no registry: the manifest is written from files the test
writes itself. The manifest is the file an operator's `lakehousectl`
verifies everything through, so its shape is checked here as carefully as
the numbers in it.

Since REL-2 S2 review round 6 BLOCKER 1, the manifest names EVERY image
the release runs — the third-party ones included. A manifest that named
only our four would leave an air-gapped host without Postgres.
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
THIRD_PARTY_DIGEST = "sha256:" + "b" * 64
IMAGES = {
    "lakehouse-api": f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}",
    "console": f"ghcr.io/rantai-dev/rantai-lakehouse-console:0.2.0@{DIGEST}",
}

# A rendered Compose file, the way `render_compose.py` writes one: our
# two services at their published digests, and a third-party service at
# the digest the S1 lint pinned.
COMPOSE = f"""\
services:
  lakehouse-api:
    image: ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}
  console:
    image: ghcr.io/rantai-dev/rantai-lakehouse-console:0.2.0@{DIGEST}
  postgres:
    image: postgres:16.15@{THIRD_PARTY_DIGEST}
"""


class ManifestTest(unittest.TestCase):
    """The manifest the script writes."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.compose = self.root / "docker-compose.yml"
        self.compose.write_text(COMPOSE, encoding="utf-8")
        self.upgrade_path = self.root / "upgrade-path.json"
        self.upgrade_path.write_text('{"releases": {}}\n', encoding="utf-8")

    def tearDown(self) -> None:
        self._directory.cleanup()

    def build(self, **overrides) -> dict[str, object]:
        arguments = {
            "version": "0.2.0",
            "git_sha": "0123456789abcdef0123456789abcdef01234567",
            "published_images": IMAGES,
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
        self.assertEqual(
            {service: manifest["images"][service] for service in IMAGES},
            IMAGES,
        )

    def test_the_manifest_names_every_service_of_the_rendered_compose_file(self) -> None:
        """REL-2 S2 review round 6 BLOCKER 1: EVERY image the release runs.

        The third-party images are in the bundle too, so the manifest has
        to name them: an image the manifest does not name is an image
        the air-gapped host cannot run.
        """
        manifest = self.build()
        self.assertEqual(
            manifest["images"],
            {
                "console": IMAGES["console"],
                "lakehouse-api": IMAGES["lakehouse-api"],
                "postgres": f"postgres:16.15@{THIRD_PARTY_DIGEST}",
            },
        )

    def test_a_third_party_image_without_a_digest_is_refused(self) -> None:
        """A tag is a name, and a name moves: the manifest must not name one."""
        self.compose.write_text(
            "services:\n  postgres:\n    image: postgres:16.15\n", encoding="utf-8"
        )
        with self.assertRaises(make_manifest.ManifestError):
            self.build(published_images={})

    def test_a_published_image_without_a_digest_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(published_images={"lakehouse-api": "ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0"})

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

    def test_a_version_that_is_not_a_version_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(version="0.2")

    def test_a_git_sha_that_is_not_a_sha_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError):
            self.build(git_sha="not-a-sha")

    def test_a_manifest_without_an_image_is_refused(self) -> None:
        self.compose.write_text("services:\n  lakehouse-api:\n    build: .\n", encoding="utf-8")
        with self.assertRaises(make_manifest.ManifestError):
            self.build(published_images={})

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
