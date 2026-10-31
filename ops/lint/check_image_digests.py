#!/usr/bin/env python3
"""CI lint for REL-2 S1-T3: every third-party image in
`docker-compose.yml` is pinned by digest.

# What this guards against

A tag is a NAME, and a name moves. `postgres:16` is "whatever the 16
series points at today"; `alpine/socat:latest` is "whatever the alpine
project published last"; `mysql:8` is "whatever MySQL's 8 series points
at today". A deployment that starts from a tag runs whatever the registry
serves at that moment, which is not the image anybody reviewed, tested or
wrote a release note about. On an air-gapped host the same name can even
mean two different images on two different days, once the operator loads
them by hand.

A digest is the image's own identity — the hash of its manifest. The same
digest is the same bits on every host, in every registry mirror, forever.
`name:tag@sha256:…` keeps the tag as the human label and the digest as
the fact.

The one image that already had this (`ghcr.io/memiiso/debezium-server-iceberg`)
became the model for the rest.

# What this script does

Reads `docker-compose.yml` (the deployment template) and fails when a
service's `image:` is

  1. a third-party image WITHOUT a `@sha256:` digest, or
  2. a third-party image whose digest is not 64 hex characters.

The images this repository BUILDS are exempt, and only they are: they are
named `${…_IMAGE:-…:local}` (`LAKEHOUSE_API_IMAGE`,
`LAKEHOUSE_DAGSTER_IMAGE`, `CONSOLE_IMAGE`, `G7_TEST_RUNNER_IMAGE`), so
their reference is a local tag by construction — there is no registry
digest to write until REL-2 S2 publishes them. Any other `${…}` value is
NOT exempt: an image reference that is only a variable hides whether it is
pinned, so it is a failure with the name it has.

# What this deliberately does NOT cover (read this before trusting a
# clean run)

- **Only `docker-compose.yml`.** The gate harnesses' own override files
  (`ops/g*/docker-compose.*.override.yml`) carry their own images
  (`mariadb`, `vault`, `moto`, ...) and are NOT checked here: they are
  CI-only test stacks, not the deployment template. Pinning them is a
  separate piece of work, and this lint says so rather than pretending
  the whole repository is covered.
- **The tag itself is not checked.** A digest pins the CONTENT, so
  `postgres:16@sha256:…` runs the same bits whatever the tag means
  later. The floating tags in this file were replaced with fixed ones in
  the same change (so the file reads the same on every host), but this
  lint does not fail on a floating tag — it fails on a missing digest,
  which is the thing that lets the bytes change.
- **The digest is not verified against the registry.** This is a static
  check on the checked-in file: it never talks to a registry, so it
  cannot tell whether a digest exists or what it points at. The proof
  that the pinned digests pull is the `docker compose up` in the change
  that pins them (REL-2 S1-T3's own commit body), not this lint.
- **Comments are not parsed.** The file is read line by line and an
  `image:` inside a comment is ignored — a comment cannot start a
  service.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
COMPOSE_PATH = REPO_ROOT / "docker-compose.yml"

# `image: <value>` — the value is one token on the line in this file.
IMAGE_RE = re.compile(r"^(\s+)image:\s*(?P<value>\S+)\s*$")

# A service key in the `services:` block (two spaces, then the name).
SERVICE_RE = re.compile(r"^  (?P<name>[A-Za-z0-9][A-Za-z0-9._-]*):\s*$")

# The locally BUILT images: `${LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}`.
# The variable name ends in `_IMAGE` and the default ends in `:local}`.
LOCAL_IMAGE_RE = re.compile(r"^\$\{[A-Z0-9_]*_IMAGE:-[^}]*:local\}$")

# A digest: `@sha256:` and 64 hex characters, at the end of the reference.
DIGEST_RE = re.compile(r"@sha256:[0-9a-f]{64}$")


def image_lines(text: str) -> list[tuple[int, str, str]]:
    """Every `image:` in the file, as (line number, service name, value).

    The service name is the closest service key above the line, so a
    failure says WHICH service is unpinned.
    """
    found: list[tuple[int, str, str]] = []
    service = "<unknown>"
    for number, line in enumerate(text.splitlines(), start=1):
        service_match = SERVICE_RE.match(line)
        if service_match:
            service = service_match.group("name")
            continue
        image_match = IMAGE_RE.match(line)
        if image_match:
            found.append((number, service, image_match.group("value")))
    return found


def is_local_build(value: str) -> bool:
    """Whether this reference names an image this repository builds."""
    return LOCAL_IMAGE_RE.match(value) is not None


def check_image(value: str) -> bool:
    """Whether a third-party reference carries a well-formed digest."""
    return DIGEST_RE.search(value) is not None


def main() -> int:
    text = COMPOSE_PATH.read_text(encoding="utf-8")
    failures: list[tuple[int, str, str]] = []
    pinned = 0
    local = 0
    for number, service, value in image_lines(text):
        if is_local_build(value):
            local += 1
            continue
        if check_image(value):
            pinned += 1
            continue
        failures.append((number, service, value))

    for number, service, value in failures:
        print(
            f"docker-compose.yml:{number}: the image of service `{service}` is not "
            f"pinned by digest: {value}"
        )
        print(
            "    fix: write it as `name:tag@sha256:<64 hex characters>` (the digest "
            "of the manifest the tag points at — `docker buildx imagetools inspect "
            "<image>` prints it), or name a locally built image "
            "`${..._IMAGE:-...:local}`"
        )

    if failures:
        print()
        print(
            f"image digest guard failed: {len(failures)} image(s) in "
            f"docker-compose.yml are not pinned by digest. A tag moves; a digest "
            f"does not. See ops/lint/check_image_digests.py."
        )
        return 1

    print(
        f"image digest guard: {pinned} third-party image(s) pinned by digest, "
        f"{local} locally built image(s) exempt."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
