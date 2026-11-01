#!/usr/bin/env python3
"""Render the release's Compose file from the deployment template
(REL-2 S2-T2).

Why this exists
---------------
`docker-compose.yml` is the TEMPLATE: it names images this repository
builds (`build:` blocks) and images it does not, and it carries the gate
harnesses the acceptance jobs run. A release needs a different file: one
that names only PUBLISHED images, each pinned by digest, and none of the
CI-only services. Writing that file by hand would mean editing a copy of
a 4,000-line template on every release, and the copy would drift from
the template the moment anyone changed it.

So the release file is RENDERED from the template, every time, by this
script. There is one source of truth (the template) and one rule for
what changes.

What it does
------------
`render_compose.py --images <json> -o <file>` writes the release's
Compose file:

  1. every service this repository BUILDS loses its `build:` block and
     names the published image at its digest instead. The images come
     from the `--images` JSON (service -> `ref@sha256:…`), which the
     pipeline writes after it has pushed the images and read their
     digests back;
  2. every gate or test service is dropped — the acceptance harnesses
     are CI's, not a deployment's;
  3. every image that is left must carry a digest. A file that names an
     image by tag alone is refused, not written.

The digest rule is the lint's rule, not a second one: this script
imports `ops/lint/check_image_digests.py` and uses ITS parser and ITS
`check_image`, so the release file cannot pass here and fail there.

It also answers ONE question about a Compose file, which is not a
transformation: `relative_mount_sources` names the relative bind-mount
sources of the services a release runs (REL-2 S4 decision D14). The
release ships every one of them, and the manifest names their
checksums; the rule lives here because this file owns the parsing of a
Compose file, and `make_manifest.py` asks this module for the answer
rather than parsing the file a second way (rule 4).

What it deliberately does NOT do
--------------------------------
- It does not touch the template. The template stays the template; the
  release file is an artifact of it.
- It does not verify that a digest exists in a registry. That needs a
  network, and the pipeline proves it by pushing the image and reading
  the digest back.
- It does not decide which services a deployment needs. It drops the
  gate and test services and keeps the rest; the profiles (`dagster`,
  `trino`, `seaweedfs`, `backups`) stay as they are, so an operator
  still chooses what to start.
- It does not resolve the compose file's `${VAR}` references. Compose
  does that, and the pipeline runs `docker compose -f <file> config`
  on the result.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
COMPOSE_PATH = REPO_ROOT / "docker-compose.yml"

# The lint that owns the image parser (rule 4: one parser, not two).
LINT_DIR = REPO_ROOT / "ops" / "lint"
if str(LINT_DIR) not in sys.path:
    sys.path.insert(0, str(LINT_DIR))

import check_image_digests  # noqa: E402  (path set just above)

# `  name:` — a service key. The lint's own pattern, so the two agree on
# what a service is.
SERVICE_RE = check_image_digests.SERVICE_RE

# `    image: value` — the lint's own pattern.
IMAGE_RE = check_image_digests.IMAGE_RE

# A key inside a service: `    profiles:`, `    build:`, `    image:` …
KEY_RE = re.compile(r"^    (?P<key>[A-Za-z0-9][A-Za-z0-9._-]*):(?P<rest>.*)$")

# A top-level key: `volumes:`, `networks:`, `secrets:`.
TOP_LEVEL_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]*:\s*$")

# A quoted string inside an inline list.
QUOTED_RE = re.compile(r"\"([^\"]*)\"|'([^']*)'")

# A short-syntax volume mount whose SOURCE is a relative path:
#
#     - ./ops/backups/clickhouse/backups.xml:/etc/clickhouse-server/config.d/x.xml:ro
#
# The target and the options are not read: what matters here is the
# source, and a source that is relative is a file the DEPLOYMENT needs
# next to its Compose file (REL-2 S4 decision D14).
MOUNT_RE = re.compile(r"^\s*-\s+(?P<source>\.{1,2}/[^\s:]+)(?::[^\s:]*)*\s*$")

# A gate or test PROFILE: `test`, `g1` … `g10`, `g6-sql`, `g6-other`,
# `g6-saas`, `g6-linklocal`. The acceptance harnesses run behind these;
# a deployment never does.
GATE_PROFILE_RE = re.compile(r"^g\d")

# A gate or test SERVICE NAME: `g1-test-runner`, `g3a-source-init`,
# `g7-test-runner`, `gold-export-test-runner`. The name is the harness's
# own naming: every acceptance gate in this repository names its runner
# after the gate.
GATE_NAME_RE = re.compile(r"^g\d")
GATE_NAME_SUFFIX = "-test-runner"

GENERATED_HEADER = """\
# GENERATED FILE — do not edit.
#
# Written by `ops/release/render_compose.py` from the deployment template
# (`docker-compose.yml`) for the release pipeline (REL-2 S2). Two things
# differ from the template:
#
#   1. the gate and test services are gone. They are the acceptance
#      harnesses CI runs, not part of a deployment;
#   2. every service this repository BUILDS names the PUBLISHED image at
#      its digest. There is no `build:` block left, so `docker compose up`
#      pulls what `release.json` names and never builds anything.
#
# The comments below are the template's, copied as they are: a comment
# that talks about building an image, or about a local `:local` tag,
# describes the TEMPLATE, not this file.
"""


class RenderError(RuntimeError):
    """The release file could not be rendered safely.

    Raised instead of writing a file that would be wrong: a release
    Compose file that names an image by tag, or that leaves a service
    building itself, is a file an operator would deploy.
    """


@dataclass(frozen=True)
class Service:
    """One service of a Compose file, as its lines and its facts."""

    name: str
    lines: tuple[str, ...]
    profiles: tuple[str, ...]
    has_build: bool
    image: str | None
    depends_on: tuple[str, ...]


def key_index(lines: list[str] | tuple[str, ...], key: str) -> int | None:
    """The index of a service's `key:` line, or None when it has none."""
    pattern = re.compile(rf"^    (?P<key>{re.escape(key)}):")
    for index, line in enumerate(lines):
        if pattern.match(line):
            return index
    return None


