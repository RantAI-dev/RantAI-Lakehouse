#!/usr/bin/env python3
"""Tests for the bundle in parts (REL-7 decision D1, D2).

GitHub refuses a release asset of 2 GiB or more, and the 0.2.0 release
failed on exactly that ("size must be less than 2147483648", a bundle of
3,794,642,569 bytes), so the release attaches the bundle split into
parts — `….tar.zst.part-00`, `.part-01`, … — and the operator joins them
(REL-7 D1, D2).

The join is the one thing in that path the pipeline cannot do for the
operator, so it is checked here on a small file: the parts are split the
way the release job splits them, checked against `SHA256SUMS` the way the
release's asset list names them, joined in name order, and the joined
file is checked against the whole file's `.sha256`. A DAMAGED part has to
be refused, and refused BEFORE the join, so the message names the
download to repeat.

The recipe under test is the one `docs/OPERATIONS.md` documents and the
dry run runs (`docs/OPERATIONS.md`, `.github/workflows/release.yml`): the
tests read that file and fail when the two drift apart, because a
documented join that the release does not produce is a bundle nobody can
install.

No network, no Docker: a temporary directory and a file of a few
kilobytes.
"""

from __future__ import annotations

import hashlib
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
OPERATIONS = REPO_ROOT / "docs" / "OPERATIONS.md"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"

# The bundle's name, as `make_bundle.sh` writes it and the release
# attaches it.
BUNDLE = "rantai-lakehouse-0.2.0-linux-x86_64.tar.zst"

# The parts' suffix. `split -d -a 2` writes `part-00`, `part-01`, …, so
# the name order the operator's `cat` uses is the byte order (REL-7 D2).
PART_SUFFIX = ".part-"

# The operator's commands, exactly as `docs/OPERATIONS.md` gives them
# (REL-7 D2), with the release's own file names: every part is checked
# against `SHA256SUMS`, the parts are joined in name order, and the
# joined file is checked against the whole file's `.sha256` before it is
# unpacked.
JOIN_RECIPE = """#!/usr/bin/env bash
set -euo pipefail
bundle="{bundle}"
sha256sum -c SHA256SUMS
cat "$bundle".part-* > "$bundle"
sha256sum -c "$bundle.sha256"
"""


def hashed(data: bytes) -> str:
    """The sha256 of some bytes, as `sha256sum` prints it."""
    return hashlib.sha256(data).hexdigest()


