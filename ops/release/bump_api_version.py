#!/usr/bin/env python3
"""Bump the API crate's version, for the update proof's bundle B (REL-2 S4b).

Why this exists
---------------
The `update-proof` job builds a SECOND API image for the update it
proves: the update waits for the API to report a version it does not
have yet, so the image it installs has to carry a version of its own.
That version lives in TWO files:

  - `rust/crates/lakehouse-api/Cargo.toml` — the version
    `CARGO_PKG_VERSION` compiles into the API, which `/health/ready`
    reports;
  - `rust/Cargo.lock` — the version of every workspace member. A build
    with `--locked` refuses a lockfile that disagrees with the
    manifests, so the lockfile is changed with the manifest.

The bump was a heredoc of Python inside the workflow, and it was wrong
in two ways (PR #335, the `update-proof` job, "Build and push the API
image of bundle B"):

  - it replaced the lockfile's `name = "lakehouse-api"\nversion = "`
    PREFIX and left the old version text behind, so the lockfile said
    `version = "0.1.0-update-proof.1"0.1.0"` and `cargo chef prepare`
    stopped with a TOML parse error;
  - it replaced the manifest's version with a pattern that matches a
    `version = "..."` line. A crate that INHERITS its version
    (`version.workspace = true`) has no such line, so the bump would
    leave `CARGO_PKG_VERSION` at the old version, and the update would
    wait for a version that never comes.

So the bump is a script of its own now, with the rules it follows
written down and tested (`ops/release/test_bump_api_version.py`):

  - the version is set in the manifest's `[package]` table — either the
    `version` key it already carries, or the `version.workspace = true`
    it inherits, which becomes the version itself;
  - the lockfile's entry for `lakehouse-api` has its WHOLE version
    value replaced, found with a regex anchored on the package's name;
  - a file that is not changed EXACTLY once is a failure, loudly: the
    bump touched nothing, or touched something it did not mean to;
  - both files are read back and parsed with `tomllib` afterwards, and
    the new version has to be in both.

What it deliberately does NOT do
--------------------------------
- It does not touch any other crate's version, and not the workspace's
  own `[workspace.package]` version: the proof changes the API's image
  and nothing else.
- It does not decide WHICH version: the caller passes it, and the
  proof's own rule owns that (REL-2 S4 decision D21,
  `ops/release/update_proof.py`).
- It does not run cargo: it edits two files and then reads them back.
  The build that follows is the job's own.

Python 3.11 or newer: the files are read with `tomllib`, which is in
the standard library from 3.11.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

# The repository this script belongs to. The default paths are read
# from there, so the script does not depend on the working directory it
# is called from.
REPO_ROOT = Path(__file__).resolve().parents[2]

# The API crate's manifest, and the workspace's lockfile.
DEFAULT_MANIFEST = "rust/crates/lakehouse-api/Cargo.toml"
DEFAULT_LOCK = "rust/Cargo.lock"

# The package the bump is about. The lockfile's entry is found with
# this name, and the manifest's `[package]` table has to carry it.
PACKAGE_NAME = "lakehouse-api"

# The manifest's own version line, inside the `[package]` table.
MANIFEST_VERSION_RE = re.compile(r'^version = "[^"]+"$', re.MULTILINE)

# A crate that inherits its version from the workspace. The bump
# replaces this ONE line with the version itself, so the crate stops
# inheriting and carries the version the proof needs.
MANIFEST_WORKSPACE_VERSION_RE = re.compile(r"^version\.workspace = true$", re.MULTILINE)

# The lockfile's entry for the package: the name, then the WHOLE
# version value. The old step replaced the PREFIX of this and left the
# old version text behind (PR #335).
LOCK_VERSION_RE = re.compile(
    r'(?P<head>name = "lakehouse-api"\nversion = ")(?P<version>[^"]*)(?P<tail>")'
)


class BumpError(RuntimeError):
    """The bump could not be made.

    Raised instead of guessing: a file that does not carry the version
    where it is expected is a file the bump must not touch.
    """


def package_table_span(text: str) -> tuple[int, int]:
    """The line span of the manifest's `[package]` table.

    The span is (start, end), both line indexes into `text.splitlines()`,
    and `end` is exclusive. A dependency's own `version` key lives
    outside it, so the bump cannot touch one by accident.
    """
    lines = text.splitlines()
    start: int | None = None
    for index, line in enumerate(lines):
        if line.strip() == "[package]":
            start = index
            break
    if start is None:
        raise BumpError("the manifest has no [package] table")

    end = len(lines)
    for index in range(start + 1, len(lines)):
        stripped = lines[index].strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            end = index
            break
    return start, end


def bump_manifest(text: str, version: str) -> str:
    """The manifest with the API crate's version set to `version`.

    A crate that inherits its version (`version.workspace = true`) has
    that one line replaced with the version itself; a crate that
    carries its own version has that line replaced.
    """
    lines = text.splitlines(keepends=True)
    start, end = package_table_span(text)
    section = "".join(lines[start:end])

    if MANIFEST_WORKSPACE_VERSION_RE.search(section):
        new_section, count = MANIFEST_WORKSPACE_VERSION_RE.subn(
            f'version = "{version}"', section
        )
        what = "version.workspace = true"
    else:
        new_section, count = MANIFEST_VERSION_RE.subn(f'version = "{version}"', section)
        what = 'version = "..."'

    if count != 1:
        raise BumpError(
            f"the manifest's [package] table holds {count} {what} lines; "
            f"the bump needs exactly one"
        )
    if new_section == section:
        raise BumpError(
            f"the manifest's [package] table already names {version}; "
            f"the bump changed nothing"
        )

    return "".join(lines[:start]) + new_section + "".join(lines[end:])


def bump_lock(text: str, version: str) -> str:
    """The lockfile with the API crate's version set to `version`.

    The entry is found with a regex anchored on the package's name, and
    its WHOLE version value is replaced — the old step replaced the
    text in front of the value and left the old value behind (PR #335).
    """
    new_text, count = LOCK_VERSION_RE.subn(
        lambda match: f"{match.group('head')}{version}{match.group('tail')}", text
    )
    if count != 1:
        raise BumpError(
            f"the lockfile holds {count} entries for {PACKAGE_NAME}; "
            f"the bump needs exactly one"
        )
    if new_text == text:
        raise BumpError(
            f"the lockfile already names {version} for {PACKAGE_NAME}; "
            f"the bump changed nothing"
        )
    return new_text


def read_manifest_version(text: str) -> str:
    """The version the manifest's `[package]` table carries."""
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise BumpError(f"the manifest is not TOML: {error}") from error
    package = data.get("package")
    if not isinstance(package, dict):
        raise BumpError("the manifest has no [package] table")
    name = package.get("name")
    if name != PACKAGE_NAME:
        raise BumpError(f"the manifest's package is {name!r}, not {PACKAGE_NAME!r}")
    version = package.get("version")
    if not isinstance(version, str):
        raise BumpError("the manifest's [package] table carries no version")
    return version


def read_lock_version(text: str) -> str:
    """The version the lockfile's `lakehouse-api` entry carries."""
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise BumpError(f"the lockfile is not TOML: {error}") from error
    packages = data.get("package")
    if not isinstance(packages, list):
        raise BumpError("the lockfile has no packages")
    found = [
        package
        for package in packages
        if isinstance(package, dict) and package.get("name") == PACKAGE_NAME
    ]
    if len(found) != 1:
        raise BumpError(f"the lockfile holds {len(found)} entries for {PACKAGE_NAME}")
    version = found[0].get("version")
    if not isinstance(version, str):
        raise BumpError(f"the lockfile's {PACKAGE_NAME} entry carries no version")
    return version


def bump(manifest_path: Path, lock_path: Path, version: str) -> None:
    """Set the API crate's version in the manifest and in the lockfile.

    Both files are read, changed, and checked BEFORE either is written:
    a bump that fails leaves the repository as it was, not with half of
    the version applied. Then both files are written and read back: the
    files that are on the disk afterwards have to parse, and they have
    to carry the new version.
    """
    manifest_text = manifest_path.read_text(encoding="utf-8")
    lock_text = lock_path.read_text(encoding="utf-8")

    new_manifest = bump_manifest(manifest_text, version)
    new_lock = bump_lock(lock_text, version)

    if read_manifest_version(new_manifest) != version:
        raise BumpError(f"the manifest would not name {version} after the bump")
    if read_lock_version(new_lock) != version:
        raise BumpError(f"the lockfile would not name {version} for {PACKAGE_NAME}")

    manifest_path.write_text(new_manifest, encoding="utf-8")
    lock_path.write_text(new_lock, encoding="utf-8")

    written_manifest = manifest_path.read_text(encoding="utf-8")
    written_lock = lock_path.read_text(encoding="utf-8")
    if read_manifest_version(written_manifest) != version:
        raise BumpError(f"the manifest does not name {version} after the bump")
    if read_lock_version(written_lock) != version:
        raise BumpError(f"the lockfile does not name {version} for {PACKAGE_NAME}")


def main(argv: list[str] | None = None) -> int:
    """The command line of the bump."""
    parser = argparse.ArgumentParser(
        description="Set the API crate's version in its manifest and in the "
        "workspace's lockfile.",
    )
    parser.add_argument("version", help="the version to set, e.g. 0.1.0-update-proof.1")
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=REPO_ROOT,
        help="the repository root the two files are read from by default",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help=f"the API crate's manifest (default: <repo-root>/{DEFAULT_MANIFEST})",
    )
    parser.add_argument(
        "--lock",
        type=Path,
        help=f"the workspace's lockfile (default: <repo-root>/{DEFAULT_LOCK})",
    )
    args = parser.parse_args(argv)

    manifest = args.manifest or args.repo_root / DEFAULT_MANIFEST
    lock = args.lock or args.repo_root / DEFAULT_LOCK

    try:
        bump(manifest, lock, args.version)
    except (BumpError, OSError) as error:
        print(f"version bump failed: {error}", file=sys.stderr)
        return 1

    print(f"version bump: {PACKAGE_NAME} is {args.version} in {manifest} and {lock}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