def key_span(lines: list[str] | tuple[str, ...], index: int) -> int:
    """The index after a key's last child line.

    A key's children are the lines indented under it. A line at the
    key's own indent (a sibling key, or a comment between two keys)
    ends them; a blank line inside them does not, so a multi-line
    command block stays whole. Trailing blank lines are not children.
    """
    end = index + 1
    last_content = index + 1
    while end < len(lines):
        line = lines[end]
        if line.strip() == "":
            end += 1
            continue
        if len(line) - len(line.lstrip(" ")) <= 4:
            break
        last_content = end + 1
        end += 1
    return last_content


def parse_profiles(lines: list[str] | tuple[str, ...], index: int) -> tuple[str, ...]:
    """The profiles of a service, from its `profiles:` key.

    Both shapes are read: the inline list this repository uses
    (`profiles: ["dagster"]`) and the block list compose also allows.
    """
    rest = KEY_RE.match(lines[index]).group("rest")  # type: ignore[union-attr]
    text = rest
    if "[" not in text:
        end = key_span(lines, index)
        for line in lines[index + 1 : end]:
            text += " " + line.strip()
    found: list[str] = []
    for match in QUOTED_RE.finditer(text):
        found.append(match.group(1) if match.group(1) is not None else match.group(2))
    return tuple(found)


def parse_service(name: str, lines: list[str]) -> Service:
    """The facts of one service, read from its block."""
    profiles: tuple[str, ...] = ()
    image: str | None = None
    depends_on: tuple[str, ...] = ()

    index = key_index(lines, "profiles")
    if index is not None:
        profiles = parse_profiles(lines, index)

    index = key_index(lines, "image")
    if index is not None:
        match = IMAGE_RE.match(lines[index])
        if match:
            image = match.group("value")

    index = key_index(lines, "depends_on")
    if index is not None:
        end = key_span(lines, index)
        children: list[str] = []
        for line in lines[index + 1 : end]:
            match = re.match(r"^      (?P<name>[A-Za-z0-9][A-Za-z0-9._-]*):\s*$", line)
            if match:
                children.append(match.group("name"))
        depends_on = tuple(children)

    return Service(
        name=name,
        lines=tuple(lines),
        profiles=profiles,
        has_build=key_index(lines, "build") is not None,
        image=image,
        depends_on=depends_on,
    )


