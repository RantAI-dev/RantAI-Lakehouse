"""Unit tests for `gate_summary` (SRC-9 task 1). No network, no docker.

Run from the repository root:

    python3 -m unittest discover -s ops/gates -p 'test_*.py'
"""
from __future__ import annotations

import io
import json
import os
import tempfile
import unittest
from pathlib import Path

import gate_summary as gs

REAL_REGISTRY = Path(__file__).resolve().parent / "coverage.json"


def small_registry() -> gs.Registry:
    steps_real = {s: "real" for s in gs.STEPS}
    return gs.Registry(
        {
            "Alpha": {"gate": "g6", "steps": steps_real},
            "Beta": {
                "gate": "g6",
                "steps": {"connect": "refused", "discover": "refused", "load": "real", "reload": "real"},
                "notes": {"connect": "SRC-5", "discover": "SRC-13"},
            },
            "Gamma": {"gate": None, "steps": {s: "missing" for s in gs.STEPS}, "notes": {"all": "task 3"}},
            "Delta": {"gate": "g4", "steps": steps_real},
        }
    )


class RegistryFileTests(unittest.TestCase):
    def test_the_checked_in_registry_loads_and_lists_thirteen_types(self) -> None:
        registry = gs.Registry.load(REAL_REGISTRY)
        self.assertEqual(len(registry.type_names()), 13)

    def test_the_checked_in_registry_does_not_list_google_sheets(self) -> None:
        # 0062 flipped it to unsupported; a gate for it would claim coverage of nothing.
        self.assertNotIn("Google Sheets", gs.Registry.load(REAL_REGISTRY).type_names())

    def test_a_registry_with_a_bad_step_value_is_refused(self) -> None:
        raw = json.loads(REAL_REGISTRY.read_text())
        raw["types"]["MySQL"]["steps"]["load"] = "maybe"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "c.json"
            path.write_text(json.dumps(raw))
            with self.assertRaises(gs.RegistryError):
                gs.Registry.load(path)

    def test_a_refused_step_without_a_note_is_refused(self) -> None:
        raw = json.loads(REAL_REGISTRY.read_text())
        raw["types"]["Kafka"]["notes"].pop("connect")
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "c.json"
            path.write_text(json.dumps(raw))
            with self.assertRaises(gs.RegistryError):
                gs.Registry.load(path)

    def test_a_type_with_no_gate_cannot_claim_a_real_step(self) -> None:
        raw = json.loads(REAL_REGISTRY.read_text())
        raw["types"]["Oracle"]["steps"]["load"] = "real"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "c.json"
            path.write_text(json.dumps(raw))
            with self.assertRaises(gs.RegistryError):
                gs.Registry.load(path)

    def test_a_missing_registry_file_is_an_error_not_an_empty_registry(self) -> None:
        with self.assertRaises(gs.RegistryError):
            gs.Registry.load("/nonexistent/coverage.json")

    def test_every_step_of_every_type_in_the_checked_in_registry_is_known(self) -> None:
        registry = gs.Registry.load(REAL_REGISTRY)
        for name in registry.type_names():
            for step in gs.STEPS:
                self.assertIn(registry.expectation(name, step), gs.EXPECTATIONS)

    def test_the_gate_registry_path_can_be_set_in_the_environment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "c.json"
            path.write_text(REAL_REGISTRY.read_text())
            old = os.environ.get("GATE_COVERAGE_PATH")
            os.environ["GATE_COVERAGE_PATH"] = str(path)
            try:
                self.assertEqual(len(gs.Registry.load().type_names()), 13)
            finally:
                if old is None:
                    del os.environ["GATE_COVERAGE_PATH"]
                else:
                    os.environ["GATE_COVERAGE_PATH"] = old


class RecorderTests(unittest.TestCase):
    def test_recording_an_unknown_type_raises(self) -> None:
        with self.assertRaises(KeyError):
            gs.Recorder(small_registry(), "g6").record("Nope", "load", "pass")

    def test_recording_an_unknown_step_raises(self) -> None:
        with self.assertRaises(ValueError):
            gs.Recorder(small_registry(), "g6").record("Alpha", "sync", "pass")

    def test_recording_an_unknown_outcome_raises(self) -> None:
        with self.assertRaises(ValueError):
            gs.Recorder(small_registry(), "g6").record("Alpha", "load", "green")

    def test_a_cell_nobody_recorded_is_skipped_with_a_reason_and_never_blank(self) -> None:
        recorder = gs.Recorder(small_registry(), "g6")
        for type_name in recorder.registry.type_names():
            for step in gs.STEPS:
                cell = recorder.cell(type_name, step)
                self.assertEqual(cell.outcome, "skipped")
                self.assertTrue(cell.detail)

    def test_an_unrecorded_cell_of_a_type_with_no_gate_says_no_gate_asserts_it(self) -> None:
        cell = gs.Recorder(small_registry(), "g6").cell("Gamma", "load")
        self.assertIn("no gate asserts this step yet", cell.detail)
        self.assertIn("task 3", cell.detail)

    def test_an_unrecorded_cell_of_another_gates_type_names_that_gate(self) -> None:
        cell = gs.Recorder(small_registry(), "g6").cell("Delta", "load")
        self.assertIn("g4", cell.detail)

    def test_an_unrecorded_cell_of_this_gates_type_says_the_gate_stopped_first(self) -> None:
        cell = gs.Recorder(small_registry(), "g6").cell("Alpha", "reload")
        self.assertIn("stopped before", cell.detail)

    def test_a_second_record_for_the_same_cell_replaces_the_first(self) -> None:
        recorder = gs.Recorder(small_registry(), "g6")
        recorder.record("Alpha", "load", "fail", "first")
        recorder.record("Alpha", "load", "pass")
        self.assertEqual(recorder.cell("Alpha", "load").outcome, "pass")
        self.assertEqual(recorder.failures(), [])

    def test_failures_lists_the_type_step_and_detail_in_registry_order(self) -> None:
        recorder = gs.Recorder(small_registry(), "g6")
        recorder.record("Beta", "load", "fail", "no rows")
        recorder.record("Alpha", "reload", "fail", "doubled")
        self.assertEqual(recorder.failures(), [("Alpha", "reload", "doubled"), ("Beta", "load", "no rows")])


