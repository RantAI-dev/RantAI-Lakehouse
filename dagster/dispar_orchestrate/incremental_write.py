"""Real incremental materialization for a compiled authored pipeline (BLD-12).

`authored_factory.py` asks the API for the statement a stored definition
compiles to, then calls this module to write its output. The factory used to
always stage the whole result and `EXCHANGE TABLES` it into the target: safe,
but every run paid full-scan cost and a downstream reader saw a complete
replacement even when one row had changed. This module keeps that atomic
full rebuild as the fallback and adds two incremental strategies:

* ``append`` — insert only rows whose cursor is newer than the live target's
  watermark. For immutable logs and events.
* ``partition_replace`` — compute a partition-aligned lower bound, stage the
  rows at or after it, and replace only the partitions the stage touched. For
  mutable tables, rollups and late-arriving data, where a row that arrived
  after yesterday's run must correct yesterday's partition.

Both strategies stage into ``__staging_<table>`` first. A failed or timed-out
source read leaves the live target completely untouched; that is the whole
reason for the extra table rather than inserting into the live one.

# Fail closed

Every value interpolated into SQL here is either a server-controlled,
validated identifier (zone/table/cursor column) or a typed literal built from
the live watermark. The cursor column is re-checked against
``authored_factory._is_safe_ch_identifier`` and must be present in the
compiled SELECT's output; ``partition_replace`` further requires a Date or
DateTime cursor, unwrapping ``Nullable`` and ``LowCardinality``. A condition
this build cannot execute raises ``AuthoredJobError``, which the factory wraps
in a non-retryable ``Failure`` — an unsupported config will not change
between attempts, so retrying it only burns the op's retry budget.

# Self-healing fallback

A full atomic rebuild runs when any of these holds: the kind is ``batch``, a
full refresh was requested, the target does not exist yet, the live column
schema differs from the compiled SELECT, the live partition key differs from
the configured one, or the target is empty. The last three make a definition
edit (or a first run after a manual cleanup) self-heal instead of appending
into a table with the wrong shape.

No fabricated metrics: ``rows`` is the real delta between the before/after
count, and ``partitions_replaced`` is the count of partition ids actually
replaced, not a guessed number.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, _sql_string_literal


def _ch_exec(target: ClickHouseTarget, statement: str) -> None:
    """Go through the factory's own names so its existing tests (which patch
    `authored_factory._ch_exec` / `_ch_query_json`) keep intercepting every
    statement. Looked up at CALL time, not import time, so the lazy import
    cannot create a cycle with `authored_factory.py`."""
    from dispar_orchestrate import authored_factory

    authored_factory._ch_exec(target, statement)


def _ch_query_json(target: ClickHouseTarget, statement: str) -> list[dict]:
    """See [`_ch_exec`] — same late lookup, same reason."""
    from dispar_orchestrate import authored_factory

    return authored_factory._ch_query_json(target, statement)

# The cursor column checked at the API boundary can be at most 128 characters.
MAX_CURSOR_CHARS = 128
MAX_LOOKBACK_HOURS = 720
PARTITION_ID_SAFE = re.compile(r"^[0-9A-Za-z_-]+$")


@dataclass(frozen=True)
class IncrementalConfig:
    """What the API's `compile` payload says about this pipeline's write."""

    kind: str
    column: str | None
    strategy: str
    partition_granularity: str
    lookback_hours: int
    full_refresh: bool = False

    @classmethod
    def from_payload(
        cls,
        definition: dict[str, Any],
        full_refresh: bool = False,
    ) -> IncrementalConfig:
        """Read the BLD-12 fields from a runnable definition or compile answer.

        A row written before migration `0156` has no strategy fields; it is
        treated as a batch pipeline, which is the behaviour it had.
        """
        return cls(
            kind=str(definition.get("kind") or "batch"),
            column=definition.get("incrementalColumn"),
            strategy=str(definition.get("incrementalStrategy") or "append"),
            partition_granularity=str(definition.get("partitionGranularity") or "day"),
            lookback_hours=int(definition.get("lookbackHours") or 0),
            full_refresh=full_refresh,
        )


@dataclass(frozen=True)
class MaterializationResult:
    """The honest outcome of one write, ready for an AssetMaterialization."""

    rows: int
    materialization_mode: str
    full_reason: str | None = None
    incremental_column: str | None = None
    watermark_before: Any = None
    watermark_after: Any = None
    partitions_replaced: int = 0

    def metadata(self) -> dict[str, Any]:
        """Dagster metadata: only the conditional keys that apply to this run."""
        out: dict[str, Any] = {
            "rows": self.rows,
            "materialization_mode": self.materialization_mode,
            "watermark_before": self.watermark_before,
            "watermark_after": self.watermark_after,
            "partitions_replaced": self.partitions_replaced,
        }
        if self.full_reason is not None:
            out["full_refresh_reason"] = self.full_reason
        if self.incremental_column is not None:
            out["incremental_column"] = self.incremental_column
        return out


def _helpers() -> tuple[type[RuntimeError], Any, Any]:
    """Lazy imports: `authored_factory.py` imports this module at load time,
    so importing it back at module scope would be a cycle. By the time a run
    calls this the factory is fully loaded."""
    from dispar_orchestrate.authored_factory import (
        AuthoredJobError,
        _is_safe_ch_identifier,
        _reading,
    )

    return AuthoredJobError, _is_safe_ch_identifier, _reading


def _refuse(message: str) -> RuntimeError:
    """A non-retryable config refusal, raised as the factory's own error type."""
    error_type, _, _ = _helpers()
    return error_type(message)


