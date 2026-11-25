#!/usr/bin/env python3
"""OPS-2 — the air gap is complete, not merely proved once (REL-12, REL-16).

Why this exists
---------------
REL-12 (#693) made the install and update proofs air-gapped IN FACT: a host
`OUTPUT` REJECT owned by uid 0 plus a must-fail `docker pull`. REL-16 (#711)
removed the last runtime package fetch — five init containers used to run
`apk add` on every start — by shipping one `rantai-lakehouse-init` image and
cutting container egress with `DOCKER-USER`. OPS-2 ("Install with no internet
access") is in acceptance only if that stays true: a release must carry every
byte a starting container needs, and nothing in it may reach a package mirror
or a registry.

These four guards hold that property where it can drift:

  1. the RENDERED release Compose file — no service fetches a package or
     downloads from an external host in `command:`/`entrypoint:`;
  2. every image that file runs is named by the signed manifest, so the
     bundle carries a `docker save` for it and Compose pulls nothing;
  3. `docs/OPERATIONS.md` keeps a self-contained air-gapped runbook;
  4. `lakehousectl` keeps pre-flight local: no internet check is added, and
     `--bundle` adds only the local containerd-store check.

What it deliberately does NOT do
--------------------------------
It does not run Docker, sign anything or touch the network. It renders the
real template with the placeholder digests `test_render_compose.py` uses, and
reads the same parser (`render_compose`, `compose_images`, `make_manifest`)
the release pipeline uses, so a guard cannot disagree with the release.
"""

from __future__ import annotations

import json
import re
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RELEASE_DIR = Path(__file__).resolve().parent
LINT_DIR = REPO_ROOT / "ops" / "lint"
for directory in (RELEASE_DIR, LINT_DIR):
    if str(directory) not in sys.path:
        sys.path.insert(0, str(directory))

import check_image_digests  # noqa: E402  (path set just above)
import compose_images  # noqa: E402  (path set just above)
import make_manifest  # noqa: E402  (path set just above)
import render_compose  # noqa: E402  (path set just above)

DIGEST = "sha256:" + "a" * 64
MAPPING_PATH = RELEASE_DIR / "release-images.json"
OPERATIONS_PATH = REPO_ROOT / "docs" / "OPERATIONS.md"
PREFLIGHT_PATH = REPO_ROOT / "rust" / "crates" / "lakehousectl" / "src" / "preflight.rs"
INSTALL_PATH = REPO_ROOT / "rust" / "crates" / "lakehousectl" / "src" / "install.rs"
RUNBOOK_HEADING = "## Air-gapped installation (no internet access)"

# Package managers that would need a mirror the air-gapped host has not got.
# The pattern is anchored on the verb: a URL in a comment or a health check
# is not an install.
PACKAGE_FETCH_RE = re.compile(
    r"\b(?:apk\s+add|apt(?:-get|\s+install)|dnf\s+install|yum\s+install|"
    r"pip(?:3)?\s+install|npm\s+(?:install|ci|i)\b|yarn\s+(?:add|install)|"
    r"gem\s+install)\b",
    re.IGNORECASE,
)
# A download is a LITERAL http(s) URL in a start command (curl/wget are
# the ones the release uses). The init jobs and health checks call internal
# Compose services (`lakekeeper`, `rustfs`, `openfga`, `localhost`); those
# are not internet egress. A variable holding a URL is not proof of one.
URL_RE = re.compile(r"https?://(?P<host>[A-Za-z0-9._-]+)", re.IGNORECASE)
PRIVATE_IPV4_RE = re.compile(
    r"^(?:10\.|192\.168\.|172\.(?:1[6-9]|2\d|3[01])\.)"
)


def published_images() -> dict[str, str]:
    """A realistic --images document: the five published images at a digest."""
    mapping = json.loads(MAPPING_PATH.read_text(encoding="utf-8"))
    return {
        service: f"ghcr.io/rantai-dev/{name}:0.2.0@{DIGEST}"
        for name, services in mapping.items()
        for service in services
    }


def render_release() -> str:
    """The Compose file the release ships, from the repository's template."""
    return render_compose.render(
        render_compose.COMPOSE_PATH.read_text(encoding="utf-8"), published_images()
    )


def rendered_services(rendered: str) -> list[render_compose.Service]:
    _header, services, _trailer = render_compose.parse_compose(rendered)
    return services


