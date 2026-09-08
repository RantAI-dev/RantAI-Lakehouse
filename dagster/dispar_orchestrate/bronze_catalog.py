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
"""

from __future__ import annotations

import os
from dataclasses import dataclass

import requests


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


def _registry_ddl(prefix: str) -> tuple[str, ...]:
    return (
        f"CREATE TABLE IF NOT EXISTS lake.`{prefix}.dataset_catalog` ("
        "slug String, title String, description String, tier String, "
        "updated_at String, table_name String"
        ") ENGINE = ReplacingMergeTree ORDER BY slug",
        f"CREATE TABLE IF NOT EXISTS lake.`{prefix}.dataset_sync` ("
        "slug String, title String, description String, table_name String, "
        "total UInt64, author String, frekuensi String, satuan String, klasifikasi String"
        ") ENGINE = ReplacingMergeTree ORDER BY slug",
        f"CREATE TABLE IF NOT EXISTS lake.`{prefix}.dataset_column` ("
        "slug String, key_asli String, tipe String, deskripsi String"
        ") ENGINE = ReplacingMergeTree ORDER BY (slug, key_asli)",
    )


# Every `lakehouse-api` catalog/governance query (`routes::catalog`,
# `routes::governance::classification`/`lineage`) `UNION ALL`s
# `bronze_meta.*` with `bronze_meta_sec.*` unconditionally — so both sets
# of tables must exist (even if `bronze_meta_sec.*` stays empty) or the
# UNION query itself fails with `UNKNOWN_TABLE`, which is exactly what a
# bare compose stack (no `demo/clickhouse/04_registry.sql` applied) hit
# during P3 verification before this module created both.
_DDL = (
    "CREATE DATABASE IF NOT EXISTS lake",
    *_registry_ddl("bronze_meta"),
    *_registry_ddl("bronze_meta_sec"),
)


def _ch_exec(target: ClickHouseTarget, statement: str) -> None:
    resp = requests.post(
        target.url,
        auth=(target.user, target.password),
        data=statement.encode("utf-8"),
        timeout=30,
    )
    resp.raise_for_status()


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
    for ddl in _DDL:
        _ch_exec(ch, ddl)

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
# maintenance job (`dispar_orchestrate/maintenance.py`). Per R10 in
# `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` (the `bronze_meta.*` schema
# already being defined in two places — this file and
# `demo/clickhouse/04_registry.sql` — is a known drift risk), this table's
# DDL is defined in EXACTLY ONE place: here. It is deliberately NOT
# mirrored into `demo/clickhouse/04_registry.sql` — that file is out of
# scope for this phase's change set (see the P4 task brief's "do not
# touch" list), so mirroring it would mean editing a file this phase is
# not permitted to touch, or leaving a second copy to drift immediately.
# A production deployment that applies `demo/clickhouse/*.sql` by hand and
# never runs this Dagster job will not have this table until either (a)
# the maintenance job runs once (`IF NOT EXISTS` creates it, same as the
# three existing tables' bootstrap story for a bare compose stack), or (b)
# a follow-up change ports this DDL into `04_registry.sql` for production
# parity — noted here explicitly so it is a tracked gap, not a silent one.
_MAINTENANCE_RUN_DDL = (
    "CREATE TABLE IF NOT EXISTS lake.`bronze_meta.maintenance_run` ("
    "table_name String, "
    "run_at String, "
    "dry_run_deleted_data_files UInt64, "
    "dry_run_deleted_manifest_files UInt64, "
    "dry_run_deleted_manifest_lists UInt64, "
    "applied_deleted_data_files UInt64, "
    "applied_deleted_manifest_files UInt64, "
    "applied_deleted_manifest_lists UInt64, "
    "skipped_verbs String"
    ") ENGINE = ReplacingMergeTree ORDER BY (table_name, run_at)"
)


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
    _ch_exec(ch, "CREATE DATABASE IF NOT EXISTS lake")
    _ch_exec(ch, _MAINTENANCE_RUN_DDL)

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
