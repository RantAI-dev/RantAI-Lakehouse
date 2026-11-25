"""Unit tests for BLD-12 incremental materialization (no network, no Postgres).

Every ClickHouse call is replaced: `_ch_exec` records statements and
`_ch_query_json` answers from a router keyed on the statement text. That
lets each test assert the EXACT SQL the writer sends — the watermark literal,
the partition-aligned lower bound, the `REPLACE PARTITION` ids — rather than
trusting a mock that returns the right row count no matter what it was asked.

Run with: `python -m pytest dagster/dispar_orchestrate/test_incremental_write.py`
"""

from __future__ import annotations

import unittest
from typing import Any, Callable
from unittest import mock

from dispar_orchestrate.bronze_catalog import ClickHouseTarget
from dispar_orchestrate.incremental_write import (
    IncrementalConfig,
    _helpers,
    _is_date_type,
    _is_nullable,
    _literal,
    write_incrementally,
)

TARGET = ClickHouseTarget("http://ch.invalid", "default", "")
SELECT = "SELECT id, event_time, amount FROM silver.`orders`"
ZONE = "serving"
TABLE = "orders_daily"

_APPEND = IncrementalConfig("incremental", "event_time", "append", "day", 0)
_REPLACE_DAY = IncrementalConfig("incremental", "event_time", "partition_replace", "day", 6)
_BATCH = IncrementalConfig("batch", None, "append", "day", 0)


def _compiled(name_type: tuple[str, str] = ("event_time", "DateTime")) -> list[dict]:
    return [{"name": name_type[0], "type": name_type[1]}]


class _ChStub:
    """Records statements and answers queries from a caller-supplied router."""

    def __init__(self, router: Callable[[str], list[dict]] | None = None) -> None:
        self.statements: list[str] = []
        self.router = router

    def execute(self, _target: ClickHouseTarget, statement: str) -> None:
        self.statements.append(statement)

    def query(self, _target: ClickHouseTarget, statement: str) -> list[dict]:
        self.statements.append(statement)
        if self.router is None:
            return []
        return self.router(statement)


class TestIncrementalConfig(unittest.TestCase):
    def test_from_payload_defaults_a_pre_migration_row_to_batch(self) -> None:
        config = IncrementalConfig.from_payload({"kind": "batch", "incrementalColumn": None})
        self.assertEqual(config.kind, "batch")
        self.assertEqual(config.strategy, "append")
        self.assertEqual(config.partition_granularity, "day")
        self.assertEqual(config.lookback_hours, 0)
        self.assertFalse(config.full_refresh)

    def test_from_payload_reads_every_bld12_field(self) -> None:
        config = IncrementalConfig.from_payload(
            {
                "kind": "incremental",
                "incrementalColumn": "event_time",
                "incrementalStrategy": "partition_replace",
                "partitionGranularity": "month",
                "lookbackHours": 24,
            },
            True,
        )
        self.assertEqual(config, IncrementalConfig("incremental", "event_time", "partition_replace", "month", 24, True))

    def test_date_types_unwrap_nullable_and_low_cardinality(self) -> None:
        for value in ("Date", "Date32", "DateTime", "DateTime64(3)", "Nullable(Date)", "LowCardinality(DateTime)"):
            self.assertTrue(_is_date_type(value), value)
        for value in ("String", "Int64", "Nullable(UInt32)"):
            self.assertFalse(_is_date_type(value), value)
        for value in ("Nullable(DateTime)", "LowCardinality(Nullable(Date))"):
            self.assertTrue(_is_nullable(value), value)
        for value in ("DateTime", "Date32", "LowCardinality(DateTime)", "String"):
            self.assertFalse(_is_nullable(value), value)


