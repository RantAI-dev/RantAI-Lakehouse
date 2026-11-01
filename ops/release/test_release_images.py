#!/usr/bin/env python3
"""Tests for `ops/release/release-images.json` (REL-2 S2-T2).

The mapping is the ONE place that says which published image each
service runs. The release pipeline reads it to know what to push and
which service each pushed image belongs to; the CI check that renders
the template reads it to build a placeholder image list. If it drifts
from the template, a release would publish an image nobody runs, or run
an image nobody published — so these tests hold it to the template.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))

import render_compose  # noqa: E402

MAPPING_PATH = Path(__file__).resolve().parent / "release-images.json"


def mapping() -> dict[str, list[str]]:
    """The mapping, as image name -> the services that run it."""
    data = json.loads(MAPPING_PATH.read_text(encoding="utf-8"))
    return {name: list(services) for name, services in data.items()}


def mapped_services() -> set[str]:
    """Every service the mapping names."""
    return {service for services in mapping().values() for service in services}


def built_services() -> set[str]:
    """The services of the template that this repository builds.

    The gate builds (the G6 stubs, the G7 runner) are built too, and they
    are NOT released: they are the acceptance harnesses' own images.
    """
    text = render_compose.COMPOSE_PATH.read_text(encoding="utf-8")
    _header, services, _trailer = render_compose.parse_compose(text)
    return {
        service.name
        for service in services
        if service.has_build and not render_compose.is_gate_service(service)
    }


class ReleaseImagesTest(unittest.TestCase):
    """The mapping against the template."""

    def test_every_service_the_template_builds_is_in_the_mapping(self) -> None:
        self.assertEqual(built_services() - mapped_services(), set())

    def test_the_mapping_names_no_service_the_template_does_not_build(self) -> None:
        self.assertEqual(mapped_services() - built_services(), set())

    def test_no_service_runs_two_published_images(self) -> None:
        listed = [service for services in mapping().values() for service in services]
        duplicates = {service for service in listed if listed.count(service) > 1}
        self.assertEqual(duplicates, set())

    def test_the_published_images_are_named_after_the_repository(self) -> None:
        for name in mapping():
            self.assertTrue(name.startswith("rantai-lakehouse-"), name)

    def test_the_mapping_holds_the_four_images_a_release_publishes(self) -> None:
        self.assertEqual(len(mapping()), 4)


if __name__ == "__main__":
    unittest.main()