def _engine_clause(config: IncrementalConfig, column: str) -> str:
    """The target's engine clause. Batch and append keep the original
    `MergeTree ORDER BY tuple()`; partition_replace partitions by the cursor
    so an `ALTER TABLE ... REPLACE PARTITION` can address a whole time window."""
    if config.kind == "incremental" and config.strategy == "partition_replace":
        function = "toYYYYMMDD" if config.partition_granularity == "day" else "toYYYYMM"
        return (
            f"ENGINE = MergeTree PARTITION BY {function}(`{column}`) "
            f"ORDER BY (`{column}`)"
        )
    return "ENGINE = MergeTree ORDER BY tuple()"


def _normalized_partition_key(expression: str | None) -> str:
    """Compare a live `system.tables.partition_key` with the expected one
    without letting backticks or whitespace decide the answer."""
    if not expression:
        return ""
    return expression.replace("`", "").replace(" ", "").strip()


def _expected_partition_key(config: IncrementalConfig, column: str) -> str:
    if config.kind == "incremental" and config.strategy == "partition_replace":
        function = "toYYYYMMDD" if config.partition_granularity == "day" else "toYYYYMM"
        return _normalized_partition_key(f"{function}(`{column}`)")
    return ""


def _base_type(ch_type: str) -> str:
    """Unwrap `Nullable(...)` / `LowCardinality(...)` to the underlying type."""
    result = ch_type.strip()
    for wrapper in ("Nullable", "LowCardinality"):
        prefix = f"{wrapper}("
        while result.startswith(prefix) and result.endswith(")"):
            result = result[len(prefix) : -1].strip()
    return result


def _is_date_type(ch_type: str) -> bool:
    base = _base_type(ch_type)
    return base == "Date" or base == "Date32" or base.startswith("DateTime")


def _literal(value: Any, ch_type: str) -> str:
    """A typed ClickHouse literal for the live watermark. A raw value is
    NEVER interpolated: strings go through `_sql_string_literal`, dates and
    UUIDs through their own cast, numbers through `int()` first."""
    base = _base_type(ch_type)
    text = str(value)
    if base in ("Date", "Date32"):
        cast = "toDate" if base == "Date" else "toDate32"
        return f"{cast}({_sql_string_literal(text)})"
    if base == "DateTime":
        return f"toDateTime({_sql_string_literal(text)})"
    if base.startswith("DateTime64"):
        precision = "3"
        if base.endswith(")") and "(" in base:
            precision = base[base.index("(") + 1 : -1].split(",")[0]
        return f"parseDateTime64BestEffort({_sql_string_literal(text)}, {precision})"
    if base == "UUID":
        return f"toUUID({_sql_string_literal(text)})"
    if base.startswith(("Int", "UInt")):
        try:
            number = int(value)
        except (TypeError, ValueError) as exc:
            raise _refuse("the incremental cursor's watermark is not an integer") from exc
        return f"toInt64({number})"
    if base.startswith("Float"):
        try:
            number = float(value)
        except (TypeError, ValueError) as exc:
            raise _refuse("the incremental cursor's watermark is not a number") from exc
        return f"toFloat64({number})"
    if base == "String":
        return _sql_string_literal(text)
    raise _refuse("unsupported incremental cursor type for a typed watermark literal")


