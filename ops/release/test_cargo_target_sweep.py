#!/usr/bin/env python3
"""Tests for `ops/dev/cargo-target-sweep.sh` (OPS-17).

The sweep is the piece of repo-owned tooling that keeps a build machine's
shared cargo target from growing without limit. It must delete old build
output inside the known subtrees, stop as soon as the size cap is reached,
never touch anything else, and never run underneath a live build.

No network and no container: every test builds a small fake target in a
temporary directory, points `SWEEP_PROC_ROOT` at a fake proc root, and runs
the script through `subprocess`. Sizes come from `du -sb`, the same measure
the script uses, so the caps are chosen from real numbers rather than from
an assumption about filesystem metadata.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "ops" / "dev" / "cargo-target-sweep.sh"

# Fixed, old timestamps: deliberately far from "now" and from one another,
# so a sort in the wrong direction cannot accidentally pass.
BASE_TIME = 1_000_000_000


class CargoTargetSweepTest(unittest.TestCase):
    """What the sweep does to a fake cargo target directory."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.target = self.root / "target"
        for relative in ("debug/deps", "debug/incremental", "debug/build", "tmp", "outside"):
            (self.target / relative).mkdir(parents=True, exist_ok=True)
        self.proc_root = self.root / "proc"
        self.proc_root.mkdir()

    def tearDown(self) -> None:
        self._directory.cleanup()

    def write_file(self, path: Path, size: int, mtime: int) -> Path:
        path.write_bytes(b"x" * size)
        os.utime(path, (mtime, mtime))
        return path

    def write_directory(self, path: Path, mtime: int) -> Path:
        path.mkdir()
        self.write_file(path / "payload.bin", 20_000, mtime)
        os.utime(path, (mtime, mtime))
        return path

    def du_size(self, path: Path) -> int:
        output = subprocess.check_output(["du", "-sb", str(path)], text=True)
        return int(output.split()[0])

    def run_sweep(self, *arguments: object) -> subprocess.CompletedProcess[str]:
        environment = dict(os.environ)
        environment["SWEEP_PROC_ROOT"] = str(self.proc_root)
        return subprocess.run(
            [str(SCRIPT), *(str(argument) for argument in arguments)],
            capture_output=True,
            text=True,
            env=environment,
        )

    def fake_running_process(self, name: str) -> Path:
        comm = self.proc_root / "4242" / "comm"
        comm.parent.mkdir(parents=True, exist_ok=True)
        comm.write_text(name + "\n", encoding="utf-8")
        return comm

    def test_the_sweep_deletes_the_oldest_entries_first_and_stops_once_under_the_cap(self) -> None:
        oldest = self.write_file(self.target / "debug/deps/old.bin", 20_000, BASE_TIME + 100)
        second = self.write_file(self.target / "tmp/tmp_old.bin", 20_000, BASE_TIME + 150)
        increment = self.write_directory(self.target / "debug/incremental/inc_old", BASE_TIME + 200)
        build = self.write_file(self.target / "debug/build/build_old.bin", 20_000, BASE_TIME + 300)
        newest = self.write_file(self.target / "debug/deps/new.bin", 20_000, BASE_TIME + 400)
        outside = self.write_file(self.target / "outside/keep.bin", 20_000, BASE_TIME)

        cap = self.du_size(self.target) - self.du_size(oldest) - self.du_size(second) + 1
        result = self.run_sweep(str(self.target), str(cap))
        output = result.stdout + result.stderr

        self.assertEqual(result.returncode, 0, output)
        self.assertFalse(oldest.exists(), "the oldest file in deps should be deleted")
        self.assertFalse(second.exists(), "the next oldest file in tmp should be deleted")
        for survivor in (increment, build, newest, outside):
            self.assertTrue(survivor.exists(), f"{survivor} should survive: {output}")
        self.assertIn("before=", output)
        self.assertIn("after=", output)
        self.assertIn("deleted=2", output)

    def test_a_dry_run_prints_the_planned_deletions_and_deletes_nothing(self) -> None:
        oldest = self.write_file(self.target / "debug/deps/old.bin", 20_000, BASE_TIME + 100)
        second = self.write_file(self.target / "tmp/tmp_old.bin", 20_000, BASE_TIME + 150)
        newer = self.write_file(self.target / "debug/deps/new.bin", 20_000, BASE_TIME + 400)

        cap = self.du_size(self.target) - self.du_size(oldest) - self.du_size(second) + 1
        result = self.run_sweep("--dry-run", str(self.target), str(cap))
        output = result.stdout + result.stderr

        self.assertEqual(result.returncode, 0, output)
        self.assertIn(f"would delete: {oldest}", output)
        self.assertIn(f"would delete: {second}", output)
        self.assertNotIn(f"would delete: {newer}", output)
        self.assertIn("would-delete=2", output)
        self.assertIn("after=", output)
        for path in (oldest, second, newer):
            self.assertTrue(path.exists(), f"{path} should still exist after a dry run")

    def test_a_running_cargo_process_makes_the_sweep_skip_with_exit_seventy_five(self) -> None:
        oldest = self.write_file(self.target / "debug/deps/old.bin", 20_000, BASE_TIME + 100)
        self.fake_running_process("cargo")

        result = self.run_sweep(str(self.target), "1")

        self.assertEqual(result.returncode, 75, result.stdout + result.stderr)
        self.assertIn("cargo", result.stdout + result.stderr)
        self.assertTrue(oldest.exists(), "nothing should be deleted under a running build")

    def test_a_running_rustc_process_makes_the_sweep_skip_with_exit_seventy_five(self) -> None:
        oldest = self.write_file(self.target / "debug/deps/old.bin", 20_000, BASE_TIME + 100)
        self.fake_running_process("rustc")

        result = self.run_sweep(str(self.target), "1")

        self.assertEqual(result.returncode, 75, result.stdout + result.stderr)
        self.assertIn("rustc", result.stdout + result.stderr)
        self.assertTrue(oldest.exists(), "nothing should be deleted under a running build")

    def test_a_symlink_inside_deps_does_not_lead_to_its_target_outside_being_deleted(self) -> None:
        victim = self.write_file(self.target / "outside/victim.bin", 20_000, BASE_TIME + 50)
        link = self.target / "debug/deps/link.bin"
        link.symlink_to(victim)
        os.utime(link, (BASE_TIME + 10, BASE_TIME + 10), follow_symlinks=False)
        real = self.write_file(self.target / "debug/deps/real.bin", 20_000, BASE_TIME + 100)

        cap = self.du_size(self.target) - self.du_size(real) + 1
        result = self.run_sweep(str(self.target), str(cap))

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(real.exists(), "the oldest real file should be deleted")
        self.assertTrue(victim.exists(), "the symlink's target outside the swept subtrees must survive")
        self.assertTrue(link.is_symlink(), "the symlink itself is not a sweep candidate")

    def test_an_invalid_cap_is_a_usage_error(self) -> None:
        self.write_file(self.target / "debug/deps/old.bin", 20_000, BASE_TIME + 100)

        for cap in ("6X", "", "1GG"):
            with self.subTest(cap=cap):
                result = self.run_sweep(str(self.target), cap)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_a_missing_target_directory_is_a_usage_error(self) -> None:
        result = self.run_sweep(str(self.root / "no-such-dir"), "10G")

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_a_cap_below_the_smallest_reachable_size_exits_two_and_prints_the_final_size(self) -> None:
        oldest = self.write_file(self.target / "debug/deps/old.bin", 20_000, BASE_TIME + 100)
        outside = self.write_file(self.target / "outside/keep.bin", 20_000, BASE_TIME)

        result = self.run_sweep(str(self.target), "1")
        output = result.stdout + result.stderr

        self.assertEqual(result.returncode, 2, output)
        self.assertIn("final size", output)
        self.assertIn(str(self.du_size(self.target)), output)
        self.assertFalse(oldest.exists(), "all allowed entries should be deleted first")
        self.assertTrue(outside.exists(), "a file outside the swept subtrees must survive")


if __name__ == "__main__":
    unittest.main()
