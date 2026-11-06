#!/usr/bin/env python3
"""Tests for `ops/release/bump_api_version.py` (REL-2 S4b, PR #335).

The bump of the API crate's version is what the update proof's bundle B
is built on: the update waits for the API to report a version it does
not have yet, so the API image carries a version of its own. The bump
touches TWO files — the crate's manifest and the workspace's lockfile —
and the version has to be right in both, or the proof waits forever for
a version that never comes, or the build refuses a lockfile that
disagrees with the manifests.

The bump was inline Python in the workflow, and it was wrong (PR #335,
the `update-proof` job): it replaced the lockfile's version PREFIX and
left the old version text behind, so the lockfile said
`version = "0.1.0-update-proof.1"0.1.0"` and `cargo chef prepare`
stopped with a TOML parse error. These tests run the script on COPIES
of the repository's own manifest and lockfile — the real files are
never touched — and they hold the script to:

  - the version is set in both files, and both files still parse;
  - NOTHING ELSE in the lockfile changes, and nothing else in the
    manifest changes: a bump that rewrites a second version, or a
    dependency's version, is a bump that did something it did not mean
    to;
  - a file the bump cannot find the version in FAILS, loudly and
    non-zero: a bump that changed nothing is not a bump;
  - a manifest that INHERITS its version (`version.workspace = true`)
    is given one — the form the API crate does not use today, and the
    form the old inline code silently did nothing about.

No network, no cargo, no container: the tests read two files and write
copies of them.
"""

from __future__ import annotations

import difflib
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RELEASE_DIR = Path(__file__).resolve().parent
SCRIPT = RELEASE_DIR / "bump_api_version.py"

# The files the bump touches, as they are in the repository.
REAL_MANIFEST = REPO_ROOT / "rust" / "crates" / "lakehouse-api" / "Cargo.toml"
REAL_LOCK = REPO_ROOT / "rust" / "Cargo.lock"

# The version the proof's bundle B carries.
NEW_VERSION = "0.1.0-update-proof.1"


def real_api_version() -> str:
    """The API crate's version, as the repository carries it today.

    Read from the manifest instead of hardcoded: the release bump changes
    this version, and a test that names the pre-release one ("0.1.0")
    fails on every release for a reason that is not a defect — the 0.2.0
    bump found three such assertions here.
    """
    with REAL_MANIFEST.open("rb") as handle:
        return tomllib.load(handle)["package"]["version"]