def parse_compose(text: str) -> tuple[list[str], list[Service], list[str]]:
    """A Compose file as its header, its services and its trailer.

    The header is everything before `services:`, the trailer everything
    from the next top-level key (`volumes:`, `networks:`, `secrets:`)
    onwards. Both are copied to the output unchanged.
    """
    lines = text.splitlines()
    start = None
    for index, line in enumerate(lines):
        if line == "services:":
            start = index
            break
    if start is None:
        raise RenderError("the file has no `services:` block")

    services: list[Service] = []
    current: list[str] | None = None
    current_name = ""
    for line in lines[start + 1 :]:
        if TOP_LEVEL_RE.match(line):
            break
        match = SERVICE_RE.match(line)
        if match:
            if current is not None:
                services.append(parse_service(current_name, current))
            current = [line]
            current_name = match.group("name")
            continue
        if current is not None:
            current.append(line)
    if current is not None:
        services.append(parse_service(current_name, current))

    header = lines[:start]
    # The trailer starts at the first top-level key after `services:`.
    trailer_start = len(lines)
    for index in range(start + 1, len(lines)):
        if TOP_LEVEL_RE.match(lines[index]):
            trailer_start = index
            break
    trailer = lines[trailer_start:]
    return header, services, trailer


def is_gate_profile(profile: str) -> bool:
    """Whether a profile is an acceptance harness's."""
    return profile == "test" or GATE_PROFILE_RE.match(profile) is not None


def is_gate_service(service: Service) -> bool:
    """Whether a service belongs to a gate or a test run.

    Two shapes, because the harnesses are named two ways:

      - the PROFILE: a service behind `test`, `g6`, `g6-sql` … is a
        harness. A service in the `dagster`, `trino`, `seaweedfs` or
        `backups` profile is not;
      - the NAME: `g1-test-runner`, `g3a-source-init`, `g7-test-runner`,
        `gold-export-test-runner`. These live in the `dagster` profile
        (the gates share it), so the profile alone cannot tell them from
        the product's Dagster services.
    """
    if GATE_NAME_RE.match(service.name) is not None:
        return True
    if service.name.endswith(GATE_NAME_SUFFIX):
        return True
    if service.profiles and all(is_gate_profile(p) for p in service.profiles):
        return True
    return False


def services_to_drop(services: list[Service]) -> set[str]:
    """The gate and test services that are not needed by what stays.

    A harness is dropped — unless a service that STAYS waits for it to
    have run. `debezium-server` waits for `g4-source-init` (the demo
    table and its publication), so dropping that one would leave the
    CDC service waiting for something that never runs. The exception is
    narrow and deliberate: it is computed from the `depends_on` graph,
    not from a list of names.
    """
    candidates = {service.name for service in services if is_gate_service(service)}
    if not candidates:
        return set()

    kept = {service.name for service in services if service.name not in candidates}
    needed: set[str] = set()
    changed = True
    while changed:
        changed = False
        for service in services:
            if service.name not in kept and service.name not in needed:
                continue
            for dependency in service.depends_on:
                if dependency in candidates and dependency not in needed:
                    needed.add(dependency)
                    kept.add(dependency)
                    changed = True
    return candidates - needed


def relative_mount_sources(text: str) -> list[str]:
    """The relative bind-mount sources of the services a RELEASE runs.

    A relative source (`./ops/backups/clickhouse/backups.xml`) is a file
    the deployment needs NEXT TO its Compose file: Docker creates an
    empty directory in its place when the file is missing, and the
    service that mounts it then runs against nothing (REL-2 S4 decision
    D14). So the release ships every one of them.

    The services of a TEST profile are left out — the acceptance
    harnesses are CI's, not a deployment's — and the gate services are
    left out by the SAME rule the renderer drops them by
    (`services_to_drop`), so the answer is the same whether it is read
    from the template or from the rendered file.

    The paths are returned WITHOUT the leading `./`: that is the path
    relative to the directory the Compose file lives in, which is the
    path the install writes the file at.
    """
    _, services, _ = parse_compose(text)
    dropped = services_to_drop(services)
    sources: set[str] = set()
    for service in services:
        if service.name in dropped:
            continue
        for line in service.lines:
            match = MOUNT_RE.match(line)
            if match is None:
                continue
            source = match.group("source")
            sources.add(source[2:] if source.startswith("./") else source)
    return sorted(sources)


