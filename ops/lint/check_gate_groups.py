#!/usr/bin/env python3
"""Check the gate-group wiring: one group per gate job, one path rule per group.

# Why this exists

`OPS-25` makes a pull request run only the acceptance gates its changed files
can affect. Three files have to agree for that to be safe, and each of them
fails quietly on its own:

- `scripts/ci/gate_paths.txt` declares the nine groups, which job belongs to
  which group, and which paths select which group;
- `.github/workflows/ci.yml` puts each gate job's `if:` behind its own
  `gates_<group>` output and passes the selection to `ci-required`;
- `scripts/ci/required_gate.sh` decides from that selection whether a skipped
  gate job was allowed to be skipped.

A gate job missing from a group, an `if:` naming another group's output, or a
`changes` output that is never declared means a gate that should have run is
skipped, and `ci-required` reads the skip as allowed. `OPS-25` D10 makes this
one check so a misplaced line cannot ship.

# What it does

Reads `ci.yml`, `gate_paths.txt` and `git ls-files`, and prints one line per
finding, exiting 1 if there is any:

(a) every job in `ci.yml` other than the fixed non-gate set (`changes`,
    `repo-lints`, the Rust and Dagster unit jobs, `images`, `image-smoke`,
    `ci-required`) is in exactly one group, and every job a `group` line names
    exists in `ci.yml`;
(b) each gate job's `if:` names `needs.changes.outputs.gates_<its group>` and
    no other `gates_` output;
(c) `changes.outputs` declares `gates_<group>` for every group and
    `gate_groups`, and the `ci-required` step that runs `required_gate.sh`
    passes `SELECTED_GATE_GROUPS`;
(d) every group is named by at least one `path` line of its own (an `all` line
    does not count: it selects the group only when every group runs);
(e) every file `git ls-files` reports under the gate directories classifies,
    through the real `detect_change_scope.sh --classify`, to a group list --
    not `all` (shared), not `none`, not `unmapped`.

A `path` line that matches no tracked file is printed as a warning, not a
failure: a rule for a directory that is created later is legitimate, and the
map is what it is.

# What it does not cover

- Whether the group a job is in is the *right* group for what the gate runs:
  that is a judgement about the gate, made once in the plan (D1) and reviewed
  in the diff, not a property a script can read.
- Whether a changed file that is *not* under the gate directories is mapped
  sensibly. `changes` fails closed on an unmapped file (every group), so a
  missing rule can only run more gates than needed, never fewer.
- Path patterns with a `?` or `[...]` class: the map uses globs, bash reads
  them, and this check does not re-implement bash's matcher. `*` (which
  matches `/` too) is what the map uses.
"""

from __future__ import annotations

import fnmatch
import os
import re
import subprocess
import sys
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
CI_YML = REPO_ROOT / ".github" / "workflows" / "ci.yml"
GATE_MAP = REPO_ROOT / "scripts" / "ci" / "gate_paths.txt"
SCOPE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "detect_change_scope.sh"

# D10 (a): the jobs that are not gates. They either have no `if:` (the scope
# and the lints), carry their own scope-only `if:` (the Rust and Dagster unit
# jobs, `images`, `image-smoke`), or are the gate itself.
NON_GATE_JOBS = frozenset(
    {
        "changes",
        "repo-lints",
        "verify",
        "fmt",
        "clippy",
        "build",
        "test",
        "msrv",
        "dagster-unit-tests",
        "dagster-loader-tests",
        "images",
        "image-smoke",
        "ci-required",
    }
)

# D10 (e): the directories whose files a gate mounts, imports or runs.
GATE_DIRS = (
    "ops/g2",
    "ops/g3",
    "ops/g3a",
    "ops/g4",
    "ops/g6",
    "ops/g7",
    "ops/g8",
    "ops/g9",
    "ops/g10",
    "ops/gold_export",
    "ops/sso",
    "ops/seaweedfs",
)

