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
        # ClickHouse 26.8 REFUSES `expire_snapshots` for catalog-backed
        # Iceberg tables (`Code: 48 ... not supported for Iceberg tables
        # backed by a transactional catalog` — deliberate: a REST catalog
        # should own snapshot expiry), and `remove_orphan_files` reclaims
        # unreferenced FILES, never accumulated SNAPSHOTS. Nothing in this
        # stack currently reclaims them (see
        # `dagster/dispar_orchestrate/maintenance.py`'s module doc for the
        # Lakekeeper Management API investigation that confirmed this is a
        # real, tracked gap, not an oversight). These three columns are
        # `maintenance.py::measure_snapshot_growth`'s per-run measurement of
        # that unbounded growth — snapshot/metadata-log entry counts read
        # straight from the Iceberg REST catalog's own table-metadata
        # document, so the growth is VISIBLE and trending in
        # `GET /api/governance/maintenance` history rather than silent.
        # `snapshot_growth_measured=0` distinguishes "measured, currently
        # zero" from "not measured this run" (e.g. a pre-R1/authz-disabled
        # stack with no `CH_OAUTH_CLIENT_ID` to mint a catalog read token
        # with) — a bare `0` in the count columns alone could not tell
        # those two apart.
        ("bronze_snapshot_count", "UInt64"),
        ("bronze_metadata_log_count", "UInt64"),
        ("snapshot_growth_measured", "UInt8"),
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


# ── Per-verb maintenance outcomes (WS2 §4 rework) ───────────────────────
#
# `_MAINTENANCE_RUN_SCHEMA` above and `record_maintenance_run` are left
# byte-identical by this rework, deliberately: `_assert_or_create_schema`'s
# R10 guard has no additive path — for an EXISTING table it requires the
# exact `(name, type)` column tuple to match, and raises `SchemaDriftError`
# otherwise — so adding columns to that already-deployed table would raise
# schema drift on every deployment's next scheduled run (the demo
# included) and stop maintenance entirely. Per-verb outcomes
# (`expire_snapshots`/`optimize`, whether they ran, were refused, skipped,
# or failed) go into this NEW table instead, which no existing deployment
# has yet, so `_assert_or_create_schema`'s "table does not exist yet"
# branch (create fresh) is always the one that fires.
_MAINTENANCE_VERB_RUN_SCHEMA = TableSchema(
    table_name="bronze_meta.maintenance_verb_run",
    columns=(
        ("table_name", "String"),
        ("run_at", "String"),
        ("verb", "String"),
        ("engine", "String"),
        ("outcome", "String"),
        ("detail", "String"),
    ),
    engine="ReplacingMergeTree",
    order_by=("table_name", "run_at", "verb"),
)


def latest_maintenance_run_at(target: ClickHouseTarget, table_name: str) -> str | None:
    """The most recent `run_at` `record_maintenance_run` wrote for
    `table_name` (`bronze_meta.maintenance_run.run_at`'s own
    `_utc_now_iso` string shape), or `None` if this table has never had a
    maintenance run recorded — including `bronze_meta.maintenance_run`
    not existing yet, the very first run on a fresh deployment.
    `maintenance.py`'s per-table schedule cadence gate treats `None` as
    "never run before", which always allows a run.
    """
    try:
        rows = _ch_query_json(
            target,
            "SELECT max(run_at) AS run_at FROM lake.`bronze_meta.maintenance_run` "
            f"WHERE table_name = {_sql_string_literal(table_name)}",
        )
    except requests.RequestException:
        # Most commonly `bronze_meta.maintenance_run` not existing yet (no
        # run has ever been recorded) or ClickHouse being unreachable —
        # both mean "no known last run", which the cadence gate treats
        # the same as "never run before" (always allowed), never a raise.
        return None
    if not rows:
        return None
    value = rows[0].get("run_at")
    return value or None