def published_ref(images: dict[str, str], service: Service) -> str:
    """The published reference of a locally built service, or an error.

    A service this repository builds and the pipeline did not publish
    has no image to name. The file is refused rather than written with
    a `build:` block left in it.
    """
    reference = images.get(service.name)
    if reference is None:
        raise RenderError(
            f"service `{service.name}` is built from source and the pipeline "
            f"published no image for it. Add it to the --images JSON."
        )
    if not check_image_digests.check_image(reference):
        raise RenderError(
            f"the published image of service `{service.name}` carries no digest: "
            f"{reference}. A tag is a name and a name moves; the release file "
            f"must name a digest."
        )
    return reference


def transform_service(service: Service, images: dict[str, str]) -> list[str]:
    """One service's block, with its build replaced by the published image."""
    lines = list(service.lines)
    if not service.has_build:
        return lines

    reference = published_ref(images, service)

    build_index = key_index(lines, "build")
    assert build_index is not None  # `has_build` said so
    build_end = key_span(lines, build_index)

    image_index = key_index(lines, "image")
    if image_index is not None:
        # The service already names an image: the published one replaces
        # it, and the build block goes.
        match = IMAGE_RE.match(lines[image_index])
        assert match is not None  # `parse_service` read this line
        lines[image_index] = f"    image: {reference}"
        del lines[build_index:build_end]
        return lines

    # The service named no image: the published one takes the build
    # block's place, so it sits where the build was.
    lines[build_index:build_end] = [f"    image: {reference}"]
    return lines


def render(text: str, images: dict[str, str]) -> str:
    """The release's Compose file, as text."""
    header, services, trailer = parse_compose(text)
    dropped = services_to_drop(services)

    out: list[str] = [GENERATED_HEADER.rstrip("\n"), ""]
    out.extend(header)
    out.append("")
    out.append("services:")
    for service in services:
        if service.name in dropped:
            continue
        out.extend(transform_service(service, images))
        out.append("")
    out.extend(trailer)

    rendered = "\n".join(out).rstrip("\n") + "\n"
    check_rendered(rendered)
    return rendered


def check_rendered(rendered: str) -> None:
    """Refuse a rendered file that is not safe to deploy.

    Three rules, and the file is refused if any of them fails:

      1. no `build:` block is left — a release file never builds;
      2. every image carries a digest;
      3. every service that names a build has an image (the transform
         guarantees it, so this is the belt to the transform's braces).
    """
    for number, line in enumerate(rendered.splitlines(), start=1):
        if re.match(r"^    build:\s*$", line):
            raise RenderError(
                f"the rendered file still has a `build:` block at line {number}"
            )

    for number, service, value in check_image_digests.image_lines(rendered):
        if not check_image_digests.check_image(value):
            raise RenderError(
                f"the rendered file names the image of service `{service}` at "
                f"line {number} without a digest: {value}"
            )


def load_images(value: str) -> dict[str, str]:
    """The published images, from an inline JSON document or a file.

    The pipeline builds the JSON with `jq -n --arg` and passes it inline;
    a file path is accepted too, because a long document is easier to
    read in a file.
    """
    text = value.strip()
    if text.startswith("{"):
        document = text
    else:
        document = Path(value).read_text(encoding="utf-8")
    try:
        data = json.loads(document)
    except ValueError as error:
        raise RenderError(f"the --images document is not JSON: {error}") from error
    if not isinstance(data, dict):
        raise RenderError("the --images document must be a JSON object")
    images: dict[str, str] = {}
    for name, reference in data.items():
        if not isinstance(name, str) or not isinstance(reference, str):
            raise RenderError(
                "the --images document must map a service name to an image reference"
            )
        images[name] = reference
    return images


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Render the release's Compose file from the template.",
    )
    parser.add_argument(
        "--images",
        required=True,
        help="the published images, as JSON: service -> ref@sha256:… "
        "(inline JSON or a path to a .json file)",
    )
    parser.add_argument(
        "-o",
        "--output",
        type=Path,
        required=True,
        help="the file to write",
    )
    parser.add_argument(
        "--compose",
        type=Path,
        default=COMPOSE_PATH,
        help="the template to render (default: docker-compose.yml)",
    )
    args = parser.parse_args(argv)

    try:
        images = load_images(args.images)
        rendered = render(args.compose.read_text(encoding="utf-8"), images)
    except (RenderError, OSError) as error:
        print(f"render_compose failed: {error}", file=sys.stderr)
        return 1

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(rendered, encoding="utf-8")
    print(f"render_compose: wrote {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
