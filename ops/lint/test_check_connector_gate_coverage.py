"""Tests for `check_connector_gate_coverage.py` (SRC-9 plan task 8).

Fixture migrations and registries are written to a temporary directory; no
network, no database. Run from the repository root:

    python3 -m unittest discover -s ops/lint -p 'test_*.py'
"""
from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_connector_gate_coverage as check  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]

CREATE = "CREATE TABLE connector_type (name TEXT PRIMARY KEY, adapter TEXT, supported BOOLEAN NOT NULL DEFAULT false, docs_url TEXT);\n"
INSERT = """
-- a comment; with a semicolon
INSERT INTO connector_type (name, adapter, supported, docs_url)
VALUES
    ('Alpha', 'sql', true, NULL),
    ('Beta', NULL, false, NULL),
    ('Gamma -- not a comment', 'rest', true, NULL)
ON CONFLICT DO NOTHING;
"""
STEPS_REAL = {"connect": "real", "discover": "real", "load": "real", "reload": "real"}


def migrations(*files: str) -> Path:
    tmp = Path(tempfile.mkdtemp(prefix="mig"))
    for i, text in enumerate(files):
        (tmp / f"{i + 1:04d}_x.sql").write_text(text, encoding="utf-8")
    return tmp


def registry_file(types: dict) -> Path:
    path = Path(tempfile.mkdtemp(prefix="reg")) / "coverage.json"
    path.write_text(json.dumps({"types": types}), encoding="utf-8")
    return path


def entry(gate="g6", steps=None, waiver=None) -> dict:
    out = {"gate": gate, "steps": dict(steps or STEPS_REAL)}
    if waiver:
        out["waiver"] = waiver
    return out