class TestTypedLiterals(unittest.TestCase):
    def test_watermarks_are_typed_and_strings_are_escaped(self) -> None:
        self.assertEqual(_literal("2026-01-02", "Date"), "toDate('2026-01-02')")
        self.assertEqual(
            _literal("2026-01-02 03:04:05", "DateTime64(6)"),
            "parseDateTime64BestEffort('2026-01-02 03:04:05', 6)",
        )
        self.assertEqual(_literal("42", "Int64"), "toInt64(42)")
        self.assertEqual(
            _literal("2026-01-02 03:04:05", "DateTime('UTC')"),
            "toDateTime('2026-01-02 03:04:05')",
        )
        self.assertEqual(
            _literal("18446744073709551615", "UInt64"),
            "toUInt64(18446744073709551615)",
        )
        self.assertEqual(_literal("o'hara", "String"), _literal("o'hara", "String"))
        self.assertIn("\\'", _literal("o'hara", "String"))
        self.assertEqual(
            _literal("2026-01-02", "Nullable(Date)"),
            "toDate('2026-01-02')",
        )

    def test_a_non_numeric_watermark_for_a_numeric_cursor_is_refused(self) -> None:
        with self.assertRaises(RuntimeError):
            _literal("not-a-number", "Int64")
        with self.assertRaises(RuntimeError):
            _literal("2026-01-02", "SomeExoticType")
        with self.assertRaises(RuntimeError):
            _literal("-1", "UInt64")


