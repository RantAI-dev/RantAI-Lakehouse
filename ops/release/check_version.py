#!/usr/bin/env python3
"""Check that a release tag, the console package, the API crate and the
changelog all name the same version (REL-2 S2-T1).

Why this exists
---------------
A release carries its version in FOUR places:

  - the git tag (`v0.2.0`);
  - `package.json` — the console's own version, which the account menu
    shows (REL-2 S1-T5);
  - `rust/crates/lakehouse-api/Cargo.toml` — the version `CARGO_PKG_VERSION`
    compiles into the API, which `/health` and `/health/ready` report;
  - the `## [x.y.z] - YYYY-MM-DD` heading in `CHANGELOG.md`.

They drift: a developer bumps the tag and forgets the crate, or renames
the changelog section and forgets the package. Nothing downstream can
notice — every image still builds, the pipeline still publishes — and the
result is a release whose console says one thing, whose API says another
and whose changelog says a third. The operator is the one who finds out,
from a version banner on a page they did not expect. So the pipeline
checks the four before it publishes anything.

What it does
------------
`check_version.py <tag>` fails unless all four name the same version. It
reads the four files from the repository root (`--repo-root` overrides
it), and it prints EVERY difference it finds, not only the first, so one
run tells the whole story.

What it deliberately does NOT do
--------------------------------
- It does not decide what the version SHOULD be and it does not bump
  anything. The version is the owner's decision (the release policy in
  `docs/core/reference/release-policy.md`).
- It does not check the changelog's CONTENT, only that a heading for the
  version exists. What the release notes say is a review question, not a
  script's.
- It does not look at the git tag itself: the tag is the argument, and
  the pipeline calls this on a `v*` tag push, where the tag is the
  trigger.
- It does not check the version of any other crate in the workspace, or
  the Dagster code location's `pyproject.toml`: those are not shipped to
  an operator as "the version of the release".

Python 3.11 or newer: the crate's manifest is read with `tomllib`, which
is in the standard library from 3.11.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

PACKAGE_JSON = REPO_ROOT / "package.json"
API_CARGO_TOML = REPO_ROOT / "rust" / "crates" / "lakehouse-api" / "Cargo.toml"
CHANGELOG = REPO_ROOT / "CHANGELOG.md"

# A release tag: `v` and three numbers, nothing else.
TAG_RE = re.compile(r"^v(?P<version>\d+\.\d+\.\d+)$")

# A changelog release heading: `## [0.2.0] - 2026-11-01`. The date is
# part of the heading because the release policy requires the release to
# carry one; a heading without it is not a release heading.
CHANGELOG_HEADING_RE = re.compile(
    r"^## \[(?P<version>\d+\.\d+\.\d+)\] - (?P<date>\d{4}-\d{2}-\d{2})\s*$",
    re.MULTILINE,
)


class VersionReadError(RuntimeError):
    """A version could not be read from one of the four files.

    Raised instead of guessing: a file that cannot be parsed is a broken
    checkout, and a version check that skips a file it cannot read is
    not a check.
    """


def version_from_tag(tag: str) -> str | None:
    """The version a tag names, or None when the tag is not a release tag."""
    match = TAG_RE.match(tag.strip())
    return match.group("version") if match else None


def package_version(path: Path) -> str:
    """The console's version, read from `package.json`."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VersionReadError(f"{path} could not be read as JSON: {error}") from error
    version = data.get("version")
    if not isinstance(version, str):
        raise VersionReadError(f"{path} has no `version` string")
    return version


def cargo_version(path: Path) -> str:
    """The API's version, read from the crate's `Cargo.toml`."""
    try:
        with path.open("rb") as handle:
            data = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise VersionReadError(f"{path} could not be read as TOML: {error}") from error
    version = data.get("package", {}).get("version")
    if not isinstance(version, str):
        raise VersionReadError(f"{path} has no `package.version` string")
    return version


def changelog_versions(text: str) -> dict[str, str]:
    """Every release heading in the changelog, as version -> date."""
    found: dict[str, str] = {}
    for match in CHANGELOG_HEADING_RE.finditer(text):
        found[match.group("version")] = match.group("date")
    return found


def check(
    tag: str,
    *,
    package_json: Path = PACKAGE_JSON,
    cargo_toml: Path = API_CARGO_TOML,
    changelog: Path = CHANGELOG,
) -> list[str]:
    """Every reason the tag and the three files disagree, as sentences.

    An empty list means the four name the same version.
    """
    version = version_from_tag(tag)
    if version is None:
        return [
            f"the tag {tag!r} is not a release tag: it must be `v<major>.<minor>.<patch>`"
        ]

    problems: list[str] = []

    package = package_version(package_json)
    if package != version:
        problems.append(
            f"the tag names {version} but {package_json.name} names {package}"
        )

    crate = cargo_version(cargo_toml)
    if crate != version:
        problems.append(
            f"the tag names {version} but the API crate names {crate}"
        )

    try:
        changelog_text = changelog.read_text(encoding="utf-8")
    except OSError as error:
        raise VersionReadError(f"{changelog} could not be read: {error}") from error
    headings = changelog_versions(changelog_text)
    if version not in headings:
        problems.append(
            f"the changelog has no `## [{version}] - YYYY-MM-DD` heading"
        )

    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail unless the tag, package.json, the API crate and the "
        "changelog all name the same version.",
    )
    parser.add_argument("tag", help="the release tag, e.g. v0.2.0")
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=REPO_ROOT,
        help="the repository root the four files are read from",
    )
    args = parser.parse_args(argv)

    root: Path = args.repo_root
    try:
        problems = check(
            args.tag,
            package_json=root / "package.json",
            cargo_toml=root / "rust" / "crates" / "lakehouse-api" / "Cargo.toml",
            changelog=root / "CHANGELOG.md",
        )
    except VersionReadError as error:
        print(f"version check failed: {error}", file=sys.stderr)
        return 1

    if problems:
        for problem in problems:
            print(f"version check failed: {problem}", file=sys.stderr)
        print(
            "version check failed: the tag, the console package, the API crate "
            "and the changelog must name the same version. Fix the files, or "
            "fix the tag.",
            file=sys.stderr,
        )
        return 1

    version = version_from_tag(args.tag)
    print(
        f"version check: the tag, {root / 'package.json'}, the API crate and "
        f"the changelog all name {version}."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
