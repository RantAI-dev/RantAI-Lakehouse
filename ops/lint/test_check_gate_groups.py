"""Tests for `check_gate_groups.py` (OPS-25 D10).

Each case works on a throwaway git repository that holds a copy of the real
`ci.yml` and `gate_paths.txt`, with one thing broken. The scope script is the
real one, so "which group does this file select?" is answered by the code CI
runs. No network, no containers; seconds:

    python3 -m unittest discover -s ops/lint -p 'test_*.py'
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_gate_groups as check  # noqa: E402


class TemporaryRepository:
    """Copies of the two files the check reads, plus one gate file, in git."""

    def __init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="gate-groups-"))
        (self.root / ".github" / "workflows").mkdir(parents=True)
        (self.root / "scripts" / "ci").mkdir(parents=True)
        (self.root / "ops" / "g9").mkdir(parents=True)
        shutil.copy(check.CI_YML, self.ci_yml)
        shutil.copy(check.GATE_MAP, self.gate_map)
        (self.root / "ops" / "g9" / "upload_test.py").write_text("", encoding="utf-8")
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=self.root, check=True)
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True)

    @property
    def ci_yml(self) -> Path:
        return self.root / ".github" / "workflows" / "ci.yml"

    @property
    def gate_map(self) -> Path:
        return self.root / "scripts" / "ci" / "gate_paths.txt"

    def edit_ci(self, edit) -> None:
        """Load `ci.yml` as data, let `edit` change it, write it back."""
        data = yaml.safe_load(self.ci_yml.read_text(encoding="utf-8"))
        edit(data)
        self.ci_yml.write_text(yaml.dump(data), encoding="utf-8")

    def edit_map(self, edit) -> None:
        lines = self.gate_map.read_text(encoding="utf-8").splitlines()
        self.gate_map.write_text("\n".join(edit(lines)) + "\n", encoding="utf-8")

    def findings(self) -> list[str]:
        problems, _warnings = check.findings(
            self.ci_yml, self.gate_map, self.root, check.SCOPE_SCRIPT
        )
        return problems

    def cleanup(self) -> None:
        shutil.rmtree(self.root, ignore_errors=True)


class GateGroupCheckTest(unittest.TestCase):
    def repository(self) -> TemporaryRepository:
        repository = TemporaryRepository()
        self.addCleanup(repository.cleanup)
        return repository

    def test_the_real_repository_passes(self) -> None:
        problems, _warnings = check.findings(
            check.CI_YML, check.GATE_MAP, check.REPO_ROOT, check.SCOPE_SCRIPT
        )
        self.assertEqual(problems, [], "the real ci.yml, gate_paths.txt and gate files disagree")

    def test_a_job_declared_in_two_groups_is_rejected(self) -> None:
        repository = self.repository()
        repository.edit_map(lambda lines: lines + ["group pipelines g1-rustfs"])
        with self.assertRaisesRegex(ValueError, "is in groups"):
            repository.findings()

    def test_a_gate_job_in_no_group_fails(self) -> None:
        repository = self.repository()
        repository.edit_map(
            lambda lines: [
                "group pipelines g3a-dagster g3-maintenance" if line.startswith("group pipelines ") else line
                for line in lines
            ]
        )
        problems = repository.findings()
        self.assertTrue(
            any("'gold-export' in ci.yml is in no gate group" in problem for problem in problems),
            problems,
        )

    def test_a_group_with_no_path_line_fails(self) -> None:
        repository = self.repository()

        def without_governance(lines: list[str]) -> list[str]:
            kept = []
            for line in lines:
                fields = line.split()
                if len(fields) == 3 and fields[0] == "path" and "governance" in fields[2].split(","):
                    continue
                kept.append(line)
            return kept

        repository.edit_map(without_governance)
        problems = repository.findings()
        self.assertIn("gate group 'governance' has no path line", problems)

    def test_a_gate_if_naming_another_groups_output_fails(self) -> None:
        repository = self.repository()
        repository.edit_ci(
            lambda data: data["jobs"]["g4-cdc"].__setitem__(
                "if", "needs.changes.outputs.gates_storage == 'true'"
            )
        )
        problems = repository.findings()
        self.assertTrue(
            any("job 'g4-cdc' (group 'cdc')" in problem for problem in problems), problems
        )

    def test_an_unmapped_file_under_a_gate_directory_fails(self) -> None:
        repository = self.repository()
        repository.edit_map(
            lambda lines: [line for line in lines if not line.startswith("path ops/g9/* upload")]
        )
        problems = repository.findings()
        self.assertTrue(
            any("ops/g9/upload_test.py selects every gate group" in problem for problem in problems),
            problems,
        )


if __name__ == "__main__":
    unittest.main()
