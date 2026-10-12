#!/usr/bin/env python3
"""SRC-9 task 1: the connector gates' coverage registry and result table.

Two jobs, kept in one small module so the gates have one thing to import:

1. Load `ops/gates/coverage.json`, the registry of what each connector type
   is gated for (`real`, `refused` or `missing` per step). A gate reads its
   expectation from here instead of keeping a second list, and the coverage
   check of SRC-9 task 8 reads the same file.
2. Record each step's result `(type, step, outcome, detail)` and print, at
   the end of the run, one table: a row per type, a column per step. A step
   that did not run is shown as `skipped` with its reason, never blank and
   never green (feature page `connector-gates.md`, decision 3).

# Where the table goes

A gate runs inside `docker compose run`, so it cannot write the host's
`GITHUB_STEP_SUMMARY` file. `emit` therefore always prints the Markdown table
to stdout between two marker lines (`SUMMARY_BEGIN`, `SUMMARY_END`); a
workflow step after the gate extracts the lines between them from the
captured output and appends them to the job summary. When
`GITHUB_STEP_SUMMARY` IS set in the process (a gate run on the host), `emit`
appends the table to that file as well. The markers are plain text and not
`::name::` lines on purpose: GitHub reads those as workflow commands.

# Sharing

Each gate runner mounts only its own `ops/gN` directory, so this module is
mounted separately at `/opt/gates` and put on `PYTHONPATH` by the gate's
compose override (`ops/g6/docker-compose.g6.override.yml`). The module has no
third-party import, so it needs nothing installed.
"""
from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TextIO

STEPS = ("connect", "discover", "load", "reload")
EXPECTATIONS = ("real", "refused", "missing")
OUTCOMES = ("pass", "refused-as-expected", "skipped", "fail")

SUMMARY_BEGIN = "GATE-SUMMARY-BEGIN"
SUMMARY_END = "GATE-SUMMARY-END"

# Mounted by the gate's compose override; the module's own directory is where
# the file sits in a checkout (the unit tests and a host run).
_CONTAINER_REGISTRY = Path("/opt/gates/coverage.json")
_LOCAL_REGISTRY = Path(__file__).resolve().parent / "coverage.json"


class RegistryError(ValueError):
    """The registry file is malformed. A gate must not run against a registry
    it cannot trust, so this is raised, never papered over."""