class TestWriteIncrementally(unittest.TestCase):
    def setUp(self) -> None:
        self.ch = _ChStub()

    def _run(self, config: IncrementalConfig, router: Callable[[str], list[dict]]) -> MaterializationResult:
        self.ch.router = router
        with mock.patch("dispar_orchestrate.incremental_write._ch_exec", side_effect=self.ch.execute), \
             mock.patch("dispar_orchestrate.incremental_write._ch_query_json", side_effect=self.ch.query):
            return write_incrementally(TARGET, ZONE, TABLE, SELECT, False, config)

    @staticmethod
    def _incremental_router(
        *,
        column_type: str = "DateTime",
        exists: int = 1,
        columns: list[dict] | None = None,
        partition_key: str = "",
        count: int = 10,
        wm: Any = "2026-01-02 03:00:00",
        partitions: list[dict] | None = None,
    ) -> Callable[[str], list[dict]]:
        def router(statement: str) -> list[dict]:
            if "FROM system.tables" in statement and "partition_key" in statement:
                return [{"partition_key": partition_key}]
            if "FROM system.tables" in statement:
                return [{"n": exists}]
            if "DESCRIBE TABLE" in statement:
                return columns if columns is not None else _compiled(("event_time", column_type))
            if "FROM system.columns" in statement:
                return columns if columns is not None else _compiled(("event_time", column_type))
            if "FROM system.parts" in statement:
                return partitions or []
            if "SELECT count() AS n, max(" in statement:
                return [{"n": count, "wm": wm}]
            if "SELECT max(" in statement:
                return [{"wm": wm}]
            if "SELECT count() AS n FROM" in statement:
                return [{"n": count}]
            raise AssertionError(f"unexpected query: {statement}")

        return router

    def test_batch_runs_an_atomic_full_rebuild_with_the_plain_engine(self) -> None:
        counts = iter([0, 3])
        result = self._run(_BATCH, lambda s: [{"n": next(counts)}])
        self.assertEqual(result.materialization_mode, "full")
        self.assertEqual(result.full_reason, "configured_batch")
        self.assertEqual(result.rows, 3)
        self.assertNotIn("incremental_column", result.metadata())
        self.assertNotIn("watermark_before", result.metadata())
        self.assertNotIn("watermark_after", result.metadata())
        create = next(s for s in self.ch.statements if s.startswith("CREATE TABLE"))
        self.assertIn("ENGINE = MergeTree ORDER BY tuple()", create)
        self.assertIn(SELECT, create)
        self.assertTrue(any(s.startswith("EXCHANGE TABLES") for s in self.ch.statements))

    def test_full_refresh_of_an_incremental_pipeline_is_reported_as_requested(self) -> None:
        config = IncrementalConfig("incremental", "event_time", "append", "day", 0, True)
        counts = iter([5, 9])
        watermarks = iter(["2026-01-02 03:00:00", "2026-01-02 05:00:00"])

        base = self._incremental_router()

        def router(statement: str) -> list[dict]:
            if statement.startswith("SELECT max("):
                return [{"wm": next(watermarks)}]
            if statement.startswith("SELECT count() AS n"):
                return [{"n": next(counts)}]
            return base(statement)

        result = self._run(config, router)
        self.assertEqual(result.full_reason, "requested")
        self.assertEqual(result.materialization_mode, "full")
        self.assertEqual(result.incremental_column, "event_time")
        # BEFORE must be read from the OLD target, not after EXCHANGE.
        self.assertEqual(result.watermark_before, "2026-01-02 03:00:00")
        self.assertEqual(result.watermark_after, "2026-01-02 05:00:00")
        self.assertNotEqual(result.watermark_before, result.watermark_after)
        self.assertIn("watermark_before", result.metadata())
        self.assertIn("watermark_after", result.metadata())

    def test_first_run_runs_a_full_rebuild(self) -> None:
        result = self._run(_APPEND, self._incremental_router(exists=0, count=0, wm=None))
        self.assertEqual(result.full_reason, "first_run")
        self.assertEqual(result.materialization_mode, "full")

    def test_a_changed_schema_triggers_a_self_healing_full_rebuild(self) -> None:
        base = self._incremental_router(
            columns=[{"name": "event_time", "type": "DateTime"}, {"name": "extra", "type": "String"}]
        )
        def router(statement: str) -> list[dict]:
            if "FROM system.columns" in statement:
                return [{"name": "event_time", "type": "DateTime"}]
            return base(statement)

        result = self._run(_APPEND, router)
        self.assertEqual(result.full_reason, "schema_changed")
        self.assertTrue(any(s.startswith("EXCHANGE TABLES") for s in self.ch.statements))

    def test_a_changed_partition_key_triggers_a_full_rebuild(self) -> None:
        result = self._run(_REPLACE_DAY, self._incremental_router(partition_key="toYYYYMM(event_time)"))
        self.assertEqual(result.full_reason, "partition_key_changed")

    def test_an_empty_target_triggers_a_full_rebuild(self) -> None:
        result = self._run(_APPEND, self._incremental_router(count=0, wm=None))
        self.assertEqual(result.full_reason, "empty_target")

    def test_append_stages_newer_rows_then_inserts_only_the_delta(self) -> None:
        staged = iter([2, 12, "2026-01-02 05:00:00"])
        result = self._run(
            _APPEND,
            lambda s: (
                self._incremental_router()(s)
                if not ("__staging_" in s and "SELECT count()" in s)
                else [{"n": next(staged)}]
            ),
        )
        self.assertEqual(result.materialization_mode, "incremental_append")
        self.assertEqual(result.rows, 2)
        self.assertEqual(result.watermark_before, "2026-01-02 03:00:00")
        self.assertEqual(result.metadata()["partitions_replaced"], 0)
        stage_insert = next(s for s in self.ch.statements if s.startswith("INSERT INTO") and "__staging_" in s)
        self.assertIn("`event_time` > toDateTime('2026-01-02 03:00:00')", stage_insert)
        self.assertIn("AS __bld12_src", stage_insert)
        self.assertFalse(any(s.startswith("EXCHANGE TABLES") for s in self.ch.statements))
        self.assertTrue(any(s == f"DROP TABLE IF EXISTS {ZONE}.`__staging_{TABLE}`" for s in self.ch.statements))

    def test_append_with_zero_new_rows_does_not_insert_into_the_live_target(self) -> None:
        router = self._incremental_router()
        def answer(s: str) -> list[dict]:
            if "__staging_" in s and "SELECT count()" in s:
                return [{"n": 0}]
            return router(s)

        result = self._run(_APPEND, answer)
        self.assertEqual(result.rows, 0)
        self.assertFalse(
            any(s.startswith("INSERT INTO") and f"`{TABLE}`" in s for s in self.ch.statements)
        )

    def test_partition_replace_uses_a_partition_aligned_window_and_replaces_only_touched_partitions(
        self,
    ) -> None:
        counts = iter([100, 120, "2026-01-02 05:00:00"])
        partitions = [{"partition_id": "20260102"}, {"partition_id": "20260103"}]
        router = self._incremental_router(partitions=partitions, partition_key="toYYYYMMDD(event_time)")
        def answer(s: str) -> list[dict]:
            if "SELECT count() AS n, max(" in s:
                return [{"n": 100, "wm": "2026-01-02 05:00:00"}]
            if "SELECT count() AS n FROM" in s and s.count("SELECT") == 1 and "__staging_" not in s and "system." not in s:
                return [{"n": next(counts)}]
            if "SELECT max(" in s and "count()" not in s:
                return [{"wm": "2026-01-02 05:00:00"}]
            return router(s)

        result = self._run(_REPLACE_DAY, answer)
        self.assertEqual(result.materialization_mode, "incremental_partition_replace")
        self.assertEqual(result.rows, 20)
        self.assertEqual(result.partitions_replaced, 2)
        create = next(s for s in self.ch.statements if s.startswith("CREATE TABLE") and "__staging_" in s)
        self.assertIn(
            "ENGINE = MergeTree PARTITION BY toYYYYMMDD(`event_time`) ORDER BY (`event_time`)",
            create,
        )
        stage_insert = next(s for s in self.ch.statements if s.startswith("INSERT INTO") and "__staging_" in s)
        self.assertIn(
            "`event_time` >= toStartOfDay(toDateTime('2026-01-02 05:00:00') - INTERVAL 6 HOUR)",
            stage_insert,
        )
        alters = [s for s in self.ch.statements if s.startswith("ALTER TABLE")]
        self.assertEqual(len(alters), 2)
        self.assertIn("REPLACE PARTITION ID '20260102'", alters[0])
        self.assertFalse(any(s.startswith("EXCHANGE TABLES") for s in self.ch.statements))

    def test_month_granularity_and_lookback_reach_the_window_expression(self) -> None:
        config = IncrementalConfig("incremental", "event_time", "partition_replace", "month", 48)
        self._run(config, self._incremental_router(partition_key="toYYYYMM(event_time)"))
        stage_insert = next(s for s in self.ch.statements if s.startswith("INSERT INTO") and "__staging_" in s)
        self.assertIn(
            "toStartOfMonth(toDateTime('2026-01-02 03:00:00') - INTERVAL 48 HOUR)",
            stage_insert,
        )
        create = next(s for s in self.ch.statements if s.startswith("CREATE TABLE") and "__staging_" in s)
        self.assertIn("toYYYYMM(`event_time`)", create)

    def test_an_unsafe_partition_id_is_refused_before_any_alter(self) -> None:
        router = self._incremental_router(
            partition_key="toYYYYMMDD(event_time)",
            partitions=[{"partition_id": "2026'; DROP TABLE x --"}],
        )
        with self.assertRaises(RuntimeError):
            self._run(_REPLACE_DAY, router)
        self.assertFalse(any(s.startswith("ALTER TABLE") for s in self.ch.statements))

    def test_staging_is_dropped_even_when_the_staged_insert_fails(self) -> None:
        self.ch.router = self._incremental_router()

        def explode(_target: ClickHouseTarget, statement: str) -> None:
            self.ch.statements.append(statement)
            if statement.startswith("INSERT INTO") and "__staging_" in statement:
                raise RuntimeError("source read timed out")

        with mock.patch("dispar_orchestrate.incremental_write._ch_exec", side_effect=explode), \
             mock.patch("dispar_orchestrate.incremental_write._ch_query_json", side_effect=self.ch.query):
            with self.assertRaises(RuntimeError):
                write_incrementally(TARGET, ZONE, TABLE, SELECT, False, _APPEND)
        self.assertEqual(self.ch.statements[-1], f"DROP TABLE IF EXISTS {ZONE}.`__staging_{TABLE}`")
        self.assertFalse(any(s.startswith("INSERT INTO") and f"`{TABLE}`" in s for s in self.ch.statements))

    def test_a_cursor_missing_from_the_compiled_output_is_refused(self) -> None:
        router = self._incremental_router(columns=[{"name": "other", "type": "DateTime"}])
        with self.assertRaises(RuntimeError):
            self._run(_APPEND, router)
        self.assertFalse(any(s.startswith("INSERT INTO") for s in self.ch.statements))

    def test_partition_replace_with_a_text_cursor_is_refused(self) -> None:
        router = self._incremental_router(column_type="String")
        with self.assertRaises(RuntimeError):
            self._run(_REPLACE_DAY, router)
        self.assertFalse(any(s.startswith("ALTER TABLE") for s in self.ch.statements))

    def test_an_unsafe_cursor_or_out_of_range_lookback_is_refused(self) -> None:
        for config in (
            IncrementalConfig("incremental", "bad; DROP", "append", "day", 0),
            IncrementalConfig("incremental", "1bad", "append", "day", 0),
            IncrementalConfig("incremental", None, "append", "day", 0),
            IncrementalConfig("incremental", "event_time", "append", "day", 721),
            IncrementalConfig("incremental", "event_time", "merge", "day", 0),
            IncrementalConfig("incremental", "event_time", "append", "week", 0),
        ):
            with self.subTest(config=config), self.assertRaises(RuntimeError):
                self._run(config, self._incremental_router())

    def test_iceberg_reads_carry_the_setting_on_every_source_read(self) -> None:
        with mock.patch("dispar_orchestrate.incremental_write._ch_exec", side_effect=self.ch.execute), \
             mock.patch("dispar_orchestrate.incremental_write._ch_query_json", side_effect=self.ch.query):
            write_incrementally(TARGET, ZONE, TABLE, SELECT, True, _BATCH)
        statements = [s for s in self.ch.statements if SELECT in s]
        self.assertTrue(statements)
        for statement in statements:
            self.assertIn("SETTINGS allow_database_iceberg = 1", statement)

    def test_authored_job_error_is_the_factories_non_retryable_type(self) -> None:
        error_type, is_safe, reading, canonical = _helpers()
        self.assertTrue(callable(is_safe))
        self.assertTrue(callable(reading))
        self.assertTrue(callable(canonical))
        self.assertTrue(callable(error_type))
        with self.assertRaises(error_type):
            self._run(_APPEND, self._incremental_router(columns=[{"name": "other", "type": "Date"}]))


    def test_a_legacy_gold_target_zone_is_canonicalized_to_serving_in_every_statement(self) -> None:
        counts = iter([0, 3])
        self.ch.router = lambda s: [{"n": next(counts)}]
        with mock.patch("dispar_orchestrate.incremental_write._ch_exec", side_effect=self.ch.execute), \
             mock.patch("dispar_orchestrate.incremental_write._ch_query_json", side_effect=self.ch.query):
            write_incrementally(TARGET, "gold", TABLE, SELECT, False, _BATCH)

        self.assertIn("CREATE DATABASE IF NOT EXISTS serving", self.ch.statements)
        self.assertTrue(
            any("CREATE TABLE" in s and "serving.`" + TABLE + "`" in s for s in self.ch.statements)
        )
        self.assertFalse(any("gold." in s for s in self.ch.statements))

    def test_probe_and_append_staging_tables_have_an_engine_and_the_database_exists(self) -> None:
        self._run(_APPEND, self._incremental_router())
        probe = next(
            s for s in self.ch.statements if s.startswith("CREATE TABLE") and "__probe_" in s
        )
        staging = next(
            s for s in self.ch.statements if s.startswith("CREATE TABLE") and "__staging_" in s
        )
        for create in (probe, staging):
            self.assertIn("ENGINE = MergeTree ORDER BY tuple()", create)
        self.assertIn("CREATE DATABASE IF NOT EXISTS serving", self.ch.statements)

    def test_partition_replace_refuses_a_nullable_date_cursor_before_any_engine_ddl(self) -> None:
        router = self._incremental_router(column_type="Nullable(DateTime)")
        with self.assertRaises(RuntimeError) as caught:
            self._run(_REPLACE_DAY, router)
        self.assertIn("non-nullable Date or DateTime cursor", str(caught.exception))
        self.assertFalse(any(s.startswith("ALTER TABLE") for s in self.ch.statements))
        self.assertFalse(
            any("PARTITION BY" in s for s in self.ch.statements if s.startswith("CREATE TABLE"))
        )


if __name__ == "__main__":
    unittest.main()
