"""Register a freshly-ingested Bronze table on the console's EXISTING
catalog surface — `lake.bronze_meta.dataset_catalog` /
`dataset_sync` / `dataset_column`, the exact tables
`rust/crates/lakehouse-api/src/routes/catalog.rs` (`GET /api/catalog`,
`GET /api/catalog/{id}`) and `routes/governance.rs`'s `classification`/
`lineage` handlers already read. This is deliberately the SAME mechanism
those routes already use, not a parallel one — see the task brief's
"do not invent a parallel mechanism."

# Why this module also creates the tables (`CREATE TABLE IF NOT EXISTS`)

`lake.bronze_meta.*` today only exists where `demo/clickhouse/04_registry.sql`
has been applied by hand (a demo/production deployment's own bootstrap) —
`docker-compose.yml` does not run any ClickHouse init SQL, so a fresh G3a
compose stack has no `lake` database at all. Rather than requiring the G3a
test to separately reproduce `demo/clickhouse/`'s schema (risking drift
from the schema the Rust routes actually query), this module creates the
three tables with the IDENTICAL column shapes `04_registry.sql` defines,
using `IF NOT EXISTS` — a no-op against a deployment where the demo/
production fixture already created them, and the thing that makes a bare
`docker compose up` stack's catalog surface actually work for a real
ingested table for the first time.

# R10 — single schema owner (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §5)

The same tables are ALSO defined by `demo/clickhouse/04_registry.sql` (a
hand-applied demo/production bootstrap fixture, out of scope for this
build to edit). Historically both sides used a bare `CREATE TABLE IF NOT
EXISTS`, which is a silent no-op against a table the other side already
created with a different shape — exactly R10's failure mode: the console
would read wrong data with no error.

This module is now the schema's single owner: `EXPECTED_SCHEMAS` below is
the canonical column/engine/sorting-key definition every one of these
tables must have. `_assert_or_create_schema` either (a) creates the table
fresh — the bare-compose-stack bootstrap case `04_registry.sql` was never
applied for — or (b) if the table already exists (typically because
`04_registry.sql` created it by hand), reads its ACTUAL schema back from
`system.columns`/`system.tables` and raises loudly if it does not match
`EXPECTED_SCHEMAS` exactly, instead of silently trusting whatever is
there. A drift between this file and `04_registry.sql` now fails the
first Dagster run that touches the registry, rather than staying invisible
forever. See `ops/g3/g3_test.py`'s R10 regression case for a
demonstration of this firing on a deliberately mismatched table.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

import requests


class SchemaDriftError(RuntimeError):
    """Raised when an existing `lake.bronze_meta*` table's actual schema
    (columns, engine, or sorting key) does not match `EXPECTED_SCHEMAS` —
    R10: the registry schema now has exactly one owner (this module), and
    a mismatch is a loud failure rather than a silently-kept stale table.
    """


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class ClickHouseTarget:
    url: str
    user: str
    password: str

    @classmethod
    def from_env(cls) -> "ClickHouseTarget":
        return cls(
            url=_env("CH_URL", "http://clickhouse:8123"),
            user=_env("CH_USER", "default"),
            password=_env("CH_PASSWORD", ""),
        )


@dataclass(frozen=True)
class TableSchema:
    """The canonical shape of one `lake.bronze_meta*` table: column
    (name, type) pairs in order, storage engine, and `ORDER BY` key
    columns — the exact facts R10 flags as able to drift silently between
    this file and `demo/clickhouse/04_registry.sql`."""

    table_name: str  # e.g. "bronze_meta.dataset_catalog" (the literal ClickHouse table name)
    columns: tuple[tuple[str, str], ...]
    engine: str
    order_by: tuple[str, ...]

    @property
    def create_ddl(self) -> str:
        cols_sql = ", ".join(f"{name} {ty}" for name, ty in self.columns)
        order_sql = self.order_by[0] if len(self.order_by) == 1 else f"({', '.join(self.order_by)})"
        return (
            f"CREATE TABLE IF NOT EXISTS lake.`{self.table_name}` ({cols_sql}) "
            f"ENGINE = {self.engine} ORDER BY {order_sql}"
        )


def _registry_schemas(prefix: str) -> tuple[TableSchema, ...]:
    return (
        TableSchema(
            table_name=f"{prefix}.dataset_catalog",
            columns=(
                ("slug", "String"),
                ("title", "String"),
                ("description", "String"),
                ("tier", "String"),
                ("updated_at", "String"),
                ("table_name", "String"),
            ),
            engine="ReplacingMergeTree",
            order_by=("slug",),
        ),
        TableSchema(
            table_name=f"{prefix}.dataset_sync",
            columns=(
                ("slug", "String"),
                ("title", "String"),
                ("description", "String"),
                ("table_name", "String"),
                ("total", "UInt64"),
                ("author", "String"),
                ("frekuensi", "String"),
                ("satuan", "String"),
                ("klasifikasi", "String"),
            ),
            engine="ReplacingMergeTree",
            order_by=("slug",),
        ),
        TableSchema(
            table_name=f"{prefix}.dataset_column",
            columns=(
                ("slug", "String"),
                ("key_asli", "String"),
                ("tipe", "String"),
                ("deskripsi", "String"),
            ),
            engine="ReplacingMergeTree",
            order_by=("slug", "key_asli"),
        ),
    )


_MAINTENANCE_RUN_SCHEMA = TableSchema(
    table_name="bronze_meta.maintenance_run",
    columns=(
        ("table_name", "String"),
        ("run_at", "String"),
        ("dry_run_deleted_data_files", "UInt64"),
        ("dry_run_deleted_manifest_files", "UInt64"),
        ("dry_run_deleted_manifest_lists", "UInt64"),
        ("applied_deleted_data_files", "UInt64"),
        ("applied_deleted_manifest_files", "UInt64"),
        ("applied_deleted_manifest_lists", "UInt64"),
        ("skipped_verbs", "String"),
    ),
    engine="ReplacingMergeTree",
    order_by=("table_name", "run_at"),
)


# Every `lakehouse-api` catalog/governance query (`routes::catalog`,
# `routes::governance::classification`/`lineage`) `UNION ALL`s
# `bronze_meta.*` with `bronze_meta_sec.*` unconditionally — so both sets
# of tables must exist (even if `bronze_meta_sec.*` stays empty) or the
# UNION query itself fails with `UNKNOWN_TABLE`, which is exactly what a
# bare compose stack (no `demo/clickhouse/04_registry.sql` applied) hit
# during P3 verification before this module created both.
EXPECTED_SCHEMAS: tuple[TableSchema, ...] = (
    *_registry_schemas("bronze_meta"),
    *_registry_schemas("bronze_meta_sec"),
)


def _ch_exec(target: ClickHouseTarget, statement: str) -> None:
    resp = requests.post(
        target.url,
        auth=(target.user, target.password),
        data=statement.encode("utf-8"),
        timeout=30,
    )
    resp.raise_for_status()


def _ch_query_json(target: ClickHouseTarget, statement: str) -> list[dict]:
    """Run `statement` (a `SELECT`) and return its rows as dicts, via
    `FORMAT JSON`."""
    resp = requests.post(
        target.url,
        auth=(target.user, target.password),
        data=(statement.rstrip().rstrip(";") + "\nFORMAT JSON").encode("utf-8"),
        timeout=30,
    )
    resp.raise_for_status()
    return resp.json().get("data", [])


def _assert_or_create_schema(target: ClickHouseTarget, schema: TableSchema) -> None:
    """R10: `schema` is the single source of truth for this table. If the
    table does not exist yet (a bare compose stack that never applied
    `demo/clickhouse/04_registry.sql`), create it. If it already exists
    (typically because `04_registry.sql` created it by hand), verify its
    ACTUAL columns/engine/sorting key match `schema` exactly and raise
    [`SchemaDriftError`] loudly if not — instead of silently trusting
    whatever `IF NOT EXISTS` left alone."""
    existing_tables = _ch_query_json(
        target,
        "SELECT engine, sorting_key FROM system.tables "
        f"WHERE database = 'lake' AND name = {_sql_string_literal(schema.table_name)}",
    )
    if not existing_tables:
        _ch_exec(target, schema.create_ddl)
        return

    actual_engine = existing_tables[0].get("engine", "")
    actual_sorting_key = existing_tables[0].get("sorting_key", "")
    expected_sorting_key = ", ".join(schema.order_by)
    if actual_engine != schema.engine or actual_sorting_key != expected_sorting_key:
        raise SchemaDriftError(
            f"R10 schema drift: lake.`{schema.table_name}` exists with engine="
            f"{actual_engine!r} sorting_key={actual_sorting_key!r}, but the "
            f"canonical schema (dagster/dispar_orchestrate/bronze_catalog.py) "
            f"expects engine={schema.engine!r} sorting_key={expected_sorting_key!r}. "
            "This table was likely created or altered outside this module "
            "(e.g. a stale demo/clickhouse/04_registry.sql) — reconcile the "
            "two before proceeding; the registry schema now has exactly one "
            "owner and this is that owner refusing to trust a mismatched table."
        )

    actual_columns = _ch_query_json(
        target,
        "SELECT name, type FROM system.columns "
        f"WHERE database = 'lake' AND table = {_sql_string_literal(schema.table_name)} "
        "ORDER BY position",
    )
    actual_pairs = tuple((c.get("name", ""), c.get("type", "")) for c in actual_columns)
    if actual_pairs != schema.columns:
        raise SchemaDriftError(
            f"R10 schema drift: lake.`{schema.table_name}` exists with columns "
            f"{actual_pairs!r}, but the canonical schema "
            f"(dagster/dispar_orchestrate/bronze_catalog.py) expects "
            f"{schema.columns!r}. This table was likely created or altered "
            "outside this module (e.g. a stale demo/clickhouse/04_registry.sql) "
            "— reconcile the two before proceeding; the registry schema now "
            "has exactly one owner and this is that owner refusing to trust "
            "a mismatched table."
        )


def _assert_or_create_all(target: ClickHouseTarget, schemas: tuple[TableSchema, ...]) -> None:
    _ch_exec(target, "CREATE DATABASE IF NOT EXISTS lake")
    for schema in schemas:
        _assert_or_create_schema(target, schema)


def _sql_string_literal(value: str) -> str:
    """Minimal single-quote escaping for a `ClickHouse` string literal.
    Every value this module inserts is either a fixed literal or a
    server-controlled table/slug name (never end-user input), so this is
    deliberately simple rather than a general-purpose SQL escaper."""
    return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"


def register_bronze_table(
    *,
    slug: str,
    title: str,
    description: str,
    bronze_table_name: str,
    row_count: int,
    author: str = "dagster",
    columns: list[tuple[str, str, str]] | None = None,
    target: ClickHouseTarget | None = None,
) -> None:
    """Upsert one Bronze dataset into the console catalog registry.

    `ReplacingMergeTree ORDER BY slug` (`dataset_catalog`/`dataset_sync`)
    means a re-run for the same `slug` after a later ingest (more rows)
    is a genuine upsert once ClickHouse merges parts, matching the
    existing tables' own replace semantics — not something this module
    invents.
    """
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, EXPECTED_SCHEMAS)

    updated_at = _utc_now_iso()
    catalog_values = (
        f"({_sql_string_literal(slug)}, {_sql_string_literal(title)}, "
        f"{_sql_string_literal(description)}, 'primer', "
        f"{_sql_string_literal(updated_at)}, {_sql_string_literal(bronze_table_name)})"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.dataset_catalog` "
        "(slug, title, description, tier, updated_at, table_name) VALUES "
        + catalog_values,
    )

    sync_values = (
        f"({_sql_string_literal(slug)}, {_sql_string_literal(title)}, "
        f"{_sql_string_literal(description)}, {_sql_string_literal(bronze_table_name)}, "
        f"{row_count}, {_sql_string_literal(author)}, 'harian', '', '')"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.dataset_sync` "
        "(slug, title, description, table_name, total, author, frekuensi, satuan, klasifikasi) "
        "VALUES " + sync_values,
    )

    if columns:
        col_values = ", ".join(
            f"({_sql_string_literal(slug)}, {_sql_string_literal(name)}, "
            f"{_sql_string_literal(dtype)}, {_sql_string_literal(desc)})"
            for name, dtype, desc in columns
        )
        _ch_exec(
            ch,
            "INSERT INTO lake.`bronze_meta.dataset_column` "
            "(slug, key_asli, tipe, deskripsi) VALUES " + col_values,
        )


def _utc_now_iso() -> str:
    from datetime import datetime, timezone

    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ── P4: maintenance-run metrics ─────────────────────────────────────────
#
# `lake.bronze_meta.maintenance_run` is a NEW table, introduced by P4's
# maintenance job (`dispar_orchestrate/maintenance.py`). It is NOT mirrored
# into `demo/clickhouse/04_registry.sql` — that file is out of scope for
# this build to edit — so this table has always had exactly one owner:
# `_MAINTENANCE_RUN_SCHEMA` above. It goes through the same R10
# `_assert_or_create_schema` path as the other three tables purely for
# consistency (and so a future manual/production copy of this table would
# also be caught if it ever drifts), not because a drift is currently
# possible here. A production deployment that applies
# `demo/clickhouse/*.sql` by hand and never runs this Dagster job will not
# have this table until either (a) the maintenance job runs once (creating
# it fresh, same as the other three tables' bootstrap story for a bare
# compose stack), or (b) a follow-up change ports this schema into
# `04_registry.sql` for production parity — noted here explicitly so it is
# a tracked gap, not a silent one.


def record_maintenance_run(
    *,
    table_name: str,
    dry_run_metrics: dict,
    applied_metrics: dict,
    skipped_verbs: list[str],
    target: "ClickHouseTarget | None" = None,
) -> None:
    """Upsert one maintenance run's dry-run + applied metrics into
    `lake.bronze_meta.maintenance_run` — the SAME registry mechanism
    (`lake.bronze_meta.*` via plain `INSERT`, read by
    `lakehouse-api::routes::governance::maintenance`) that
    `register_bronze_table` already uses for the dataset catalog, per the
    task brief's "reuse that mechanism; do not invent a parallel one."
    """
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, (_MAINTENANCE_RUN_SCHEMA,))

    run_at = _utc_now_iso()
    values = (
        f"({_sql_string_literal(table_name)}, {_sql_string_literal(run_at)}, "
        f"{int(dry_run_metrics.get('deleted_data_files_count', 0))}, "
        f"{int(dry_run_metrics.get('deleted_manifest_files_count', 0))}, "
        f"{int(dry_run_metrics.get('deleted_manifest_lists_count', 0))}, "
        f"{int(applied_metrics.get('deleted_data_files_count', 0))}, "
        f"{int(applied_metrics.get('deleted_manifest_files_count', 0))}, "
        f"{int(applied_metrics.get('deleted_manifest_lists_count', 0))}, "
        f"{_sql_string_literal('; '.join(skipped_verbs))})"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.maintenance_run` "
        "(table_name, run_at, dry_run_deleted_data_files, "
        "dry_run_deleted_manifest_files, dry_run_deleted_manifest_lists, "
        "applied_deleted_data_files, applied_deleted_manifest_files, "
        "applied_deleted_manifest_lists, skipped_verbs) VALUES " + values,
    )