def startup_script(service: render_compose.Service) -> str:
    """A service's `command:` and `entrypoint:`, block scalars included.

    `healthcheck.test` is deliberately out: it probes the loopback address
    inside the container, which an air-gapped host can still reach.
    """
    lines: list[str] = []
    for key in ("command", "entrypoint"):
        index = render_compose.key_index(service.lines, key)
        if index is not None:
            lines.extend(service.lines[index : render_compose.key_span(service.lines, index)])
    return "\n".join(lines)


def internal_host(host: str, services: list[render_compose.Service]) -> bool:
    """Whether a literal URL host is local to the deployment, not internet."""
    host = host.lower()
    if host in {"localhost", "127.0.0.1", "::1"}:
        return True
    if PRIVATE_IPV4_RE.match(host):
        return True
    return host in {service.name for service in services}


def fetch_findings(rendered: str) -> list[tuple[str, str]]:
    """Every (service, reason) that would need the internet on first start."""
    services = rendered_services(rendered)
    findings: list[tuple[str, str]] = []
    for service in services:
        script = startup_script(service)
        if PACKAGE_FETCH_RE.search(script):
            findings.append((service.name, "installs a package at start"))
        for line in script.splitlines():
            if line.lstrip().startswith("#"):
                continue
            for match in URL_RE.finditer(line):
                host = match.group("host")
                if not internal_host(host, services):
                    findings.append((service.name, f"downloads from {host} at start"))
    return findings


def runbook() -> str:
    """The air-gapped section of the operator docs, or an empty string."""
    text = OPERATIONS_PATH.read_text(encoding="utf-8")
    start = text.find(RUNBOOK_HEADING)
    if start == -1:
        return ""
    following = text.find("\n## ", start + len(RUNBOOK_HEADING))
    return text[start:] if following == -1 else text[start:following]


def preflight_check_names() -> list[str]:
    """The names in `preflight.rs`'s `CHECKS` constant."""
    text = PREFLIGHT_PATH.read_text(encoding="utf-8")
    match = re.search(
        r"pub const CHECKS:\s*\[&str;\s*\d+\]\s*=\s*\[(?P<body>.*?)\];",
        text,
        re.DOTALL,
    )
    if match is None:
        return []
    return re.findall(r'"([^"]+)"', match.group("body"))


class NoRuntimeFetchTest(unittest.TestCase):
    """Guard 1: a release's containers start without a package mirror."""

    def setUp(self) -> None:
        self.rendered = render_release()

    def test_no_rendered_service_installs_a_package_when_it_starts(self) -> None:
        findings = [f for f in fetch_findings(self.rendered) if "installs a package" in f[1]]
        self.assertEqual(findings, [], f"runtime package installs remain: {findings}")

    def test_no_rendered_service_downloads_from_an_external_host_when_it_starts(self) -> None:
        findings = [
            f for f in fetch_findings(self.rendered) if "downloads from" in f[1]
        ]
        self.assertEqual(findings, [], f"runtime internet downloads remain: {findings}")

    def test_the_detector_fails_a_service_that_would_install_a_package(self) -> None:
        # A guard proves only the property it names; this one shows it would
        # actually catch the REL-16 regression rather than pass vacuously.
        template = """\
services:
  a-service:
    image: alpine:3.20@sha256:{digest}
    command: |
      apk add --no-cache curl
""".format(digest=DIGEST[7:])
        self.assertIn(
            ("a-service", "installs a package at start"), fetch_findings(template)
        )

    def test_the_detector_fails_a_service_that_would_download_from_a_mirror(self) -> None:
        template = """\
services:
  a-service:
    image: alpine:3.20@sha256:{digest}
    command: |
      curl -fsSL https://dl-cdn.alpinelinux.org/key | sh
""".format(digest=DIGEST[7:])
        self.assertIn(
            ("a-service", "downloads from dl-cdn.alpinelinux.org at start"),
            fetch_findings(template),
        )

    def test_internal_health_checks_are_not_mistaken_for_internet_egress(self) -> None:
        # The rendered file is FULL of `curl http://127.0.0.1:...` health
        # checks and init jobs calling `http://lakekeeper:8181`. A guard
        # that flagged those would be wrong, not strict.
        template = """\
services:
  a-service:
    image: alpine:3.20@sha256:{digest}
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:8080/health"]
    command: |
      curl -fsS http://a-service:8080/health || exit 1
""".format(digest=DIGEST[7:])
        self.assertEqual(
            fetch_findings(template), [], "a loopback or Compose-local call was flagged"
        )
        self.assertEqual(
            fetch_findings(self.rendered), [], "a real rendered service was flagged"
        )