def _table_exists(target: ClickHouseTarget, zone: str, table: str) -> bool:
    rows = _ch_query_json(
        target,
        "SELECT count() AS n FROM system.tables WHERE database = "
        f"{_sql_string_literal(zone)} AND name = {_sql_string_literal(table)}",
    )
    return bool(rows) and int(rows[0]["n"]) > 0


def _live_columns(target: ClickHouseTarget, zone: str, table: str) -> list[tuple[str, str]]:
    rows = _ch_query_json(
        target,
        "SELECT name, type FROM system.columns WHERE database = "
        f"{_sql_string_literal(zone)} AND table = {_sql_string_literal(table)} "
        "ORDER BY position",
    )
    return [(str(row["name"]), str(row["type"])) for row in rows]


def _live_partition_key(target: ClickHouseTarget, zone: str, table: str) -> str:
    rows = _ch_query_json(
        target,
        "SELECT partition_key FROM system.tables WHERE database = "
        f"{_sql_string_literal(zone)} AND name = {_sql_string_literal(table)}",
    )
    return _normalized_partition_key(str(rows[0]["partition_key"]) if rows else None)


def _compiled_columns(
    target: ClickHouseTarget,
    zone: str,
    table: str,
    select_sql: str,
    reads_iceberg: bool,
) -> list[tuple[str, str]]:
    """Describe the compiled SELECT through a zero-row probe table. A source
    read that fails leaves no probe behind (`DROP` is in the `finally`), and
    the live target is never touched."""
    _, _, _reading = _helpers()
    probe = f"__probe_{table}"
    _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{probe}`")
    try:
        _ch_exec(
            target,
            _reading(
                reads_iceberg,
                f"CREATE TABLE {zone}.`{probe}` AS {select_sql} LIMIT 0",
            ),
        )
        rows = _ch_query_json(target, f"DESCRIBE TABLE {zone}.`{probe}`")
        return [(str(row["name"]), str(row["type"])) for row in rows]
    finally:
        _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{probe}`")


def _staging_name(table: str) -> str:
    return f"__staging_{table}"


def _count(target: ClickHouseTarget, zone: str, table: str) -> int:
    rows = _ch_query_json(target, f"SELECT count() AS n FROM {zone}.`{table}`")
    return int(rows[0]["n"]) if rows else 0


def _watermark(
    target: ClickHouseTarget, zone: str, table: str, column: str
) -> Any:
    rows = _ch_query_json(
        target,
        f"SELECT max(`{column}`) AS wm FROM {zone}.`{table}`",
    )
    return rows[0]["wm"] if rows else None


def _full_rebuild(
    target: ClickHouseTarget,
    zone: str,
    table: str,
    select_sql: str,
    reads_iceberg: bool,
    config: IncrementalConfig,
    column: str | None,
    reason: str,
) -> MaterializationResult:
    """The original atomic staging + `EXCHANGE TABLES`, with the engine clause
    the target actually needs. This is both the batch path and the fallback
    for every shape that cannot be safely materialized incrementally."""
    _, _, _reading = _helpers()
    staging = _staging_name(table)
    clause = _engine_clause(config, column) if column is not None else _engine_clause(config, "")
    # Keep the target-creation sequence the pre-BLD-12 op used: the database
    # and the target's zero-row shape exist before the staging swap, so a
    # reader that looks the target up mid-run still sees the right table.
    _ch_exec(target, f"CREATE DATABASE IF NOT EXISTS {zone}")
    _ch_exec(
        target,
        _reading(
            reads_iceberg,
            f"CREATE TABLE IF NOT EXISTS {zone}.`{table}` {clause} AS {select_sql} LIMIT 0",
        ),
    )
    _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{staging}`")
    try:
        _ch_exec(
            target,
            _reading(
                reads_iceberg,
                f"CREATE TABLE {zone}.`{staging}` {clause} AS {select_sql} LIMIT 0",
            ),
        )
        before = _count(target, zone, table)
        _ch_exec(
            target,
            _reading(reads_iceberg, f"INSERT INTO {zone}.`{staging}` {select_sql}"),
        )
        _ch_exec(
            target,
            f"EXCHANGE TABLES {zone}.`{table}` AND {zone}.`{staging}`",
        )
        after = _count(target, zone, table)
    finally:
        _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{staging}`")
    return MaterializationResult(
        rows=after - before,
        materialization_mode="full",
        full_reason=reason,
        incremental_column=column if config.kind == "incremental" else None,
        watermark_before=_watermark(target, zone, table, column) if column else None,
        watermark_after=_watermark(target, zone, table, column) if column else None,
    )


