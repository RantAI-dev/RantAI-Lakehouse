#!/usr/bin/env python3
"""Tests for `ops/release/render_compose.py` (REL-2 S2-T2).

No network, no containers: the renderer is a text transformation over a
Compose file, and these tests give it a small file of their own and read
the result. The last group runs it over the REAL template, because the
template is what the release actually renders.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))

import render_compose  # noqa: E402

DIGEST_A = "sha256:" + "a" * 64
DIGEST_B = "sha256:" + "b" * 64
DIGEST_C = "sha256:" + "c" * 64

TEMPLATE = """\
# The deployment template.
services:
  postgres:
    image: postgres:16.15@{digest_a}
    restart: unless-stopped

  lakehouse-api:
    # The API's own image, built from source.
    image: ${{LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}}
    build:
      context: ./rust
      dockerfile: Dockerfile
      # The build's own arguments.
      args:
        GIT_SHA: ${{GIT_SHA:-unknown}}
    ports:
      - "${{API_HOST_PORT:-8080}}:8080"

  oidc-mock:
    build:
      context: ./ops/oidc-mock
    environment:
      OIDC_MOCK_PORT: "8090"

  g1-test-runner:
    profiles: ["test"]
    image: rust:1.99.1-slim@{digest_b}

  g6-test-runner:
    profiles: ["g6", "g6-other"]
    build:
      context: ./ops/g6

  g4-source-init:
    profiles: ["dagster"]
    image: postgres:16.15@{digest_a}

  debezium-server:
    profiles: ["dagster"]
    image: ghcr.io/memiiso/debezium-server-iceberg:1.1.1.Final@{digest_c}
    depends_on:
      g4-source-init:
        condition: service_completed_successfully

volumes:
  lakehouse_api_data:
""".format(digest_a=DIGEST_A, digest_b=DIGEST_B, digest_c=DIGEST_C)

IMAGES = {
    "lakehouse-api": f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST_A}",
    "oidc-mock": f"ghcr.io/rantai-dev/rantai-lakehouse-oidc-mock:0.2.0@{DIGEST_B}",
}


def render(template: str = TEMPLATE, images: dict[str, str] | None = None) -> str:
    return render_compose.render(template, IMAGES if images is None else images)


def build_keys(rendered: str) -> list[str]:
    """The `build:` keys of a rendered file.

    A KEY, not the word: the generated header explains in a comment that
    no `build:` block is left, and the header is part of the file.
    """
    return [
        line
        for line in rendered.splitlines()
        if line.startswith("    build:") and line.strip() == "build:"
    ]


class RenderTest(unittest.TestCase):
    """What the renderer does to a service."""

    def test_a_locally_built_service_loses_its_build_block_and_gets_the_published_image(self) -> None:
        rendered = render()
        self.assertEqual(build_keys(rendered), [])
        self.assertIn(f"    image: ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST_A}", rendered)

    def test_a_service_that_named_no_image_gets_one_where_the_build_block_was(self) -> None:
        rendered = render()
        self.assertIn(f"  oidc-mock:\n    image: ghcr.io/rantai-dev/rantai-lakehouse-oidc-mock:0.2.0@{DIGEST_B}", rendered)

    def test_the_rendered_file_keeps_the_comments_of_the_template(self) -> None:
        rendered = render()
        self.assertIn("# The API's own image, built from source.", rendered)

    def test_the_rendered_file_says_that_it_is_generated(self) -> None:
        rendered = render()
        self.assertTrue(rendered.startswith("# GENERATED FILE"))

    def test_every_image_in_the_rendered_file_carries_a_digest(self) -> None:
        rendered = render()
        for _number, _service, value in render_compose.check_image_digests.image_lines(rendered):
            self.assertTrue(
                render_compose.check_image_digests.check_image(value),
                f"image without a digest: {value}",
            )

    def test_a_gate_service_is_dropped_from_the_rendered_file(self) -> None:
        rendered = render()
        self.assertNotIn("g1-test-runner", rendered)
        self.assertNotIn("g6-test-runner", rendered)

    def test_a_gate_service_that_a_kept_service_waits_for_stays(self) -> None:
        rendered = render()
        self.assertIn("g4-source-init", rendered)
        self.assertIn("debezium-server", rendered)

    def test_a_locally_built_service_without_a_published_image_is_refused(self) -> None:
        with self.assertRaises(render_compose.RenderError):
            render_compose.render(TEMPLATE, {})

    def test_a_published_image_without_a_digest_is_refused(self) -> None:
        with self.assertRaises(render_compose.RenderError):
            render_compose.render(
                TEMPLATE,
                {
                    "lakehouse-api": "ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0",
                    "oidc-mock": IMAGES["oidc-mock"],
                },
            )

    def test_a_third_party_image_without_a_digest_is_refused(self) -> None:
        broken = TEMPLATE.replace(
            f"postgres:16.15@{DIGEST_A}", "postgres:16.15", 1
        )
        with self.assertRaises(render_compose.RenderError):
            render_compose.render(broken, IMAGES)


class GateServiceTest(unittest.TestCase):
    """Which services the renderer calls a harness."""

    def test_a_service_behind_a_gate_profile_is_a_harness(self) -> None:
        _header, services, _trailer = render_compose.parse_compose(TEMPLATE)
        by_name = {service.name: service for service in services}
        self.assertTrue(render_compose.is_gate_service(by_name["g1-test-runner"]))
        self.assertTrue(render_compose.is_gate_service(by_name["g6-test-runner"]))

    def test_a_service_in_the_dagster_profile_is_not_a_harness_only_for_its_name(self) -> None:
        _header, services, _trailer = render_compose.parse_compose(TEMPLATE)
        by_name = {service.name: service for service in services}
        # `g4-source-init` IS named after a gate, so it is a harness by
        # name — and it stays anyway, because `debezium-server` waits for
        # it. `postgres` is not a harness at all.
        self.assertTrue(render_compose.is_gate_service(by_name["g4-source-init"]))
        self.assertFalse(render_compose.is_gate_service(by_name["postgres"]))

    def test_a_harness_named_after_its_gate_is_a_harness_even_in_the_dagster_profile(self) -> None:
        template = TEMPLATE.replace(
            "volumes:\n",
            "  g7-test-runner:\n"
            '    profiles: ["dagster"]\n'
            "    build:\n"
            "      context: ./ops/g7\n"
            "\n"
            "volumes:\n",
        )
        _header, services, _trailer = render_compose.parse_compose(template)
        by_name = {service.name: service for service in services}
        self.assertTrue(render_compose.is_gate_service(by_name["g7-test-runner"]))
        self.assertIn("g7-test-runner", render_compose.services_to_drop(services))


class RealTemplateTest(unittest.TestCase):
    """The template the release actually renders."""

    def setUp(self) -> None:
        self.template = render_compose.COMPOSE_PATH.read_text(encoding="utf-8")
        self.images = {
            name: f"ghcr.io/rantai-dev/rantai-lakehouse-{name}:0.2.0@{DIGEST_A}"
            for name in (
                "lakehouse-api",
                "console",
                "oidc-mock",
                "dagster-code-location",
                "dagster-webserver",
                "dagster-daemon",
                "backup-job",
            )
        }
        self.rendered = render_compose.render(self.template, self.images)

    def test_the_rendered_template_has_no_build_block_left(self) -> None:
        self.assertNotIn("\n    build:", self.rendered)

    def test_every_image_in_the_rendered_template_carries_a_digest(self) -> None:
        for _number, _service, value in render_compose.check_image_digests.image_lines(
            self.rendered
        ):
            self.assertTrue(
                render_compose.check_image_digests.check_image(value),
                f"image without a digest: {value}",
            )

    def test_the_rendered_template_keeps_the_product_services(self) -> None:
        for name in (
            "lakehouse-api",
            "console",
            "oidc-mock",
            "dagster-code-location",
            "dagster-webserver",
            "dagster-daemon",
            "backup-job",
            "postgres",
            "clickhouse",
            "lakekeeper",
            "seaweedfs",
            "trino",
        ):
            self.assertIn(f"\n  {name}:", self.rendered)

    def test_the_rendered_template_drops_the_acceptance_harnesses(self) -> None:
        for name in (
            "g1-test-runner",
            "g2-test-runner",
            "g3a-test-runner",
            "g4-test-runner",
            "g6-test-runner",
            "g7-test-runner",
            "g8-test-runner",
            "g9-test-runner",
            "bld1-test-runner",
            "gold-export-test-runner",
            "rest-stub-g6",
            "saas-stub-g6",
            "mysql-g6",
            "mssql-g6",
            "mongo-g6",
            "kafka-g6",
        ):
            self.assertNotIn(f"\n  {name}:", self.rendered)

    def test_the_rendered_template_keeps_the_one_harness_a_product_service_waits_for(self) -> None:
        # `debezium-server` waits for `g4-source-init` (the demo table and
        # its publication), so dropping that harness would leave the CDC
        # service waiting for something that never runs.
        self.assertIn("\n  g4-source-init:", self.rendered)


if __name__ == "__main__":
    unittest.main()