class Registry:
    """The parsed, validated `coverage.json`."""

    def __init__(self, types: dict[str, dict]) -> None:
        self._types = types

    @classmethod
    def load(cls, path: Path | str | None = None) -> "Registry":
        """Read and validate the registry.

        `path` defaults to `$GATE_COVERAGE_PATH`, then the compose mount, then
        the file beside this module.

        # Errors

        Raises `RegistryError` if the file is missing or malformed.
        """
        if path is None:
            env = os.environ.get("GATE_COVERAGE_PATH")
            path = Path(env) if env else (_CONTAINER_REGISTRY if _CONTAINER_REGISTRY.exists() else _LOCAL_REGISTRY)
        try:
            raw = json.loads(Path(path).read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise RegistryError(f"cannot read the coverage registry {path}: {exc}") from exc
        types = raw.get("types")
        if not isinstance(types, dict) or not types:
            raise RegistryError("the coverage registry has no 'types' object")
        for name, entry in types.items():
            gate = entry.get("gate")
            if gate is not None and not isinstance(gate, str):
                raise RegistryError(f"{name}: 'gate' must be a string or null")
            steps = entry.get("steps")
            if not isinstance(steps, dict) or set(steps) != set(STEPS):
                raise RegistryError(f"{name}: 'steps' must have exactly {', '.join(STEPS)}")
            for step, value in steps.items():
                if value not in EXPECTATIONS:
                    raise RegistryError(f"{name}.{step}: {value!r} is not one of {', '.join(EXPECTATIONS)}")
                if value == "refused" and not (entry.get("notes") or {}).get(step):
                    raise RegistryError(f"{name}.{step}: a 'refused' step needs a note naming the backlog item")
            if gate is None and any(v != "missing" for v in steps.values()):
                raise RegistryError(f"{name}: a type with no gate cannot have a step other than 'missing'")
        return cls(types)

    def type_names(self) -> list[str]:
        return list(self._types)

    def gate_of(self, type_name: str) -> str | None:
        return self._entry(type_name)["gate"]

    def expectation(self, type_name: str, step: str) -> str:
        """`real`, `refused` or `missing` for one step of one type."""
        _check_step(step)
        return self._entry(type_name)["steps"][step]

    def note(self, type_name: str, step: str) -> str:
        """The note for a step, falling back to the type's `all` note; empty if none."""
        _check_step(step)
        notes = self._entry(type_name).get("notes") or {}
        return notes.get(step) or notes.get("all") or ""

    def _entry(self, type_name: str) -> dict:
        try:
            return self._types[type_name]
        except KeyError:
            raise KeyError(f"{type_name!r} is not in the coverage registry") from None


def _check_step(step: str) -> None:
    if step not in STEPS:
        raise ValueError(f"unknown step {step!r}; expected one of {', '.join(STEPS)}")


@dataclass(frozen=True)
class StepResult:
    outcome: str
    detail: str


class Recorder:
    """Collects step results for one gate run and renders them.

    `gate` is the gate's name in the registry (`g6`). A cell nobody recorded
    is rendered `skipped`, with the reason that fits: the registry says no
    gate asserts it yet, another gate owns it, or this gate stopped first.
    """

    def __init__(self, registry: Registry, gate: str) -> None:
        self.registry = registry
        self.gate = gate
        self._results: dict[tuple[str, str], StepResult] = {}

    def record(self, type_name: str, step: str, outcome: str, detail: str = "") -> None:
        """Record one step's result. A second record for the same cell
        replaces the first, so a retried step shows its last attempt.

        # Errors

        Raises `KeyError` for a type the registry does not list, and
        `ValueError` for an unknown step or outcome: a typo in a gate must
        not become a silently empty cell.
        """
        self.registry.gate_of(type_name)  # raises KeyError for an unknown type
        _check_step(step)
        if outcome not in OUTCOMES:
            raise ValueError(f"unknown outcome {outcome!r}; expected one of {', '.join(OUTCOMES)}")
        self._results[(type_name, step)] = StepResult(outcome, detail)

    def failures(self) -> list[tuple[str, str, str]]:
        """`(type, step, detail)` of every cell recorded as `fail`, in registry order."""
        found = []
        for type_name in self.registry.type_names():
            for step in STEPS:
                result = self._results.get((type_name, step))
                if result is not None and result.outcome == "fail":
                    found.append((type_name, step, result.detail))
        return found

    def cell(self, type_name: str, step: str) -> StepResult:
        """The recorded result, or the `skipped` result that stands in for one."""
        recorded = self._results.get((type_name, step))
        if recorded is not None:
            return recorded
        if self.registry.expectation(type_name, step) == "missing":
            note = self.registry.note(type_name, step)
            return StepResult("skipped", "no gate asserts this step yet" + (f" ({note})" if note else ""))
        owner = self.registry.gate_of(type_name)
        if owner != self.gate:
            return StepResult("skipped", f"covered by gate {owner}, which did not run here")
        return StepResult("skipped", f"gate {self.gate} stopped before this step ran")

    def render_text(self) -> str:
        """Plain-text table for the job log."""
        header = ["connector type", "gate", *STEPS]
        rows = [header]
        for type_name in self.registry.type_names():
            rows.append(
                [type_name, self.registry.gate_of(type_name) or "-"]
                + [self.cell(type_name, step).outcome for step in STEPS]
            )
        widths = [max(len(row[i]) for row in rows) for i in range(len(header))]
        lines = ["  ".join(value.ljust(widths[i]) for i, value in enumerate(row)).rstrip() for row in rows]
        lines.insert(1, "  ".join("-" * w for w in widths))
        return "\n".join(lines) + "\n" + self._details_text()

    def render_markdown(self) -> str:
        """Markdown table for the job summary, with the details of every
        cell that is not a plain pass underneath."""
        lines = [
            "### Connector gates: coverage and results",
            "",
            "| Connector type | Gate | " + " | ".join(STEPS) + " |",
            "| --- | --- | " + " | ".join("---" for _ in STEPS) + " |",
        ]
        for type_name in self.registry.type_names():
            cells = []
            for step in STEPS:
                outcome = self.cell(type_name, step).outcome
                cells.append("**FAIL**" if outcome == "fail" else outcome)
            lines.append(f"| {type_name} | {self.registry.gate_of(type_name) or '-'} | " + " | ".join(cells) + " |")
        details = self._detail_lines()
        if details:
            lines += ["", "Details of every step that is not a plain pass:", ""]
            lines += [f"- {type_name}, {step}: {outcome}. {detail}".rstrip() for type_name, step, outcome, detail in details]
        return "\n".join(lines) + "\n"

    def emit(self, stream: TextIO | None = None) -> None:
        """Print the text table, then the Markdown between the two marker
        lines; append the Markdown to `$GITHUB_STEP_SUMMARY` when it is set.

        The file write is best effort: a gate that passed or failed on its
        own merits must not change its verdict because the summary file could
        not be written, and the stdout copy is the one CI extracts.
        """
        out = stream if stream is not None else sys.stdout
        markdown = self.render_markdown()
        print(self.render_text(), file=out)
        print(SUMMARY_BEGIN, file=out)
        print(markdown, file=out, end="")
        print(SUMMARY_END, file=out, flush=True)
        path = os.environ.get("GITHUB_STEP_SUMMARY")
        if path:
            try:
                with open(path, "a", encoding="utf-8") as fh:
                    fh.write(markdown + "\n")
            except OSError as exc:
                print(f"gate_summary: could not append to GITHUB_STEP_SUMMARY: {exc}", file=sys.stderr)

    def _detail_lines(self) -> list[tuple[str, str, str, str]]:
        lines = []
        for type_name in self.registry.type_names():
            for step in STEPS:
                result = self.cell(type_name, step)
                if result.outcome != "pass":
                    lines.append((type_name, step, result.outcome, result.detail))
        return lines

    def _details_text(self) -> str:
        details = self._detail_lines()
        if not details:
            return ""
        return "\n" + "\n".join(f"  {t} / {s}: {o}. {d}".rstrip() for t, s, o, d in details) + "\n"


def extract_summary(log_text: str) -> str:
    """The Markdown between the marker lines of a captured gate log, or an
    empty string if the gate never reached `emit`. Used by the workflow step
    that appends it to the job summary (the same rule, in shell, is in
    `.github/workflows/ci.yml`); kept here so it is tested once."""
    captured: list[str] = []
    inside = False
    for line in log_text.splitlines():
        stripped = line.strip()
        if stripped == SUMMARY_BEGIN:
            inside, captured = True, []
        elif stripped == SUMMARY_END and inside:
            inside = False
        elif inside:
            captured.append(line)
    return "\n".join(captured) + ("\n" if captured else "")