def _append(
    target: ClickHouseTarget,
    zone: str,
    table: str,
    select_sql: str,
    reads_iceberg: bool,
    config: IncrementalConfig,
    column: str,
    column_type: str,
    watermark: Any,
) -> MaterializationResult:
    """Watermark append: stage rows newer than the live max and insert only
    those. A failed source read leaves the live target untouched."""
    _, _, _reading = _helpers()
    staging = _staging_name(table)
    wm_expr = _literal(watermark, column_type)
    _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{staging}`")
    try:
        _ch_exec(
            target,
            _reading(
                reads_iceberg,
                f"CREATE TABLE {zone}.`{staging}` AS {select_sql} LIMIT 0",
            ),
        )
        _ch_exec(
            target,
            _reading(
                reads_iceberg,
                f"INSERT INTO {zone}.`{staging}` SELECT * FROM ({select_sql}) "
                f"AS __bld12_src WHERE `{column}` > {wm_expr}",
            ),
        )
        delta = _count(target, zone, staging)
        if delta > 0:
            _ch_exec(
                target,
                f"INSERT INTO {zone}.`{table}` SELECT * FROM {zone}.`{staging}`",
            )
        after = _watermark(target, zone, table, column)
    finally:
        _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{staging}`")
    return MaterializationResult(
        rows=delta,
        materialization_mode="incremental_append",
        incremental_column=column,
        watermark_before=watermark,
        watermark_after=after,
    )


def _window_start_expr(watermark: Any, column_type: str, config: IncrementalConfig) -> str:
    """A PARTITION-boundary lower bound, never a partial day or month. The
    lookback moves it back so late rows still land in a replaced partition."""
    wm_expr = _literal(watermark, column_type)
    function = "toStartOfDay" if config.partition_granularity == "day" else "toStartOfMonth"
    return f"{function}({wm_expr} - INTERVAL {config.lookback_hours} HOUR)"