class SupportedTypesTests(unittest.TestCase):
    def test_the_real_migrations_list_twenty_three_supported_types_and_not_google_sheets(self) -> None:
        names = check.supported_types(REPO_ROOT / "rust" / "migrations")
        # 13 before SRC-9 decision 8 (0066); 11 after it; +8 saas types in 0096 (SRC-3 T7.1);
        # +2 in 0117 (SRC-4 slice 3: MySQL CDC and SQL Server CDC are supported again).
        # +1 in 0119 (SRC-4 slice 5: MongoDB CDC is a new supported type), +1 in 0120 (slice 6: Oracle CDC).
        self.assertEqual(len(names), 23)
        self.assertIn("MongoDB CDC", names)
        self.assertIn("Oracle CDC", names)
        self.assertIn("MySQL CDC", names)
        self.assertIn("SQL Server CDC", names)
        self.assertNotIn("Google Sheets", names)
        self.assertIn("Oracle", names)
        self.assertIn("Salesforce", names)
        self.assertNotIn("Workday Reports", names)  # SRC-3 decision 10: waits for a real account
        self.assertNotIn("NetSuite", names)

    def test_inserted_rows_are_supported_by_their_flag_and_a_dash_dash_inside_a_string_is_text(self) -> None:
        names = check.supported_types(migrations(CREATE + INSERT))
        self.assertEqual(names, ["Alpha", "Gamma -- not a comment"])

    def test_an_update_flips_a_row_only_when_its_where_conditions_hold(self) -> None:
        flip = """
        UPDATE connector_type SET supported = true, adapter = 'kafka'
        WHERE name = 'Beta' AND adapter IS NULL AND supported = false;
        UPDATE connector_type SET supported = false, unsupported_reason = 'why; not'
        WHERE name = 'Alpha' AND supported = true AND adapter = 'sql';
        """
        names = check.supported_types(migrations(CREATE + INSERT, flip))
        self.assertEqual(names, ["Beta", "Gamma -- not a comment"])

    def test_an_update_whose_adapter_condition_does_not_hold_changes_nothing(self) -> None:
        names = check.supported_types(migrations(
            CREATE + INSERT,
            "UPDATE connector_type SET supported = false WHERE name = 'Alpha' AND adapter = 'files';",
        ))
        self.assertIn("Alpha", names)

    def test_an_update_naming_the_reason_it_replaces_applies_only_when_that_text_is_current(self) -> None:
        down = "UPDATE connector_type SET supported = false, unsupported_reason = 'not yet' WHERE name = 'Alpha' AND supported = true;"
        up = (
            "UPDATE connector_type SET supported = true, unsupported_reason = NULL "
            "WHERE name = 'Alpha' AND supported = false AND unsupported_reason = '{}';"
        )
        self.assertIn("Alpha", check.supported_types(migrations(CREATE + INSERT, down, up.format("not yet"))))
        # The row was edited since: its reason is no longer the one named, so nothing changes.
        self.assertNotIn("Alpha", check.supported_types(migrations(CREATE + INSERT, down, up.format("something else"))))

    def test_an_update_of_a_row_that_does_not_exist_changes_nothing(self) -> None:
        names = check.supported_types(migrations(CREATE + INSERT, "UPDATE connector_type SET supported = true WHERE name = 'Nope';"))
        self.assertNotIn("Nope", names)

    def test_a_later_insert_with_on_conflict_do_nothing_keeps_the_existing_row(self) -> None:
        again = "INSERT INTO connector_type (name, adapter, supported, docs_url) VALUES ('Alpha', NULL, false, NULL) ON CONFLICT DO NOTHING;"
        self.assertIn("Alpha", check.supported_types(migrations(CREATE + INSERT, again)))

    def test_an_insert_of_an_existing_row_without_on_conflict_is_refused(self) -> None:
        again = "INSERT INTO connector_type (name, adapter, supported, docs_url) VALUES ('Alpha', NULL, false, NULL);"
        with self.assertRaises(check.MigrationShapeError):
            check.supported_types(migrations(CREATE + INSERT, again))

    def test_alter_table_that_adds_a_column_or_a_constraint_is_understood(self) -> None:
        alter = (
            "ALTER TABLE connector_type ADD COLUMN IF NOT EXISTS unsupported_reason TEXT;\n"
            "ALTER TABLE connector_type DROP CONSTRAINT connector_type_adapter_check;\n"
            "ALTER TABLE connector_type ADD CONSTRAINT connector_type_adapter_check CHECK (adapter IS NULL OR adapter IN ('sql'));\n"
        )
        self.assertEqual(check.supported_types(migrations(CREATE + INSERT, alter)), ["Alpha", "Gamma -- not a comment"])

    def test_a_statement_shape_it_does_not_know_fails_and_names_the_file(self) -> None:
        for sql in (
            "DELETE FROM connector_type WHERE name = 'Alpha';",
            "ALTER TABLE connector_type DROP COLUMN supported;",
            "UPDATE connector_type SET docs_url = 'x' WHERE name = 'Alpha';",
            "UPDATE connector_type SET supported = false WHERE adapter = 'sql';",
            "UPDATE connector_type SET supported = false WHERE name = 'Alpha' OR name = 'Beta';",
            "UPDATE connector_type SET supported = maybe WHERE name = 'Alpha';",
            "INSERT INTO connector_type (name, supported) VALUES ('Z', true);",
            "INSERT INTO connector_type (name, adapter, supported, docs_url) SELECT 'Z', NULL, true, NULL;",
            "TRUNCATE connector_type;",
        ):
            with self.subTest(sql=sql), self.assertRaises(check.MigrationShapeError) as raised:
                check.supported_types(migrations(CREATE + INSERT, sql))
            self.assertIn("0002_x.sql", str(raised.exception))

    def test_a_dollar_quoted_block_that_mentions_the_table_fails(self) -> None:
        block = "DO $$ BEGIN UPDATE connector_type SET supported = false WHERE name = 'Alpha'; END $$;"
        with self.assertRaises(check.MigrationShapeError):
            check.supported_types(migrations(CREATE + INSERT, block))

    def test_a_migration_that_only_mentions_the_table_in_a_comment_or_a_longer_name_is_ignored(self) -> None:
        other = "-- touches connector_type in a comment only\nALTER TABLE connector DROP CONSTRAINT connector_type_fk_unused;\n"
        self.assertEqual(check.supported_types(migrations(CREATE + INSERT, other)), ["Alpha", "Gamma -- not a comment"])

    def test_a_directory_without_migrations_is_an_error(self) -> None:
        with self.assertRaises(FileNotFoundError):
            check.supported_types(Path(tempfile.mkdtemp(prefix="empty")))


