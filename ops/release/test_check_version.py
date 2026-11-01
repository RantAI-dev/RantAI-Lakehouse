#!/usr/bin/env python3
"""Tests for `ops/release/check_version.py` (REL-2 S2-T1).

No network, no repository: the check reads four files, and these tests
write four small files of their own and ask the check what it makes of
them. They run under `pytest ops/release` and under
`python3 -m unittest discover -s ops/release` alike.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_version  # noqa: E402


PACKAGE_JSON = """{
  "name": "rantai-lakehouse-console",
  "version": "0.2.0"
}
"""

CARGO_TOML = """[package]
name = "lakehouse-api"
version = "0.2.0"
edition.workspace = true

[dependencies]
lakehouse-core = { path = "../lakehouse-core" }
"""

CHANGELOG = """# Changelog

## [Unreleased]

### Added

- Something that has not been released yet.

## [0.2.0] - 2026-11-01

### Added

- The first release through the pipeline.
"""


class VersionCheckTest(unittest.TestCase):
    """What the check says about four files that may disagree."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.package_json = self.root / "package.json"
        self.cargo_toml = self.root / "rust" / "crates" / "lakehouse-api" / "Cargo.toml"
        self.changelog = self.root / "CHANGELOG.md"
        self.cargo_toml.parent.mkdir(parents=True, exist_ok=True)
        self.package_json.write_text(PACKAGE_JSON, encoding="utf-8")
        self.cargo_toml.write_text(CARGO_TOML, encoding="utf-8")
        self.changelog.write_text(CHANGELOG, encoding="utf-8")

    def tearDown(self) -> None:
        self._directory.cleanup()

    def check(self, tag: str) -> list[str]:
        return check_version.check(
            tag,
            package_json=self.package_json,
            cargo_toml=self.cargo_toml,
            changelog=self.changelog,
        )

    def test_the_tag_the_package_the_crate_and_the_changelog_may_all_name_the_same_version(self) -> None:
        self.assertEqual(self.check("v0.2.0"), [])

    def test_a_package_that_names_another_version_than_the_tag_is_refused(self) -> None:
        self.package_json.write_text(
            PACKAGE_JSON.replace('"0.2.0"', '"0.1.0"'), encoding="utf-8"
        )
        problems = self.check("v0.2.0")
        self.assertEqual(len(problems), 1)
        self.assertIn("package.json", problems[0])
        self.assertIn("0.1.0", problems[0])

    def test_a_crate_that_names_another_version_than_the_tag_is_refused(self) -> None:
        self.cargo_toml.write_text(
            CARGO_TOML.replace('version = "0.2.0"', 'version = "0.1.0"'), encoding="utf-8"
        )
        problems = self.check("v0.2.0")
        self.assertEqual(len(problems), 1)
        self.assertIn("API crate", problems[0])
        self.assertIn("0.1.0", problems[0])

    def test_a_changelog_that_names_another_version_than_the_tag_is_refused(self) -> None:
        self.changelog.write_text(
            CHANGELOG.replace("## [0.2.0] - 2026-11-01", "## [0.1.0] - 2026-08-30"),
            encoding="utf-8",
        )
        problems = self.check("v0.2.0")
        self.assertEqual(len(problems), 1)
        self.assertIn("changelog", problems[0])
        self.assertIn("0.2.0", problems[0])

    def test_a_changelog_without_a_heading_for_the_version_is_refused(self) -> None:
        self.changelog.write_text(
            CHANGELOG.replace("## [0.2.0] - 2026-11-01", "## [0.2.0]"),
            encoding="utf-8",
        )
        problems = self.check("v0.2.0")
        self.assertEqual(len(problems), 1)
        self.assertIn("changelog", problems[0])

    def test_every_difference_is_reported_and_not_only_the_first(self) -> None:
        self.package_json.write_text(
            PACKAGE_JSON.replace('"0.2.0"', '"0.1.0"'), encoding="utf-8"
        )
        self.cargo_toml.write_text(
            CARGO_TOML.replace('version = "0.2.0"', 'version = "0.1.0"'), encoding="utf-8"
        )
        self.changelog.write_text(
            CHANGELOG.replace("## [0.2.0] - 2026-11-01", "## [0.1.0] - 2026-08-30"),
            encoding="utf-8",
        )
        self.assertEqual(len(self.check("v0.2.0")), 3)

    def test_a_tag_that_is_not_a_release_tag_is_refused(self) -> None:
        problems = self.check("0.2.0")
        self.assertEqual(len(problems), 1)
        self.assertIn("release tag", problems[0])


class CommandLineTest(unittest.TestCase):
    """The exit code the pipeline reads."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        (self.root / "rust" / "crates" / "lakehouse-api").mkdir(parents=True)
        (self.root / "package.json").write_text(PACKAGE_JSON, encoding="utf-8")
        (self.root / "rust" / "crates" / "lakehouse-api" / "Cargo.toml").write_text(
            CARGO_TOML, encoding="utf-8"
        )
        (self.root / "CHANGELOG.md").write_text(CHANGELOG, encoding="utf-8")

    def tearDown(self) -> None:
        self._directory.cleanup()

    def test_the_command_line_succeeds_when_the_four_versions_agree(self) -> None:
        code = check_version.main(["v0.2.0", "--repo-root", str(self.root)])
        self.assertEqual(code, 0)

    def test_the_command_line_fails_when_one_version_differs(self) -> None:
        (self.root / "package.json").write_text(
            PACKAGE_JSON.replace('"0.2.0"', '"0.1.0"'), encoding="utf-8"
        )
        code = check_version.main(["v0.2.0", "--repo-root", str(self.root)])
        self.assertEqual(code, 1)


class RepositoryVersionTest(unittest.TestCase):
    """The repository's own four files, as they stand in the checkout."""

    def test_the_checked_out_repository_names_one_version_in_all_four_places(self) -> None:
        problems = check_version.check("v" + check_version.package_version(check_version.PACKAGE_JSON))
        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