GATE_OUTPUT_RE = re.compile(r"needs\.changes\.outputs\.gates_([a-z]+)")
ANY_GATE_NAME_RE = re.compile(r"gates_([a-z]+)")
GATE_GROUPS_VALUE_RE = re.compile(r"^,([a-z]+,)+$")


class GateMap:
    """The parsed `gate_paths.txt`."""

    def __init__(self, groups: list[str], members: dict[str, str], paths: list[tuple[str, str]]) -> None:
        self.groups = groups
        self.members = members
        self.paths = paths


def read_gate_map(path: Path) -> GateMap:
    """Parse the map; raise `ValueError` on a line this check cannot read."""
    groups: list[str] = []
    members: dict[str, str] = {}
    paths: list[tuple[str, str]] = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        keyword = fields[0]
        if keyword == "group":
            if len(fields) < 3:
                raise ValueError(f"gate_paths.txt:{number}: a group line needs a name and a job")
            name = fields[1]
            if name not in groups:
                groups.append(name)
            for job in fields[2:]:
                previous = members.get(job)
                if previous is not None and previous != name:
                    raise ValueError(
                        f"gate_paths.txt:{number}: job '{job}' is in groups '{previous}' and '{name}'"
                    )
                members[job] = name
        elif keyword == "path":
            if len(fields) != 3:
                raise ValueError(f"gate_paths.txt:{number}: a path line needs a pattern and a group list")
            paths.append((fields[1], fields[2]))
        else:
            raise ValueError(f"gate_paths.txt:{number}: expected a group or path line, got '{line}'")
    return GateMap(groups, members, paths)


def tracked_files(repo_root: Path) -> list[str]:
    """`git ls-files`, relative to `repo_root`."""
    completed = subprocess.run(
        ["git", "ls-files"], cwd=repo_root, capture_output=True, text=True, check=False
    )
    if completed.returncode != 0:
        raise RuntimeError(f"git ls-files failed: {completed.stderr.strip()}")
    return [line for line in completed.stdout.splitlines() if line]


def classify(script: Path, map_path: Path, repo_root: Path, paths: list[str]) -> dict[str, str]:
    """Ask the real scope script what each path selects."""
    environment = {
        "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"),
        "GATE_PATHS_FILE": str(map_path),
    }
    completed = subprocess.run(
        ["bash", str(script), "--classify", *paths],
        cwd=repo_root,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"detect_change_scope.sh --classify exited {completed.returncode}: "
            f"{completed.stdout.strip()} {completed.stderr.strip()}".strip()
        )
    answer: dict[str, str] = {}
    for line in completed.stdout.splitlines():
        path, _, groups = line.partition("\t")
        if not _:
            raise RuntimeError(f"unexpected --classify line: {line!r}")
        answer[path] = groups
    return answer