class CompareTests(unittest.TestCase):
    def load(self, types: dict):
        return check.gate_summary.Registry.load(registry_file(types))

    def test_a_supported_type_with_no_registry_entry_is_a_problem(self) -> None:
        report = check.compare(["Alpha", "Beta"], self.load({"Alpha": entry()}))
        self.assertEqual(len(report.problems), 1)
        self.assertIn("Beta", report.problems[0])

    def test_a_registry_entry_for_an_unsupported_type_is_a_problem(self) -> None:
        report = check.compare(["Alpha"], self.load({"Alpha": entry(), "Old": entry()}))
        self.assertTrue(any(p.startswith("Old:") for p in report.problems))

    def test_a_type_with_no_gate_and_no_waiver_is_a_problem(self) -> None:
        missing = {s: "missing" for s in check.gate_summary.STEPS}
        report = check.compare(["Alpha"], self.load({"Alpha": entry(gate=None, steps=missing)}))
        self.assertIn("no gate", report.problems[0])
        self.assertIn("not waived", report.problems[0])

    def test_a_single_missing_step_of_a_gated_type_is_a_problem(self) -> None:
        steps = dict(STEPS_REAL, discover="missing")
        report = check.compare(["Alpha"], self.load({"Alpha": entry(steps=steps)}))
        self.assertIn("discover missing", report.problems[0])

    def test_a_waived_type_is_listed_as_waived_and_is_not_a_problem(self) -> None:
        missing = {s: "missing" for s in check.gate_summary.STEPS}
        waiver = {"until": "SRC-4", "reason": "no template"}
        report = check.compare(["Alpha"], self.load({"Alpha": entry(gate=None, steps=missing, waiver=waiver)}))
        self.assertEqual(report.problems, [])
        self.assertEqual(len(report.waived), 1)
        self.assertIn("WAIVED until SRC-4", report.waived[0])
        self.assertIn("no template", report.waived[0])

    def test_a_waiver_on_a_fully_covered_type_is_a_problem(self) -> None:
        waiver = {"until": "SRC-4", "reason": "stale"}
        report = check.compare(["Alpha"], self.load({"Alpha": entry(waiver=waiver)}))
        self.assertIn("remove the waiver", report.problems[0])

    def test_refused_steps_count_as_covered(self) -> None:
        steps = {"connect": "refused", "discover": "refused", "load": "real", "reload": "real"}
        notes = {"connect": "SRC-5", "discover": "SRC-13"}
        reg = self.load({"Alpha": dict(entry(steps=steps), notes=notes)})
        report = check.compare(["Alpha"], reg)
        self.assertEqual((report.problems, report.covered), ([], ["Alpha"]))


class MainTests(unittest.TestCase):
    def run_main(self, mig: Path, reg: Path) -> tuple[int, str, str]:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = check.main(["--migrations", str(mig), "--registry", str(reg)])
        return code, out.getvalue(), err.getvalue()

    def test_a_clean_tree_exits_zero_and_prints_its_waivers(self) -> None:
        mig = migrations(CREATE + INSERT)
        missing = {s: "missing" for s in check.gate_summary.STEPS}
        reg = registry_file({
            "Alpha": entry(),
            "Gamma -- not a comment": entry(gate=None, steps=missing, waiver={"until": "SRC-4", "reason": "r"}),
        })
        code, out, _ = self.run_main(mig, reg)
        self.assertEqual(code, 0)
        self.assertIn("WAIVED until SRC-4", out)

    def test_a_missing_entry_exits_one_and_names_the_type_on_stderr(self) -> None:
        code, _, err = self.run_main(migrations(CREATE + INSERT), registry_file({"Alpha": entry()}))
        self.assertEqual(code, 1)
        self.assertIn("Gamma -- not a comment", err)

    def test_a_migration_shape_it_cannot_read_exits_one(self) -> None:
        code, _, err = self.run_main(migrations(CREATE + INSERT, "TRUNCATE connector_type;"), registry_file({"Alpha": entry()}))
        self.assertEqual(code, 1)
        self.assertIn("does not understand", err)

    def test_the_real_tree_passes_today(self) -> None:
        code, out, err = self.run_main(check.DEFAULT_MIGRATIONS, check.DEFAULT_REGISTRY)
        self.assertEqual((code, err), (0, ""), out)


if __name__ == "__main__":
    unittest.main()