class BumpApiVersionTest(unittest.TestCase):
    """What the bump does to the manifest and to the lockfile."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.manifest = self.root / "Cargo.toml"
        self.lock = self.root / "Cargo.lock"
        self.manifest.write_text(
            REAL_MANIFEST.read_text(encoding="utf-8"), encoding="utf-8"
        )
        self.lock.write_text(REAL_LOCK.read_text(encoding="utf-8"), encoding="utf-8")
        self.real_version = real_api_version()

    def tearDown(self) -> None:
        self._directory.cleanup()

    def run_bump(self, version: str = NEW_VERSION) -> subprocess.CompletedProcess[str]:
        """The script, run on the copies."""
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                version,
                "--manifest",
                str(self.manifest),
                "--lock",
                str(self.lock),
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_the_bump_sets_the_version_in_the_manifest_and_the_lock(self) -> None:
        """The version is the new one in both files, and both parse."""
        result = self.run_bump()
        self.assertEqual(result.returncode, 0, result.stderr)

        manifest = tomllib.loads(self.manifest.read_text(encoding="utf-8"))
        self.assertEqual(manifest["package"]["version"], NEW_VERSION)

        lock = tomllib.loads(self.lock.read_text(encoding="utf-8"))
        entries = [
            package for package in lock["package"] if package["name"] == "lakehouse-api"
        ]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["version"], NEW_VERSION)

    def test_the_bump_leaves_the_rest_of_the_lock_alone(self) -> None:
        """One line of the lockfile changes: the API crate's version."""
        before = REAL_LOCK.read_text(encoding="utf-8").splitlines()
        self.assertEqual(self.run_bump().returncode, 0)
        after = self.lock.read_text(encoding="utf-8").splitlines()

        self.assertEqual(len(before), len(after))
        changed = [
            (index, old, new)
            for index, (old, new) in enumerate(zip(before, after))
            if old != new
        ]
        self.assertEqual(
            len(changed),
            1,
            "the lockfile changed more than the version:\n"
            + "\n".join(
                difflib.unified_diff(before, after, "before", "after", lineterm="")
            ),
        )
        index, old, new = changed[0]
        self.assertEqual(old, f'version = "{self.real_version}"')
        self.assertEqual(new, f'version = "{NEW_VERSION}"')
        self.assertEqual(before[index - 1], 'name = "lakehouse-api"')

    def test_the_bump_leaves_the_rest_of_the_manifest_alone(self) -> None:
        """One line of the manifest changes: the crate's version."""
        before = REAL_MANIFEST.read_text(encoding="utf-8").splitlines()
        self.assertEqual(self.run_bump().returncode, 0)
        after = self.manifest.read_text(encoding="utf-8").splitlines()

        self.assertEqual(len(before), len(after))
        changed = [
            (index, old, new)
            for index, (old, new) in enumerate(zip(before, after))
            if old != new
        ]
        self.assertEqual(
            len(changed),
            1,
            "the manifest changed more than the version:\n"
            + "\n".join(
                difflib.unified_diff(before, after, "before", "after", lineterm="")
            ),
        )
        index, old, new = changed[0]
        self.assertEqual(old, f'version = "{self.real_version}"')
        self.assertEqual(new, f'version = "{NEW_VERSION}"')
        self.assertEqual(before[index - 1], 'name = "lakehouse-api"')

    def test_a_lock_without_the_api_entry_fails(self) -> None:
        """A lockfile with no `lakehouse-api` entry: the bump says so and
        exits non-zero, and it changes nothing."""
        text = self.lock.read_text(encoding="utf-8")
        self.lock.write_text(
            text.replace('name = "lakehouse-api"', 'name = "lakehouse-api-renamed"'),
            encoding="utf-8",
        )
        before = self.lock.read_text(encoding="utf-8")

        result = self.run_bump()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("lockfile", result.stderr)
        self.assertEqual(self.lock.read_text(encoding="utf-8"), before)

    def test_a_manifest_without_a_version_fails(self) -> None:
        """A manifest with no version line: the bump says so and exits
        non-zero, and it changes nothing."""
        lines = self.manifest.read_text(encoding="utf-8").splitlines(keepends=True)
        kept = [line for line in lines if not line.startswith("version = ")]
        self.manifest.write_text("".join(kept), encoding="utf-8")
        before = self.manifest.read_text(encoding="utf-8")

        result = self.run_bump()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("manifest", result.stderr)
        self.assertEqual(self.manifest.read_text(encoding="utf-8"), before)

    def test_a_manifest_that_inherits_its_version_is_given_one(self) -> None:
        """A crate with `version.workspace = true` gets the version
        itself: the line is replaced, not left alone."""
        lines = self.manifest.read_text(encoding="utf-8").splitlines(keepends=True)
        rewritten = [
            'version.workspace = true\n' if line.startswith("version = ") else line
            for line in lines
        ]
        self.manifest.write_text("".join(rewritten), encoding="utf-8")

        result = self.run_bump()
        self.assertEqual(result.returncode, 0, result.stderr)

        text = self.manifest.read_text(encoding="utf-8")
        package = text.split("[package]", 1)[1].split("[lib]", 1)[0]
        self.assertNotIn("version.workspace = true", package.splitlines())
        self.assertIn(f'version = "{NEW_VERSION}"', package.splitlines())
        manifest = tomllib.loads(text)
        self.assertEqual(manifest["package"]["version"], NEW_VERSION)

    def test_the_api_crate_declares_its_version_explicitly(self) -> None:
        """The API crate's manifest names its own version, so the bump
        rewrites that line. The inherited form is handled too (the test
        above), but the form this crate uses is recorded here."""
        text = REAL_MANIFEST.read_text(encoding="utf-8")
        package = text.split("[package]", 1)[1].split("[lib]", 1)[0]
        self.assertIn(f'version = "{real_api_version()}"', package.splitlines())
        self.assertNotIn("version.workspace = true", package.splitlines())


if __name__ == "__main__":
    unittest.main()
