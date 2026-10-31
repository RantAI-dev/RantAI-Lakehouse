#!/usr/bin/env python3
"""Unit tests for `ops/lint/check_image_digests.py` (REL-2 S1-T3).

No network, no containers: the lint is a static check on the checked-in
compose file, and these tests drive its decision functions directly. Run
by CI's "Repo lints" job (`python3 -m unittest discover -s ops/lint`).
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_image_digests as lint  # noqa: E402


class ImageLinesTest(unittest.TestCase):
    """Which lines the lint reads, and whose service they belong to."""

    def test_reads_each_image_with_its_service_name(self) -> None:
        text = """
services:
  postgres:
    image: postgres:16.15@sha256:1beaa3084eee2676f20a9cd9928f0bf2c5bf674f8e49f81017912183d377c61c
    restart: unless-stopped
  console:
    image: alpine:3.20@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc
"""
        found = lint.image_lines(text)
        self.assertEqual(len(found), 2)
        self.assertEqual(found[0][1], "postgres")
        self.assertEqual(found[1][1], "console")

    def test_ignores_an_image_inside_a_comment(self) -> None:
        text = """
services:
  postgres:
    # image: postgres:16
    image: postgres:16.15@sha256:1beaa3084eee2676f20a9cd9928f0bf2c5bf674f8e49f81017912183d377c61c
"""
        found = lint.image_lines(text)
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0][1], "postgres")


class ExemptionTest(unittest.TestCase):
    """Only the locally BUILT images are exempt, and only by their shape."""

    def test_a_locally_built_image_is_exempt(self) -> None:
        self.assertTrue(lint.is_local_build("${LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}"))
        self.assertTrue(lint.is_local_build("${CONSOLE_IMAGE:-rantai-lakehouse-console:local}"))

    def test_a_variable_that_is_not_a_local_build_is_not_exempt(self) -> None:
        # A hidden reference cannot be shown to be pinned, so it fails.
        self.assertFalse(lint.is_local_build("${SOME_IMAGE:-postgres:16}"))
        self.assertFalse(lint.is_local_build("${LAKEHOUSE_API_IMAGE:-postgres:16}"))

    def test_a_third_party_image_is_not_exempt(self) -> None:
        self.assertFalse(lint.is_local_build("postgres:16.15@sha256:" + "a" * 64))


class DigestTest(unittest.TestCase):
    """What counts as a digest."""

    def test_a_full_digest_passes(self) -> None:
        self.assertTrue(lint.check_image("postgres:16.15@sha256:" + "a" * 64))

    def test_no_digest_fails(self) -> None:
        self.assertFalse(lint.check_image("postgres:16.15"))

    def test_a_short_digest_fails(self) -> None:
        self.assertFalse(lint.check_image("postgres:16.15@sha256:" + "a" * 63))

    def test_a_digest_that_is_not_at_the_end_fails(self) -> None:
        self.assertFalse(lint.check_image("postgres:16.15@sha256:" + "a" * 64 + "@x"))


class ComposeFileTest(unittest.TestCase):
    """The checked-in compose file itself."""

    def test_every_third_party_image_in_the_compose_file_is_pinned(self) -> None:
        text = lint.COMPOSE_PATH.read_text(encoding="utf-8")
        unpinned = [
            (number, service, value)
            for number, service, value in lint.image_lines(text)
            if not lint.is_local_build(value) and not lint.check_image(value)
        ]
        self.assertEqual(unpinned, [], f"unpinned images in docker-compose.yml: {unpinned}")


if __name__ == "__main__":
    unittest.main()