def record_maintenance_verb_run(
    *,
    table_name: str,
    run_at: str,
    verb_runs: list[dict[str, str]],
    target: "ClickHouseTarget | None" = None,
) -> None:
    """Records this run's per-verb outcomes for `table_name` into the NEW
    `lake.bronze_meta.maintenance_verb_run` table (see the section comment
    above for why this is a new table rather than new columns on
    `_MAINTENANCE_RUN_SCHEMA`), via the same `_assert_or_create_all` R10
    path every other table in this module goes through.

    `verb_runs` is a list of
    `{"verb": ..., "engine": ..., "outcome": ..., "detail": ...}` dicts,
    `outcome` one of `applied`, `refused`, `skipped` or `failed`. Does
    nothing (no `INSERT`, no schema check) when `verb_runs` is empty — a
    table with no configured policy has nothing to record here.
    """
    if not verb_runs:
        return
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, (_MAINTENANCE_VERB_RUN_SCHEMA,))

    values = ", ".join(
        f"({_sql_string_literal(table_name)}, {_sql_string_literal(run_at)}, "
        f"{_sql_string_literal(v['verb'])}, {_sql_string_literal(v['engine'])}, "
        f"{_sql_string_literal(v['outcome'])}, {_sql_string_literal(v['detail'])})"
        for v in verb_runs
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.maintenance_verb_run` "
        "(table_name, run_at, verb, engine, outcome, detail) VALUES " + values,
    )


# ── P6: bucket capacity snapshot ────────────────────────────────────────
#
# `_CAPACITY_SNAPSHOT_SCHEMA` is a NEW table (`capacity_snapshot.py`'s
# daily bucket-size job), so — like `_MAINTENANCE_RUN_SCHEMA` and
# `_MAINTENANCE_VERB_RUN_SCHEMA` above — it always takes
# `_assert_or_create_schema`'s "table does not exist yet" branch (create
# fresh) on every deployment, never the additive path that does not
# exist. `TableSchema.create_ddl` always emits `lake.\`{table_name}\``,
# so `table_name` is one identifier that may contain a dot, always
# inside `lake` — `bronze_meta.capacity_snapshot`, following the same
# convention as `bronze_meta.maintenance_run` above, not a separate
# `console` database (which `_assert_or_create_all` never creates).
# `clickhouse_bytes_on_disk` is deliberately not a column here:
# `GET /api/lakehouse/capacity` reads ClickHouse's own `system.parts`
# live, so this table stores only the numbers it is the sole source
# for.
_CAPACITY_SNAPSHOT_SCHEMA = TableSchema(
    table_name="bronze_meta.capacity_snapshot",
    columns=(
        ("measured_at", "DateTime64(3, 'UTC')"),
        ("bucket_name", "String"),
        ("bytes", "UInt64"),
        ("objects", "UInt64"),
    ),
    engine="ReplacingMergeTree",
    order_by=("bucket_name", "measured_at"),
)


def _utc_now_ch_timestamp() -> str:
    """A `DateTime64(3, 'UTC')`-parseable literal with millisecond
    precision, e.g. `2026-09-13 12:34:56.789` — no explicit zone suffix,
    because the column's own declared type already fixes it to UTC."""
    from datetime import datetime, timezone

    now = datetime.now(timezone.utc)
    return now.strftime("%Y-%m-%d %H:%M:%S.") + f"{now.microsecond // 1000:03d}"


def record_capacity_snapshot(
    *,
    bucket_name: str,
    bytes_: int,
    objects: int,
    target: "ClickHouseTarget | None" = None,
) -> None:
    """Insert one row into `lake.bronze_meta.capacity_snapshot`
    (`_CAPACITY_SNAPSHOT_SCHEMA` above), via the same `_assert_or_create_all`
    R10 path every other table in this module goes through — the writer
    lives beside its schema and its sibling recorders
    (`record_maintenance_run`, `record_maintenance_verb_run`) so the
    registry schema keeps exactly one owner for both its definition and
    its write.

    One row per run — `ReplacingMergeTree ORDER BY (bucket_name,
    measured_at)` means two rows in the same millisecond for the same
    bucket would collide, which cannot happen for a job invoked at most
    once a day. Called from `capacity_snapshot.py`'s daily bucket-size job.
    """
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, (_CAPACITY_SNAPSHOT_SCHEMA,))
    values = (
        f"({_sql_string_literal(_utc_now_ch_timestamp())}, "
        f"{_sql_string_literal(bucket_name)}, {int(bytes_)}, {int(objects)})"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.capacity_snapshot` "
        "(measured_at, bucket_name, bytes, objects) VALUES " + values,
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
    snapshot_growth: dict | None = None,
    target: "ClickHouseTarget | None" = None,
) -> None:
    """Upsert one maintenance run's dry-run + applied metrics into
    `lake.bronze_meta.maintenance_run` — the SAME registry mechanism
    (`lake.bronze_meta.*` via plain `INSERT`, read by
    `lakehouse-api::routes::governance::maintenance`) that
    `register_bronze_table` already uses for the dataset catalog, per the
    task brief's "reuse that mechanism; do not invent a parallel one."

    `snapshot_growth` is `maintenance.py::measure_snapshot_growth`'s output
    shape (`{"measured": bool, "snapshot_count": int,
    "metadata_log_count": int}`) — optional and defaulting to "not
    measured" so any OTHER future caller of this function does not have to
    know about the snapshot-growth gap-tracking columns to keep working.
    """
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, (_MAINTENANCE_RUN_SCHEMA,))

    growth = snapshot_growth or {}
    run_at = _utc_now_iso()
    values = (
        f"({_sql_string_literal(table_name)}, {_sql_string_literal(run_at)}, "
        f"{int(dry_run_metrics.get('deleted_data_files_count', 0))}, "
        f"{int(dry_run_metrics.get('deleted_manifest_files_count', 0))}, "
        f"{int(dry_run_metrics.get('deleted_manifest_lists_count', 0))}, "
        f"{int(applied_metrics.get('deleted_data_files_count', 0))}, "
        f"{int(applied_metrics.get('deleted_manifest_files_count', 0))}, "
        f"{int(applied_metrics.get('deleted_manifest_lists_count', 0))}, "
        f"{_sql_string_literal('; '.join(skipped_verbs))}, "
        f"{int(growth.get('snapshot_count', 0))}, "
        f"{int(growth.get('metadata_log_count', 0))}, "
        f"{1 if growth.get('measured') else 0})"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.maintenance_run` "
        "(table_name, run_at, dry_run_deleted_data_files, "
        "dry_run_deleted_manifest_files, dry_run_deleted_manifest_lists, "
        "applied_deleted_data_files, applied_deleted_manifest_files, "
        "applied_deleted_manifest_lists, skipped_verbs, "
        "bronze_snapshot_count, bronze_metadata_log_count, "
        "snapshot_growth_measured) VALUES " + values,
    )


