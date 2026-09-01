"""P5 Dagster job: Postgres replication-slot lag / WAL-retention metrics —
R5 in `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`'s risk register ("a stuck or
lagging replication slot pins WAL and fills the customer's production
database disk"), the most dangerous risk in the whole build. This job is
the first-class metrics surface the task brief requires, and it deliberately
reuses the EXACT mechanism `dispar_orchestrate.maintenance.record_maintenance_run`
already established in P4: a plain `INSERT` into a `lake.bronze_meta.*`
table, read back by `lakehouse-api::routes::governance` — not a parallel
registry, per R10 (the `bronze_meta.*` schema already has enough owners).

# What this measures, and why via SQL against the source, not Debezium

`pg_replication_slots` (queryable on ANY Postgres server with a slot, no
special privilege beyond `pg_monitor` or superuser) reports, per slot:
`active` (is a consumer currently connected), `restart_lsn` (the oldest WAL
Postgres must retain for this slot), and `confirmed_flush_lsn` (the newest
WAL position the consumer has acknowledged). `pg_current_wal_lsn() -
restart_lsn` (via `pg_wal_lsn_diff`) is exactly "how much WAL is being
pinned by this slot" — the quantity that fills a disk if a slot goes stale.
This is measured directly against the SOURCE Postgres server, not asked of
Debezium Server itself, because a customer's production database is the
actual thing at risk, and it already exposes this via a system view with no
extra component required — asking Debezium instead would mean trusting a
CDC connector's own self-report of a risk that manifests on a database it
does not control.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import psycopg2
import requests
from dagster import Definitions, ScheduleDefinition, job, op

from dispar_orchestrate.bronze_catalog import ClickHouseTarget


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def _ch_exec(target: ClickHouseTarget, statement: str) -> None:
    """Duplicated from `bronze_catalog._ch_exec` rather than imported: that
    function (and `_sql_string_literal`/`_utc_now_iso`, also duplicated
    below) are module-private by convention (leading underscore) — this
    module gets its own copies rather than reaching across that boundary,
    the same call `maintenance.py` already made for its own `_ch_query`."""
    resp = requests.post(
        target.url,
        auth=(target.user, target.password),
        data=statement.encode("utf-8"),
        timeout=30,
    )
    resp.raise_for_status()


def _sql_string_literal(value: str) -> str:
    return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"


def _utc_now_iso() -> str:
    from datetime import datetime, timezone

    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


@dataclass(frozen=True)
class ReplicationConfig:
    ch: ClickHouseTarget
    source_dsn: str

    @classmethod
    def from_env(cls) -> "ReplicationConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            # `postgresql://`, matching `BRONZE_SOURCE_DATABASE_URL`'s own
            # scheme convention (SQLAlchemy-compatible, set by
            # `docker-compose.yml`'s `dagster-code-location` service).
            source_dsn=_env(
                "BRONZE_SOURCE_DATABASE_URL",
                "postgresql://lakehouse:lakehouse@postgres:5432/lakehouse",
            ),
        )


# `lake.bronze_meta.replication_slot` is a NEW table, introduced by this P5
# job. Per R10, its DDL is defined in EXACTLY ONE place: here — mirroring
# `maintenance.py`'s `_MAINTENANCE_RUN_DDL` precedent exactly, including the
# same reason it is NOT mirrored into `demo/clickhouse/04_registry.sql`
# (out of scope for this phase's change set).
_REPLICATION_SLOT_DDL = (
    "CREATE TABLE IF NOT EXISTS lake.`bronze_meta.replication_slot` ("
    "connector_id String, "
    "slot_name String, "
    "checked_at String, "
    "active UInt8, "
    "wal_retained_bytes Int64, "
    "confirmed_flush_lag_bytes Int64, "
    "status String"
    ") ENGINE = ReplacingMergeTree ORDER BY (connector_id, checked_at)"
)

# Conservative, arbitrary thresholds — this is a first-class metric, not a
# tuned alert; an operator with real WAL volume data should override these
# via env vars rather than this job hardcoding a "correct" number nothing
# has measured yet (the same "don't guess a number, measure it" posture
# `docs/adr/0004-bronze-naming-partitioning-retention.md` took for
# retention).
_WARN_WAL_RETAINED_BYTES = int(_env("REPLICATION_SLOT_WARN_WAL_BYTES", str(500 * 1024 * 1024)))
_CRITICAL_WAL_RETAINED_BYTES = int(
    _env("REPLICATION_SLOT_CRITICAL_WAL_BYTES", str(2 * 1024 * 1024 * 1024))
)


def _status_for(wal_retained_bytes: int, active: bool) -> str:
    """A slot that is not `active` (no consumer connected) but still
    exists is ALREADY the dangerous state R5 describes — a disconnected
    Debezium Server still pins WAL at its last `restart_lsn` indefinitely,
    which is exactly why this checks `active` independently of the byte
    thresholds, not only as a tiebreaker."""
    if not active:
        return "critical"
    if wal_retained_bytes >= _CRITICAL_WAL_RETAINED_BYTES:
        return "critical"
    if wal_retained_bytes >= _WARN_WAL_RETAINED_BYTES:
        return "warning"
    return "ok"


def check_replication_slots(cfg: ReplicationConfig) -> list[dict[str, Any]]:
    """Query every logical replication slot on the source Postgres server.
    `connector_id` is derived from the slot name (this build's own naming
    convention, `<connector_slug>_slot` — see
    `lakehouse_store::cdc::render_debezium_properties`), not looked up
    against the `connector` table: this job runs against the SOURCE
    database directly and has no reason to depend on the console's own
    Postgres being reachable, matching `bronze_catalog.py`'s existing
    posture of talking to exactly the systems a check needs and no more."""
    results: list[dict[str, Any]] = []
    conn = psycopg2.connect(cfg.source_dsn)
    try:
        conn.set_session(readonly=True, autocommit=True)
        with conn.cursor() as cur:
            cur.execute(
                "SELECT slot_name, active, "
                "pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)::bigint AS wal_retained_bytes, "
                "COALESCE(pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn), 0)::bigint "
                "  AS confirmed_flush_lag_bytes "
                "FROM pg_replication_slots WHERE slot_type = 'logical'"
            )
            for slot_name, active, wal_retained_bytes, confirmed_flush_lag_bytes in cur.fetchall():
                connector_id = slot_name[: -len("_slot")] if slot_name.endswith("_slot") else slot_name
                results.append(
                    {
                        "connector_id": connector_id,
                        "slot_name": slot_name,
                        "active": bool(active),
                        "wal_retained_bytes": int(wal_retained_bytes),
                        "confirmed_flush_lag_bytes": int(confirmed_flush_lag_bytes),
                        "status": _status_for(int(wal_retained_bytes), bool(active)),
                    }
                )
    finally:
        conn.close()
    return results


def record_replication_slot_metrics(
    slots: list[dict[str, Any]], *, target: "ClickHouseTarget | None" = None
) -> None:
    """Upsert every checked slot's metrics into
    `lake.bronze_meta.replication_slot` — the SAME registry mechanism
    `record_maintenance_run` already uses, read by
    `lakehouse-api::routes::governance::replication`."""
    ch = target or ClickHouseTarget.from_env()
    _ch_exec(ch, "CREATE DATABASE IF NOT EXISTS lake")
    _ch_exec(ch, _REPLICATION_SLOT_DDL)

    if not slots:
        return

    checked_at = _utc_now_iso()
    values = ", ".join(
        f"({_sql_string_literal(s['connector_id'])}, {_sql_string_literal(s['slot_name'])}, "
        f"{_sql_string_literal(checked_at)}, {1 if s['active'] else 0}, "
        f"{int(s['wal_retained_bytes'])}, {int(s['confirmed_flush_lag_bytes'])}, "
        f"{_sql_string_literal(s['status'])})"
        for s in slots
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.replication_slot` "
        "(connector_id, slot_name, checked_at, active, wal_retained_bytes, "
        "confirmed_flush_lag_bytes, status) VALUES " + values,
    )


@op
def run_replication_slot_check(context) -> list[dict[str, Any]]:
    cfg = ReplicationConfig.from_env()
    slots = check_replication_slots(cfg)
    context.log.info(f"checked {len(slots)} replication slot(s): {slots}")
    for slot in slots:
        if slot["status"] != "ok":
            context.log.warning(
                f"replication slot {slot['slot_name']!r} is {slot['status']}: "
                f"wal_retained_bytes={slot['wal_retained_bytes']}, active={slot['active']} "
                "(R5: a stuck/lagging slot pins WAL and can fill the source database's disk)"
            )
    record_replication_slot_metrics(slots, target=cfg.ch)
    context.add_output_metadata({"slots_checked": len(slots)})
    return slots


@job
def replication_slot_check_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `replication_slot_check_job`."""
    run_replication_slot_check()


# Every 15 minutes — WAL can accumulate fast under write-heavy load, so this
# is deliberately much more frequent than the daily Bronze maintenance
# schedule; a stuck slot is a same-day operational emergency, not a
# maintenance-window concern.
replication_slot_check_schedule = ScheduleDefinition(
    job=replication_slot_check_job,
    cron_schedule="*/15 * * * *",
)

replication_defs = Definitions(
    jobs=[replication_slot_check_job],
    schedules=[replication_slot_check_schedule],
)