class BundlePartsTest(unittest.TestCase):
    """Split, check, join: the release's parts round-trip."""

    def setUp(self) -> None:
        self.directory = Path(tempfile.mkdtemp(prefix="rel7-bundle-parts-"))
        self.addCleanup(shutil.rmtree, self.directory, ignore_errors=True)
        # A file of a few kilobytes, split into three parts: small enough
        # to hash in a test, and more than one part, which is the case a
        # single-part bundle would not exercise.
        self.data = bytes(range(256)) * 8
        self.original_sha = hashed(self.data)
        (self.directory / "source").write_bytes(self.data)
        subprocess.run(
            [
                "split",
                "-b",
                "700",
                "-d",
                "-a",
                "2",
                self.directory / "source",
                self.directory / f"{BUNDLE}{PART_SUFFIX}",
            ],
            check=True,
        )
        # The whole file's checksum, as `make_bundle.sh` writes it: the
        # bare name, in the directory the file is in.
        (self.directory / f"{BUNDLE}.sha256").write_text(
            f"{self.original_sha}  {BUNDLE}\n", encoding="utf-8"
        )
        # The release's `SHA256SUMS` line for the parts, as the release
        # job writes it (`.github/workflows/release.yml`, "The release's
        # assets").
        parts = self.parts()
        self.assertGreater(len(parts), 1, "the split made one part: nothing to join")
        (self.directory / "SHA256SUMS").write_text(
            "".join(
                f"{hashed(part.read_bytes())}  ./{part.name}\n" for part in parts
            ),
            encoding="utf-8",
        )

    def parts(self) -> list[Path]:
        """The parts, as the operator's glob finds them: in name order."""
        return sorted(self.directory.glob(f"{BUNDLE}{PART_SUFFIX}*"))

    def join(self) -> subprocess.CompletedProcess[str]:
        """Run the documented join recipe in the directory."""
        script = self.directory / "join.sh"
        script.write_text(JOIN_RECIPE.format(bundle=BUNDLE), encoding="utf-8")
        return subprocess.run(
            ["bash", script],
            cwd=self.directory,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_the_documented_join_restores_the_file_the_release_published(self) -> None:
        """REL-7 D2: name order, then the whole file's own checksum."""
        result = self.join()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(hashed((self.directory / BUNDLE).read_bytes()), self.original_sha)
        self.assertIn("OK", result.stdout)

    def test_the_parts_are_in_name_order_and_are_the_byte_order(self) -> None:
        """`-d -a 2` pads the numbers, so a plain sort is the join order.

        A join in any other order is a different file, and the `.sha256`
        at the end is what says so (REL-7 D2).
        """
        names = [part.name for part in self.parts()]
        suffixes = [name.rsplit(PART_SUFFIX, 1)[1] for name in names]
        self.assertEqual(suffixes, [f"{index:02d}" for index in range(len(names))])

    def test_a_damaged_part_is_refused_before_the_join(self) -> None:
        """A download that is not the release's bytes stops there.

        `SHA256SUMS` lists every part, so the check runs before anything
        is joined and the message names the part: the operator knows
        which download to repeat (REL-7 D2).
        """
        damaged = self.parts()[1]
        bytes_ = bytearray(damaged.read_bytes())
        bytes_[0] ^= 0xFF
        damaged.write_bytes(bytes(bytes_))
        result = self.join()
        self.assertNotEqual(result.returncode, 0, "a damaged part was accepted")
        self.assertIn(damaged.name, result.stderr + result.stdout)
        self.assertFalse(
            (self.directory / BUNDLE).exists(),
            "the parts were joined before the damaged one was refused",
        )

    def test_a_join_of_parts_of_two_releases_is_refused(self) -> None:
        """The `.sha256` is the release's own, and the join must match it.

        Parts left behind from another release — or a part of another
        version dropped into the directory — join into a file that is not
        the one this release published, and the `.sha256` refuses it.
        """
        intruder = self.directory / f"{BUNDLE}{PART_SUFFIX}99"
        intruder.write_bytes(b"not a part of this release")
        result = self.join()
        self.assertNotEqual(result.returncode, 0, "a foreign part was accepted")
        self.assertIn(BUNDLE, result.stderr + result.stdout)

    def test_the_docs_document_the_join_the_release_produces(self) -> None:
        """One join, one place (REL-7 D2).

        The part suffix and the whole file's `.sha256` are the release's
        names, and the documented commands are the ones this test runs —
        so a change to either side that the other does not follow fails
        here.
        """
        text = OPERATIONS.read_text(encoding="utf-8")
        self.assertIn('bundle="rantai-lakehouse-$version-linux-x86_64.tar.zst"', text)
        self.assertIn('sha256sum -c "$bundle.sha256"', text)
        self.assertIn('cat "$bundle".part-* > "$bundle"', text)
        self.assertIn("sha256sum -c SHA256SUMS", text)

    def test_the_release_splits_with_the_numbering_the_docs_join(self) -> None:
        """The release's `split` and the operator's glob agree.

        `-d` numbers the parts and `-a 2` pads them, which is what makes
        the name order the byte order; without either, the operator's
        join reads them in the wrong order.
        """
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("split -b 1900M -d -a 2", text)
        self.assertIn(f"$name{PART_SUFFIX}", text)


if __name__ == "__main__":
    unittest.main()
