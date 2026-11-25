#!/usr/bin/env python3
"""REL-16 — the offline init image and the end of runtime package installs.

Before this change five one-shot init containers ran plain Alpine and
installed curl, jq or openssl from their `command:` on EVERY start. A
truly offline install has no package mirror, so the stack failed on first
start even though every image in the release bundle was present. The fix
is one small first-party image built and shipped like the other first-
party images, plus an air-gap proof that blocks container egress.

These tests are pure text/YAML checks: no Docker daemon and no network.
The workflow contract itself is tested in `test_release_workflow.py`.
"""

from __future__ import annotations

import json
import re
import sys
import unittest
from pathlib import Path
from typing import Any

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
RELEASE_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(RELEASE_DIR))

import compose_images  # noqa: E402
import render_compose  # noqa: E402

COMPOSE_PATH = REPO_ROOT / "docker-compose.yml"
DOCKERFILE_PATH = REPO_ROOT / "ops" / "init" / "Dockerfile"
MAPPING_PATH = RELEASE_DIR / "release-images.json"
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "release.yml"

DIGEST = "sha256:" + "a" * 64
ALPINE_DIGEST = "sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc"
INIT_IMAGE_REF = "${LAKEHOUSE_INIT_IMAGE:-rantai-lakehouse-init:local}"

# The five services that used to install packages at start.
INIT_SERVICES = (
    "lakekeeper-warehouse-init",
    "lakekeeper-authz-init",
    "lakekeeper-warehouse-init-seaweedfs",
    "lakekeeper-authz-init-seaweedfs",
    "mssql-g6-cert-init",
)

# The four of those a RELEASE runs. The G6 certificate job is gate-only,
# so `render_compose.py` drops it and it is not published.
RELEASED_INIT_SERVICES = INIT_SERVICES[:-1]


def compose_text() -> str:
    return COMPOSE_PATH.read_text(encoding="utf-8")


def service_block(text: str, name: str) -> str:
    """The lines of one service in the template."""
    _header, services, _trailer = render_compose.parse_compose(text)
    for service in services:
        if service.name == name:
            return "\n".join(service.lines)
    raise AssertionError(f"the template has no service {name!r}")


def without_comments(text: str) -> str:
    """Drop YAML/shell comment lines: they explain history, they do not run."""
    return "\n".join(
        line for line in text.splitlines() if not line.lstrip().startswith("#")
    )


def steps_of(document: dict[str, Any], job: str) -> list[dict[str, Any]]:
    return list(((document.get("jobs") or {}).get(job) or {}).get("steps") or [])


def step_named(document: dict[str, Any], job: str, name: str) -> dict[str, Any]:
    for step in steps_of(document, job):
        if step.get("name") == name:
            return step
    raise AssertionError(f"no step named {name!r} in {job!r}")


def workflow() -> dict[str, Any]:
    return yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))


class NoRuntimeInstallTest(unittest.TestCase):
    """A starting container must never need a package mirror."""

    def test_no_apk_add_remains_in_the_template(self) -> None:
        found = [
            (number, line.strip())
            for number, line in enumerate(without_comments(compose_text()).splitlines(), 1)
            if re.search(r"\bapk\s+add\b", line)
        ]
        self.assertEqual(found, [], f"runtime package installs remain: {found}")

    def test_no_init_service_installs_a_package_at_start(self) -> None:
        for name in INIT_SERVICES:
            block = without_comments(service_block(compose_text(), name))
            self.assertNotIn("apk", block, f"{name} still installs a package at start")
            self.assertNotIn("apt-get", block, f"{name} still installs a package at start")