class ImageCoverageTest(unittest.TestCase):
    """Guard 2: `docker compose up` cannot pull an image from the network."""

    def setUp(self) -> None:
        self.rendered = render_release()
        self.images = compose_images.compose_images(self.rendered)
        self.published = published_images()
        self.manifest_images = make_manifest.manifest_images(self.rendered, self.published)

    def test_every_image_line_of_the_rendered_file_is_named_by_the_manifest(self) -> None:
        self.assertNotEqual(self.images, {}, "the rendered file named no images")
        self.assertEqual(set(self.manifest_images), set(self.images))
        for service, reference in self.images.items():
            self.assertEqual(
                self.manifest_images[service],
                reference,
                f"{service} would be bundled under the wrong reference",
            )

    def test_every_reference_a_host_would_pull_has_a_digest_and_is_bundled(self) -> None:
        references = set(self.images.values())
        self.assertEqual(references, set(self.manifest_images.values()))
        for reference in references:
            self.assertTrue(
                check_image_digests.check_image(reference),
                f"an image would be pulled by tag on an air-gapped host: {reference}",
            )
            self.assertNotIn("${", reference, f"an unresolved variable: {reference}")

    def test_every_service_this_repository_publishes_is_in_the_rendered_file(self) -> None:
        # A built service left in the template as a `build:` block, or a
        # published image no rendered service runs, is a drift the bundle
        # cannot repair on an air-gapped host.
        self.assertTrue(set(self.published).issubset(set(self.images)))
        header, services, _trailer = render_compose.parse_compose(self.rendered)
        self.assertIn("GENERATED FILE", header[0])
        self.assertEqual(
            [service.name for service in services if service.has_build], []
        )


class OperatorRunbookTest(unittest.TestCase):
    """Guard 3: the operator can do the install with only these docs."""

    def setUp(self) -> None:
        self.section = runbook()

    def test_the_runbook_section_exists(self) -> None:
        self.assertNotEqual(self.section, "", "the air-gapped runbook is missing")

    def test_the_runbook_names_every_asset_and_command_it_depends_on(self) -> None:
        for token in (
            "SHA256SUMS",
            "release.pub",
            "lakehousectl install --bundle",
            "lakehousectl update --bundle",
            "lakehousectl rollback",
            "rantai-lakehouse-init",
            "gh release download",
        ):
            self.assertIn(token, self.section, f"the runbook does not say {token}")

    def test_the_runbook_states_the_join_and_verify_order(self) -> None:
        for token in ("part-*", "sha256sum -c", "cat", "tar --zstd"):
            self.assertIn(token, self.section, f"the runbook does not say {token}")

    def test_the_runbook_names_the_optional_profiles_the_bundle_carries(self) -> None:
        # From `render_compose.parse_compose` of the real template, not a
        # remembered list: `dagster`, `backups`, `trino`, `seaweedfs`, `cdc`.
        profiles = {p for service in rendered_services(render_release()) for p in service.profiles}
        for profile in ("dagster", "backups", "trino", "seaweedfs", "cdc"):
            self.assertIn(profile, profiles)
            self.assertIn(profile, self.section, f"the runbook omits profile {profile}")


class OfflinePreflightTest(unittest.TestCase):
    """Guard 4: pre-flight stays local, and `--bundle` adds no network check."""

    def setUp(self) -> None:
        self.preflight = PREFLIGHT_PATH.read_text(encoding="utf-8")
        self.install = INSTALL_PATH.read_text(encoding="utf-8")

    def test_no_preflight_check_needs_the_internet(self) -> None:
        names = preflight_check_names()
        self.assertTrue(names, "the CHECKS constant could not be read")
        forbidden = ("internet", "connectiv", "online", "network", "registry", "mirror", "reachab")
        for name in names:
            for word in forbidden:
                self.assertNotIn(word, name.lower(), f"a network check was added: {name}")

    def test_preflight_checks_are_pure_functions_of_locally_gathered_facts(self) -> None:
        self.assertIn("type Check = fn(&Facts) -> Result<(), String>", self.preflight)
        for call in ("curl", "reqwest", "ureq", "TcpStream"):
            self.assertNotIn(call, self.preflight, f"pre-flight now calls {call}")

    def test_the_bundle_flag_only_gates_the_local_image_store_check(self) -> None:
        self.assertIn('if name == "image-store" && !bundle', self.preflight)
        self.assertIn("preflight::run(&facts, &args.ignore_preflight, args.bundle.is_some())", self.install)

    def test_a_rust_test_guards_the_offline_preflight(self) -> None:
        self.assertIn(
            "fn a_bundle_install_runs_no_internet_dependent_preflight_check()",
            self.preflight,
            "the Rust regression guard for an air-gapped pre-flight is missing",
        )


if __name__ == "__main__":
    unittest.main()