class RenderTests(unittest.TestCase):
    def _recorder(self) -> gs.Recorder:
        recorder = gs.Recorder(small_registry(), "g6")
        for step in gs.STEPS:
            recorder.record("Alpha", step, "pass")
        recorder.record("Beta", "connect", "refused-as-expected", "supported:false")
        recorder.record("Beta", "load", "fail", "got 0 rows")
        return recorder

    def test_the_markdown_table_has_a_row_per_type_and_a_column_per_step(self) -> None:
        lines = self._recorder().render_markdown().splitlines()
        table = [line for line in lines if line.startswith("|")]
        self.assertEqual(len(table), 2 + 4)
        self.assertEqual(table[0], "| Connector type | Gate | connect | discover | load | reload |")
        self.assertEqual(table[2], "| Alpha | g6 | pass | pass | pass | pass |")

    def test_a_failed_cell_is_marked_in_bold_in_the_markdown(self) -> None:
        self.assertIn("**FAIL**", self._recorder().render_markdown())

    def test_the_markdown_lists_the_detail_of_every_cell_that_is_not_a_plain_pass(self) -> None:
        markdown = self._recorder().render_markdown()
        self.assertIn("- Beta, load: fail. got 0 rows", markdown)
        self.assertIn("- Gamma, connect: skipped. no gate asserts this step yet", markdown)
        self.assertNotIn("- Alpha,", markdown)

    def test_no_cell_of_either_table_is_empty(self) -> None:
        recorder = self._recorder()
        for line in recorder.render_markdown().splitlines():
            if line.startswith("| ") and not line.startswith("| ---"):
                self.assertTrue(all(cell.strip() for cell in line.strip("|").split("|")), line)

    def test_the_text_table_names_every_type_and_step_outcome(self) -> None:
        text = self._recorder().render_text()
        for needle in ("Alpha", "Beta", "Gamma", "Delta", "refused-as-expected", "fail", "skipped"):
            self.assertIn(needle, text)

    def test_emit_prints_the_markdown_between_the_two_marker_lines(self) -> None:
        recorder = self._recorder()
        buffer = io.StringIO()
        recorder.emit(buffer)
        self.assertEqual(gs.extract_summary(buffer.getvalue()), recorder.render_markdown())

    def test_emit_appends_to_the_step_summary_file_when_the_variable_is_set(self) -> None:
        recorder = self._recorder()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "summary.md"
            path.write_text("existing\n")
            old = os.environ.get("GITHUB_STEP_SUMMARY")
            os.environ["GITHUB_STEP_SUMMARY"] = str(path)
            try:
                recorder.emit(io.StringIO())
            finally:
                if old is None:
                    del os.environ["GITHUB_STEP_SUMMARY"]
                else:
                    os.environ["GITHUB_STEP_SUMMARY"] = old
            content = path.read_text()
        self.assertTrue(content.startswith("existing\n"))
        self.assertIn("| Alpha | g6 | pass | pass | pass | pass |", content)

    def test_emit_does_not_raise_when_the_step_summary_file_cannot_be_written(self) -> None:
        old = os.environ.get("GITHUB_STEP_SUMMARY")
        os.environ["GITHUB_STEP_SUMMARY"] = "/nonexistent-dir/summary.md"
        try:
            self._recorder().emit(io.StringIO())
        finally:
            if old is None:
                del os.environ["GITHUB_STEP_SUMMARY"]
            else:
                os.environ["GITHUB_STEP_SUMMARY"] = old

    def test_extract_summary_returns_nothing_when_the_gate_never_printed_it(self) -> None:
        self.assertEqual(gs.extract_summary("[g6] logged in\n[g6] FAILED: x\n"), "")

    def test_extract_summary_ignores_lines_outside_the_markers(self) -> None:
        log = f"before\n{gs.SUMMARY_BEGIN}\n| a |\n{gs.SUMMARY_END}\nafter\n"
        self.assertEqual(gs.extract_summary(log), "| a |\n")


if __name__ == "__main__":
    unittest.main()