def findings(
    ci_path: Path, map_path: Path, repo_root: Path, script: Path
) -> tuple[list[str], list[str]]:
    """Return (findings, warnings). An empty finding list is a pass."""
    problems: list[str] = []
    warnings: list[str] = []

    gate_map = read_gate_map(map_path)
    ci = yaml.safe_load(ci_path.read_text(encoding="utf-8"))
    jobs = ci.get("jobs", {})
    files = tracked_files(repo_root)

    # (a) one group per gate job, and every named job exists.
    for job in jobs:
        if job in NON_GATE_JOBS:
            continue
        group = gate_map.members.get(job)
        if group is None:
            problems.append(f"job '{job}' in ci.yml is in no gate group of gate_paths.txt")
        elif group not in gate_map.groups:
            problems.append(f"job '{job}' is in group '{group}', which no group line declares")
    for job in sorted(gate_map.members):
        if job not in jobs:
            problems.append(f"gate_paths.txt names job '{job}', which is not a job of ci.yml")

    # (b) each gate's `if:` names exactly its own group's output.
    for job, group in sorted(gate_map.members.items()):
        if job not in jobs:
            continue
        condition = str(jobs[job].get("if", ""))
        named = GATE_OUTPUT_RE.findall(condition)
        if named != [group]:
            problems.append(
                f"job '{job}' (group '{group}') has if: naming {named or 'no gates_ output'}; "
                f"it must name needs.changes.outputs.gates_{group} and no other"
            )

    # (c) the outputs `changes` declares and the selection `ci-required` reads.
    outputs = jobs.get("changes", {}).get("outputs", {}) or {}
    for group in gate_map.groups:
        if f"gates_{group}" not in outputs:
            problems.append(f"changes.outputs does not declare gates_{group}")
    if "gate_groups" not in outputs:
        problems.append("changes.outputs does not declare gate_groups")
    gate_step = None
    for step in jobs.get("ci-required", {}).get("steps", []) or []:
        # `scripts/ci/required_gate.sh` and not the self-test
        # (`scripts/ci/tests/test_required_gate.sh`).
        if "scripts/ci/required_gate.sh" in str(step.get("run", "")):
            gate_step = step
            break
    if gate_step is None:
        problems.append("ci-required has no step that runs required_gate.sh")
    elif "SELECTED_GATE_GROUPS" not in (gate_step.get("env") or {}):
        problems.append("the ci-required step that runs required_gate.sh does not pass SELECTED_GATE_GROUPS")

    # (d) every group is reachable from at least one path line that names it.
    # An `all` line is not enough: it selects the group only when every group
    # runs, which is the case the map exists to avoid.
    for group in gate_map.groups:
        if not any(group in groups.split(",") for _, groups in gate_map.paths):
            problems.append(f"gate group '{group}' has no path line")

    # A path line that matches no tracked file is a warning, not a failure.
    for pattern, _groups in gate_map.paths:
        if not any(fnmatch.fnmatchcase(name, pattern) for name in files):
            warnings.append(f"path pattern '{pattern}' matches no tracked file")

    # (e) every gate file classifies to a group list, through the real script.
    gate_files = sorted(
        name for name in files if any(name.startswith(f"{directory}/") for directory in GATE_DIRS)
    )
    if gate_files:
        if not script.is_file():
            problems.append(f"the scope script is missing: {script}")
        else:
            answer = classify(script, map_path, repo_root, gate_files)
            for name in gate_files:
                groups = answer.get(name)
                if groups is None:
                    problems.append(f"{name} was not classified by detect_change_scope.sh --classify")
                elif groups == "all":
                    problems.append(
                        f"{name} selects every gate group (a shared path rule caught it); "
                        "add a narrower path line"
                    )
                elif groups == "none":
                    problems.append(
                        f"{name} selects no gate group; a gate file must select the gate it runs"
                    )
                elif groups == "unmapped" or groups == "docs":
                    problems.append(
                        f"{name} is not in gate_paths.txt; add the path line that selects its gate"
                    )
                elif not GATE_GROUPS_VALUE_RE.match(f",{groups},"):
                    problems.append(f"{name} classified to an unreadable group list: {groups!r}")
                else:
                    for group in groups.split(","):
                        if group not in gate_map.groups:
                            problems.append(f"{name} classified to unknown group '{group}'")
    return problems, warnings


def main() -> int:
    try:
        problems, warnings = findings(CI_YML, GATE_MAP, REPO_ROOT, SCOPE_SCRIPT)
    except (OSError, RuntimeError, ValueError, yaml.YAMLError) as error:
        print(f"FAIL: {error}")
        return 2
    for warning in warnings:
        print(f"warning: {warning}")
    for problem in problems:
        print(f"FAIL: {problem}")
    if problems:
        print(f"{len(problems)} gate-group finding(s)")
        return 1
    print("gate groups are consistent: every gate job has one group, its if: names it, and every gate file maps to a group")
    return 0


if __name__ == "__main__":
    sys.exit(main())