def _partition_replace(
    target: ClickHouseTarget,
    zone: str,
    table: str,
    select_sql: str,
    reads_iceberg: bool,
    config: IncrementalConfig,
    column: str,
    column_type: str,
    watermark: Any,
) -> MaterializationResult:
    """Replace only the partitions the staged window touched. `system.parts`
    is the source of truth for which partitions that is — no guessing from
    the watermark's own value (a late row can change the answer)."""
    _, _, _reading = _helpers()
    staging = _staging_name(table)
    window_start = _window_start_expr(watermark, column_type, config)
    clause = _engine_clause(config, column)
    _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{staging}`")
    try:
        _ch_exec(
            target,
            _reading(
                reads_iceberg,
                f"CREATE TABLE {zone}.`{staging}` {clause} AS {select_sql} LIMIT 0",
            ),
        )
        _ch_exec(
            target,
            _reading(
                reads_iceberg,
                f"INSERT INTO {zone}.`{staging}` SELECT * FROM ({select_sql}) "
                f"AS __bld12_src WHERE `{column}` >= {window_start}",
            ),
        )
        before = _count(target, zone, table)
        partitions = _ch_query_json(
            target,
            "SELECT DISTINCT partition_id FROM system.parts WHERE database = "
            f"{_sql_string_literal(zone)} AND table = {_sql_string_literal(staging)} "
            "AND active = 1 ORDER BY partition_id",
        )
        replaced: list[str] = []
        for row in partitions:
            partition_id = str(row["partition_id"])
            if not PARTITION_ID_SAFE.fullmatch(partition_id):
                raise _refuse("an unsafe ClickHouse partition id was returned by the staging table")
            _ch_exec(
                target,
                f"ALTER TABLE {zone}.`{table}` REPLACE PARTITION ID "
                f"{_sql_string_literal(partition_id)} FROM {zone}.`{staging}`",
            )
            replaced.append(partition_id)
        after = _count(target, zone, table)
        watermark_after = _watermark(target, zone, table, column)
    finally:
        _ch_exec(target, f"DROP TABLE IF EXISTS {zone}.`{staging}`")
    return MaterializationResult(
        rows=after - before,
        materialization_mode="incremental_partition_replace",
        incremental_column=column,
        watermark_before=watermark,
        watermark_after=watermark_after,
        partitions_replaced=len(replaced),
    )


def write_incrementally(
    target: ClickHouseTarget,
    target_zone: str,
    target_table: str,
    select_sql: str,
    reads_iceberg: bool,
    config: IncrementalConfig,
) -> MaterializationResult:
    """Materialize `select_sql` into `target_zone.target_table` as configured.

    # Errors

    `AuthoredJobError` for a cursor that is unsafe, absent from the compiled
    SELECT, or not a Date/DateTime under `partition_replace`, and for an
    out-of-range lookback. Transport and engine failures propagate as before
    (they may succeed on retry); a config refusal is wrapped in a
    non-retryable Dagster `Failure` by the calling op.
    """
    _, is_safe, _ = _helpers()
    column = config.column
    if config.kind == "incremental":
        if not column or len(column) > MAX_CURSOR_CHARS or not is_safe(column):
            raise _refuse("an unsafe incremental cursor column reached the writer")
        if config.strategy not in ("append", "partition_replace"):
            raise _refuse("unsupported incremental strategy")
        if config.partition_granularity not in ("day", "month"):
            raise _refuse("unsupported partition granularity")
        if not 0 <= config.lookback_hours <= MAX_LOOKBACK_HOURS:
            raise _refuse("lookbackHours is outside 0..720")
    else:
        column = None

    compiled: list[tuple[str, str]] = []
    if column is not None:
        compiled = _compiled_columns(target, target_zone, target_table, select_sql, reads_iceberg)
        types = dict(compiled)
        if column not in types:
            raise _refuse("the incremental cursor column is not in the compiled SELECT output")
        column_type = types[column]
        if config.strategy == "partition_replace" and not _is_date_type(column_type):
            raise _refuse(
                "partition_replace requires a Date or DateTime cursor column; "
                "use the append strategy for a numeric or text cursor"
            )
    else:
        column_type = ""

    if config.kind != "incremental":
        return _full_rebuild(
            target, target_zone, target_table, select_sql, reads_iceberg, config,
            None, "configured_batch",
        )
    if column is None:
        raise _refuse("the incremental cursor column is missing")
    if config.full_refresh:
        return _full_rebuild(
            target, target_zone, target_table, select_sql, reads_iceberg, config,
            column, "requested",
        )
    exists = _table_exists(target, target_zone, target_table)
    if not exists:
        return _full_rebuild(
            target, target_zone, target_table, select_sql, reads_iceberg, config,
            column, "first_run",
        )
    if _live_columns(target, target_zone, target_table) != compiled:
        return _full_rebuild(
            target, target_zone, target_table, select_sql, reads_iceberg, config,
            column, "schema_changed",
        )
    if _live_partition_key(target, target_zone, target_table) != _expected_partition_key(config, column):
        return _full_rebuild(
            target, target_zone, target_table, select_sql, reads_iceberg, config,
            column, "partition_key_changed",
        )
    wm = _watermark(target, target_zone, target_table, column)
    state = _ch_query_json(
        target,
        f"SELECT count() AS n, max(`{column}`) AS wm FROM {target_zone}.`{target_table}`",
    )
    if not state or int(state[0]["n"]) == 0 or state[0]["wm"] is None:
        return _full_rebuild(
            target, target_zone, target_table, select_sql, reads_iceberg, config,
            column, "empty_target",
        )
    if config.strategy == "append":
        return _append(
            target, target_zone, target_table, select_sql, reads_iceberg,
            config, column, column_type, wm,
        )
    return _partition_replace(
        target, target_zone, target_table, select_sql, reads_iceberg,
        config, column, column_type, wm,
    )