class InitDockerfileTest(unittest.TestCase):
    """The image the init services run."""

    def setUp(self) -> None:
        self.text = DOCKERFILE_PATH.read_text(encoding="utf-8")

    def test_the_dockerfile_exists_and_pins_the_same_alpine_digest(self) -> None:
        self.assertIn(f"alpine:3.20@{ALPINE_DIGEST}", self.text)
        # The tag is a label, the digest is the identity: a file with only
        # `FROM alpine:3.20` would move under the release's feet.
        self.assertIn("@sha256:", self.text)

    def test_curl_jq_and_openssl_are_installed_at_build_time(self) -> None:
        run_lines = [
            line.strip()
            for line in self.text.splitlines()
            if line.strip().startswith("RUN ") and "apk add" in line
        ]
        self.assertTrue(run_lines, "the Dockerfile never installs the tools")
        install = " ".join(run_lines)
        self.assertIn("--no-cache", install)
        for tool in ("curl", "jq", "openssl"):
            self.assertIn(tool, install.split())

    def test_the_image_has_no_runtime_install_hook(self) -> None:
        # No ENTRYPOINT/CMD that installs anything: the compose services
        # own their commands, and the image only contributes the tools.
        self.assertNotIn("ENTRYPOINT", self.text)
        self.assertNotIn("CMD", self.text)


class ComposeWiringTest(unittest.TestCase):
    """All five init services run the one built image."""

    def setUp(self) -> None:
        self.text = compose_text()

    def test_every_init_service_uses_the_built_image_and_a_build_block(self) -> None:
        for name in INIT_SERVICES:
            block = service_block(self.text, name)
            self.assertIn(f"image: {INIT_IMAGE_REF}", block, name)
            self.assertIn("build:", block, name)
            self.assertIn("context: ./ops/init", block, name)

    def test_the_image_name_is_named_after_the_repository(self) -> None:
        self.assertIn("rantai-lakehouse-init", INIT_IMAGE_REF)


class ReleasePipelineTest(unittest.TestCase):
    """The image is built, mapped, rendered, bundled and signed."""

    def setUp(self) -> None:
        self.mapping = json.loads(MAPPING_PATH.read_text(encoding="utf-8"))

    def test_release_images_maps_every_released_init_service(self) -> None:
        self.assertEqual(
            set(self.mapping.get("rantai-lakehouse-init", [])),
            set(RELEASED_INIT_SERVICES),
        )
        # The gate-only certificate job is deliberately NOT published.
        self.assertNotIn("mssql-g6-cert-init", self.mapping["rantai-lakehouse-init"])

    def test_render_compose_replaces_all_five_builds_with_the_published_image(self) -> None:
        images = {
            service: f"ghcr.io/rantai-dev/{image_name}:0.2.0@{DIGEST}"
            for image_name, services in self.mapping.items()
            for service in services
        }
        rendered = render_compose.render(compose_text(), images)
        self.assertNotIn("\n    build:", rendered)
        for name in RELEASED_INIT_SERVICES:
            block = service_block(rendered, name)
            self.assertIn("rantai-lakehouse-init", block, name)
        # The G6 job is a harness and its SERVICE is dropped from the
        # release file (its name still appears in copied template comments).
        self.assertNotIn("\n  mssql-g6-cert-init:", rendered)

    def test_the_manifest_does_not_treat_the_init_image_as_third_party(self) -> None:
        published = {
            service: f"ghcr.io/rantai-dev/{image_name}:0.2.0@{DIGEST}"
            for image_name, services in self.mapping.items()
            for service in services
        }
        rendered = render_compose.render(compose_text(), published)
        images = compose_images.compose_images(rendered)
        third = compose_images.third_party(images, published)
        for name in RELEASED_INIT_SERVICES:
            self.assertNotIn(name, third, f"{name} would be pulled as third-party")
            self.assertIn(published[name], images.values())

    def test_the_release_workflow_builds_signs_and_sboms_the_init_image(self) -> None:
        document = workflow()
        for job in ("dry-run", "release"):
            build = step_named(document, job, "Build and push the init image")
            with_ = build.get("with") or {}
            self.assertEqual(with_.get("context"), "ops/init")
            self.assertEqual(with_.get("file"), "ops/init/Dockerfile")
            self.assertIn("rantai-lakehouse-init", str(with_.get("tags")))

            sbom = step_named(document, job, "The SBOM of the init image")
            self.assertIn(
                "rantai-lakehouse-init", str((sbom.get("with") or {}).get("output-file"))
            )
            self.assertEqual((sbom.get("with") or {}).get("format"), "cyclonedx-json")

            mapping_step = step_named(document, job, "The images the services run")
            run = str(mapping_step.get("run") or "")
            self.assertIn("--arg init", run)
            self.assertIn('"rantai-lakehouse-init": $init', run)


if __name__ == "__main__":
    unittest.main()
