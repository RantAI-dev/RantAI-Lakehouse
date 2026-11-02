#!/usr/bin/env python3
"""Tests for `ops/release/compose_images.py` (REL-2 S2 review round 6).

The manifest and the release pipeline must agree on which image a
service runs and on which of those images are ours. Both ask this
script, so the rule is tested here: a rule that drifts is a manifest
that names one image and a bundle that holds another.

No network, no registry: the test writes the Compose file itself.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compose_images  # noqa: E402

RELEASE_DIR = Path(__file__).resolve().parent

DIGEST = "sha256:" + "a" * 64
THIRD_PARTY_DIGEST = "sha256:" + "b" * 64

COMPOSE = f"""\
services:
  lakehouse-api:
    image: ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}
  dagster-webserver:
    image: ghcr.io/rantai-dev/rantai-lakehouse-dagster:0.2.0@{DIGEST}
  postgres:
    image: postgres:16.15@{THIRD_PARTY_DIGEST}
  openfga:
    image: openfga/openfga:v1.8@{THIRD_PARTY_DIGEST}
"""

# The same third-party image under two services: seven of the release's
# services run Postgres.
COMPOSE_WITH_A_SHARED_IMAGE = f"""\
services:
  postgres:
    image: postgres:16.15@{THIRD_PARTY_DIGEST}
  postgres-cert-init:
    image: postgres:16.15@{THIRD_PARTY_DIGEST}
"""

PUBLISHED = {
    "lakehouse-api": f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}",
    "dagster-webserver": f"ghcr.io/rantai-dev/rantai-lakehouse-dagster:0.2.0@{DIGEST}",
}


class ComposeImagesTest(unittest.TestCase):
    """The images of a Compose file."""

    def test_every_service_that_names_an_image_is_in_the_map(self) -> None:
        self.assertEqual(
            compose_images.compose_images(COMPOSE),
            {
                "lakehouse-api": f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}",
                "dagster-webserver": f"ghcr.io/rantai-dev/rantai-lakehouse-dagster:0.2.0@{DIGEST}",
                "postgres": f"postgres:16.15@{THIRD_PARTY_DIGEST}",
                "openfga": f"openfga/openfga:v1.8@{THIRD_PARTY_DIGEST}",
            },
        )

    def test_a_service_that_names_no_image_is_not_in_the_map(self) -> None:
        images = compose_images.compose_images(
            "services:\n  lakehouse-api:\n    build: .\n"
        )
        self.assertEqual(images, {})

    def test_the_third_party_images_are_the_ones_we_did_not_publish(self) -> None:
        images = compose_images.compose_images(COMPOSE)
        self.assertEqual(
            compose_images.third_party(images, PUBLISHED),
            {
                "postgres": f"postgres:16.15@{THIRD_PARTY_DIGEST}",
                "openfga": f"openfga/openfga:v1.8@{THIRD_PARTY_DIGEST}",
            },
        )


class LoadPublishedTest(unittest.TestCase):
    """The published images, from an inline document or a file."""

    def test_an_inline_document_is_read(self) -> None:
        self.assertEqual(
            compose_images.load_published(json.dumps(PUBLISHED)), PUBLISHED
        )

    def test_a_file_is_read(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "images.json"
            path.write_text(json.dumps(PUBLISHED), encoding="utf-8")
            self.assertEqual(compose_images.load_published(str(path)), PUBLISHED)

    def test_a_document_that_is_not_json_is_refused(self) -> None:
        with self.assertRaises(compose_images.ComposeImagesError):
            compose_images.load_published("{not json")

    def test_a_document_that_is_not_an_object_is_refused(self) -> None:
        with self.assertRaises(compose_images.ComposeImagesError):
            compose_images.load_published('["a"]')

    def test_a_file_that_does_not_exist_is_refused(self) -> None:
        with self.assertRaises(OSError):
            compose_images.load_published("/nonexistent/images.json")


class CommandTest(unittest.TestCase):
    """The command the release pipeline runs."""

    def run_command(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(RELEASE_DIR / "compose_images.py"), *arguments],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_the_third_party_references_are_printed_one_per_line(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compose = root / "docker-compose.yml"
            compose.write_text(COMPOSE, encoding="utf-8")
            published = root / "images.json"
            published.write_text(json.dumps(PUBLISHED), encoding="utf-8")
            result = self.run_command(
                "--compose", str(compose), "--published-images", str(published)
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.split(),
                [
                    f"postgres:16.15@{THIRD_PARTY_DIGEST}",
                    f"openfga/openfga:v1.8@{THIRD_PARTY_DIGEST}",
                ],
            )

    def test_without_the_published_images_every_image_is_printed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            compose = Path(directory) / "docker-compose.yml"
            compose.write_text(COMPOSE, encoding="utf-8")
            result = self.run_command("--compose", str(compose))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(result.stdout.split()), 4)

    def test_an_image_two_services_run_is_printed_once(self) -> None:
        """The pipeline pulls an image once, and describes it once."""
        with tempfile.TemporaryDirectory() as directory:
            compose = Path(directory) / "docker-compose.yml"
            compose.write_text(COMPOSE_WITH_A_SHARED_IMAGE, encoding="utf-8")
            result = self.run_command("--compose", str(compose))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.split(), [f"postgres:16.15@{THIRD_PARTY_DIGEST}"]
            )

    def test_a_compose_file_that_does_not_exist_is_refused(self) -> None:
        result = self.run_command("--compose", "/nonexistent/docker-compose.yml")
        self.assertEqual(result.returncode, 1)
        self.assertIn("compose_images failed", result.stderr)


if __name__ == "__main__":
    unittest.main()