# ── Ingest-run outcomes (WS3 plan review Z2, Z9) ────────────────────────
#
# `lake.bronze_meta.ingest_run` is a NEW table, introduced for
# `ingest_factory.py`'s per-connector ingest jobs. Same story as
# `_MAINTENANCE_RUN_SCHEMA` above: not
# mirrored into `demo/clickhouse/04_registry.sql` (out of scope for this
# build to edit), so this table has exactly one owner, `_INGEST_RUN_SCHEMA`
# below -- deliberately its OWN one-element tuple passed to
# `_assert_or_create_all`, the same pattern `record_maintenance_run` uses,
# NOT folded into the top-level `EXPECTED_SCHEMAS` tuple (that tuple holds
# only the three `dataset_catalog`/`dataset_sync`/`dataset_column`
# registry tables `register_bronze_table` writes; the grand plan's "via
# `EXPECTED_SCHEMAS`" names the single-canonical-schema-per-table
# MECHANISM `_assert_or_create_schema` enforces, not a literal shared
# tuple every writer must append to).
_INGEST_RUN_SCHEMA = TableSchema(
    table_name="bronze_meta.ingest_run",
    columns=(
        ("connector_id", "String"),
        ("job", "String"),
        ("object", "String"),
        ("rows", "Nullable(UInt64)"),  # WS3 plan review Z9: NULL means "not measured", never a fabricated 0
        ("started_at", "String"),
        ("ended_at", "String"),
        ("status", "String"),
        ("error", "String"),
    ),
    engine="ReplacingMergeTree",
    order_by=("connector_id", "job", "started_at"),
)


def record_ingest_run(
    *,
    connector_id: str,
    job: str,
    object_name: str,
    rows: int | None,
    started_at: str,
    ended_at: str,
    status: str,
    error: str = "",
    target: "ClickHouseTarget | None" = None,
) -> None:
    """Upsert one ingest run's outcome into `lake.bronze_meta.ingest_run`
    -- the SAME `_assert_or_create_schema` R10 pattern
    `record_maintenance_run` (`:377-425` at the time of this task's plan)
    uses for `_MAINTENANCE_RUN_SCHEMA`, not the top-level
    `EXPECTED_SCHEMAS` tuple (see this section's header comment).

    `rows=None` (WS3 plan review Z9) writes SQL `NULL` -- the real row
    count comes from `adapters/sink.py`'s
    `pipeline.last_trace.last_normalize_info.row_counts` (dlt's own
    normalize metrics), which is genuinely absent for an outcome where no
    load was attempted at all (e.g. an SSRF-blocked or
    unsupported-column-type rejection before `dlt` ever ran) -- recording
    `0` there would claim "zero rows loaded," a specific, false,
    measured-sounding number AGENTS.md's "never fabricate" rule (principle
    2) forbids. `NULL` says "not measured," which is what actually
    happened.
    """
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, (_INGEST_RUN_SCHEMA,))
    rows_literal = "NULL" if rows is None else str(int(rows))
    values = (
        f"({_sql_string_literal(connector_id)}, {_sql_string_literal(job)}, "
        f"{_sql_string_literal(object_name)}, {rows_literal}, "
        f"{_sql_string_literal(started_at)}, {_sql_string_literal(ended_at)}, "
        f"{_sql_string_literal(status)}, {_sql_string_literal(error)})"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.ingest_run` "
        "(connector_id, job, object, rows, started_at, ended_at, status, error) "
        "VALUES " + values,
    )
