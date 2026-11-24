"""The latest-copy op: one Iceberg table per published Gold mart that
always holds only the newest copy (DATA-6).

Why this module exists at all (M1): the Rust Iceberg library the export
writes `gold.<table>` with cannot replace a table's contents — it offers
only `fast_append` on a `Transaction`, the action trait a replace would
implement is `pub(crate)`, and so is the commit builder. PyIceberg's
`Transaction.overwrite` stages the delete and the append and sends them
in ONE catalog commit — measured by the acceptance gate on the real
stack (`DATA-6` T6, AC2: the metadata log grew by exactly 1 and a
continuous reader saw only whole copies). So the latest-copy table is
written here, by the orchestrator, never by the API.

Who launches it (D6): the API, from `routes::gold::export` — the one
place every publish goes through — with the publish's own facts in the
run config (D9): the mart, the table, the ONE `_exported_at` stamp the
publish wrote on every row, and the row count. Reading the rows back out
of `gold.<table>` (never out of `ClickHouse`) keeps the copy exactly
what the publish wrote, masking included.

How the copy is kept honest:

- D8: the table is created WITH its first copy in one commit, so it
  never exists empty; its columns must match `gold.<table>`'s, or the op
  fails `schema_incompatible` and writes nothing — there is no schema
  evolution here, the same as the export itself.
- D7: the table carries the `rantai.latest-copy-of` marker, and the op
  refuses to write a `gold.<table>_latest` that exists without it or
  whose marker names another table (`name_taken`) — the Rust side
  refuses the other direction (a mart publishing INTO a latest table),
  so a mart named `<other>_latest` and the latest table of another mart
  can never fight over one Iceberg table.
- D9: a run that executes out of order is `superseded` — the table never
  moves backwards — and a row count that does not match the publish's
  (0 included: the trim may already have removed that copy) fails
  `copy_incomplete`.
- D10: the publishing switch is checked when the run executes, not when
  it was launched — a mart switched off after the launch is skipped
  (`publishing_off`), and an unreadable publication list fails closed
  (`publication_state_unavailable`).
- D11: after a successful replace, every snapshot but the current one is
  expired and the orphan files removed (the trim's own two helpers, T3),
  so the table's storage does not grow one copy per publish; a failed
  cleanup is recorded in its own `cleanup` column and never fails the
  copy, which WAS replaced. Two fixed values say when no cleanup was due:
  `already_present` (an idempotent rerun) and `reload_failed` (the
  committed table could not be read back — DATA-6 review R2).
- D12: every run writes exactly one row to
  `console.gold_latest_copy_run` — the same DDL the API's
  `gold_latest_copy.rs` holds, identical text — with a `status` from a
  closed set and a `reason` from a closed set of fixed strings, never
  exception text. The exception detail goes to the run log only.
- D16: the job is serial (one run at a time; runs queue), so two updates
  of the same table never race. Only the API launches it: no schedule,
  no sensor, for the same reason `discover_job` has none — there is no
  such thing as "the" publish to run it on without a run config.
"""

from __future__ import annotations

import datetime
from dataclasses import dataclass
from typing import Any

from dagster import Failure, job, op

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, _ch_exec
from dispar_orchestrate.gold_export import (
    INVALID_ORPHAN_MIN_AGE_REASON,
    GoldExportConfig,
    InvalidOrphanMinAgeError,
    _enabled_publication_marts,
    _sql_dt64,
    _sql_num,
    _sql_str,
    expire_all_but_current_snapshot,
    load_gold_catalog,
    parse_gold_orphan_min_age_seconds,
    remove_gold_orphan_files,
)
from dispar_orchestrate.ingest_factory import job_tags
from dispar_orchestrate.maintenance import maintenance_paused
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

# DATA-6 D1/D7: the mirror of `lakehouse_iceberg::gold`'s
# `LATEST_TABLE_SUFFIX`/`LATEST_COPY_OF_PROPERTY` (that module's doc
# comment names this one back). The Rust refusal and this module's write
# must agree on the same strings, so both sides pin them in tests.
LATEST_TABLE_SUFFIX = "_latest"
LATEST_COPY_OF_PROPERTY = "rantai.latest-copy-of"

# The closed reason sets (D12, section 6 of the plan). Every recorded
# `reason` is one of these fixed strings, never exception text.
REASON_NOT_STARTED = "not_started"  # written by the API, not this module
REASON_INVALID_REQUEST = "invalid_request"
REASON_PUBLICATION_STATE_UNAVAILABLE = "publication_state_unavailable"
REASON_PUBLISHING_OFF = "publishing_off"
REASON_CATALOG_UNAVAILABLE = "catalog_unavailable"
REASON_NAME_TAKEN = "name_taken"
REASON_SCHEMA_INCOMPATIBLE = "schema_incompatible"
REASON_NEWER_COPY_PRESENT = "newer_copy_present"
REASON_COPY_INCOMPLETE = "copy_incomplete"
REASON_COMMIT_CONFLICT = "commit_conflict"
REASON_WRITE_FAILED = "write_failed"

# DATA-6 review R2: the `cleanup` column says why no cleanup ran when the
# replace had already committed but its post-commit snapshot read failed,
# or when the requested copy was already in place. Fixed strings, never
# exception text (D12).
CLEANUP_RELOAD_FAILED = "reload_failed"
CLEANUP_ALREADY_PRESENT = "already_present"

# `failed` reasons a retry can heal; every other failure is a fact about
# the request or the deployment that a retry would re-meet.
_RETRYABLE_REASONS = frozenset(
    {REASON_COMMIT_CONFLICT, REASON_CATALOG_UNAVAILABLE, REASON_PUBLICATION_STATE_UNAVAILABLE}
)


class InvalidLatestCopyRequest(ValueError):
    """Raised when the run config does not name one publish's copy
    (section 6's `invalid_request` row: a missing field, `rows < 1`,
    `copy_exported_at_micros < 1`, or a `latest_table` that is not the
    table name plus `_latest`). The message is for the run log; the
    recorded reason is the fixed `invalid_request`."""


def _sql_dt64_micros(us: int | None) -> str:
    """`Nullable(DateTime64(6))` from epoch microseconds, or `NULL`."""
    if us is None:
        return "NULL"
    return f"fromUnixTimestamp64Micro({int(us)})"


def ensure_gold_latest_copy_run_table(target: ClickHouseTarget) -> None:
    """Ensure `console.gold_latest_copy_run` exists (DATA-6 D12).

    The DDL is byte-identical to the Rust writer's
    (`lakehouse_api::gold_latest_copy::ensure_gold_latest_copy_run_table`):
    two writers, one table shape, no schema owner on either side that the
    other could drift from.
    """
    _ch_exec(target, "CREATE DATABASE IF NOT EXISTS console")
    _ch_exec(
        target,
        """CREATE TABLE IF NOT EXISTS console.gold_latest_copy_run (
            id String,
            mart String,
            latest_table String,
            status String,
            reason Nullable(String),
            copy_exported_at Nullable(DateTime64(6)),
            rows Nullable(UInt64),
            snapshot_id Nullable(Int64),
            cleanup Nullable(String),
            started_at DateTime64(3),
            finished_at DateTime64(3)
        ) ENGINE = MergeTree ORDER BY (mart, started_at)""",
    )


def record_gold_latest_copy_run(
    target: ClickHouseTarget | None,
    *,
    mart: str,
    latest_table: str,
    status: str,
    reason: str | None = None,
    copy_exported_at_micros: int | None = None,
    rows: int | None = None,
    snapshot_id: int | None = None,
    cleanup: str | None = None,
    started_at: Any = None,
    finished_at: Any = None,
) -> None:
    """Record one latest-copy run into `console.gold_latest_copy_run`
    (DATA-6 D12), modelled on `gold_export.record_gold_trim_run`."""
    if target is None:
        return
    ensure_gold_latest_copy_run_table(target)
    import uuid

    run_id = str(uuid.uuid4())
    started_at = started_at or datetime.datetime.now(datetime.timezone.utc)
    finished_at = finished_at or datetime.datetime.now(datetime.timezone.utc)
    sql = (
        f"INSERT INTO console.gold_latest_copy_run ("
        f"id, mart, latest_table, status, reason, copy_exported_at, rows, "
        f"snapshot_id, cleanup, started_at, finished_at) VALUES ("
        f"{_sql_str(run_id)}, {_sql_str(mart)}, {_sql_str(latest_table)}, "
        f"{_sql_str(status)}, {_sql_str(reason)}, "
        f"{_sql_dt64_micros(copy_exported_at_micros)}, {_sql_num(rows)}, "
        f"{_sql_num(snapshot_id)}, {_sql_str(cleanup)}, "
        f"{_sql_dt64(started_at)}, {_sql_dt64(finished_at)})"
    )
    _ch_exec(target, sql)


@dataclass(frozen=True)
class LatestCopyRequest:
    """One publish's copy, as the API named it in the run config (D9)."""

    mart: str
    table: str
    latest_table: str
    copy_exported_at_micros: int
    rows: int

    @classmethod
    def from_op_config(cls, config: dict) -> "LatestCopyRequest":
        """Validate the run config against section 6's `invalid_request`
        row, raising `InvalidLatestCopyRequest` (its message names the
        rule for the run log; the recorded reason stays the fixed
        `invalid_request`). Dagster's own `config_schema` already types
        the five keys; this re-checks everything so the rules are also
        unit-testable without a Dagster run.
        """
        if not isinstance(config, dict):
            raise InvalidLatestCopyRequest("the run config must be an object")
        for key in ("mart", "table", "latest_table", "copy_exported_at_micros", "rows"):
            if key not in config:
                raise InvalidLatestCopyRequest(f"missing required field: {key}")
        mart = config["mart"]
        table = config["table"]
        latest_table = config["latest_table"]
        if not all(isinstance(v, str) for v in (mart, table, latest_table)):
            raise InvalidLatestCopyRequest("mart, table and latest_table must be strings")
        if latest_table != table + LATEST_TABLE_SUFFIX:
            raise InvalidLatestCopyRequest(
                "latest_table must be the published table's name plus "
                f"{LATEST_TABLE_SUFFIX!r}"
            )
        micros = config["copy_exported_at_micros"]
        rows = config["rows"]
        if not isinstance(micros, int) or isinstance(micros, bool) or micros < 1:
            raise InvalidLatestCopyRequest("copy_exported_at_micros must be an integer >= 1")
        if not isinstance(rows, int) or isinstance(rows, bool) or rows < 1:
            raise InvalidLatestCopyRequest("rows must be an integer >= 1")
        return cls(
            mart=mart, table=table, latest_table=latest_table,
            copy_exported_at_micros=micros, rows=rows,
        )


def _columns(schema: Any) -> list[tuple[str, str, bool]]:
    """The schema as (name, type, required) triples — the identity D8
    compares: same names, types and required flags, in order. Anything
    else is `schema_incompatible`, never evolved on the fly (this module
    has no schema evolution, the same as the Rust export)."""
    return [(f.name, str(f.field_type), f.required) for f in schema.fields]


def _exported_at_as_micros(value: Any) -> int:
    """A `_exported_at` cell (PyIceberg reads a `Timestamp` column as a
    naive UTC datetime) as exact epoch microseconds. Integer timedelta
    arithmetic, never `timestamp()` float seconds: a float loses exact
    microseconds at current epoch magnitudes, and the superseded
    comparison (D9) must not be off by a microsecond.
    """
    if not isinstance(value, datetime.datetime):
        raise TypeError(f"_exported_at must be a datetime, got {type(value).__name__}")
    if value.tzinfo is None:
        value = value.replace(tzinfo=datetime.timezone.utc)
    delta = value - datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
    return delta // datetime.timedelta(microseconds=1)


def refresh_latest_copy(
    cfg: GoldExportConfig, req: LatestCopyRequest, log: Any = None
) -> dict[str, Any]:
    """Replace `gold.{latest_table}` with exactly this publish's copy of
    `gold.{table}`, in one catalog commit. Every exit records exactly one
    row (D12) and returns a `{"status": ..., "reason": ...}` dict the op
    turns into a run outcome. No network in the unit tests: this function
    only calls helpers its caller can fake.

    The step order is the plan's, and the early exits are what keep the
    table honest: a mart that is switched off (D10), a table name that
    is taken by someone else's latest copy (D7), a schema that drifted
    (D8), a run that is out of order (D9) and a copy that is not whole
    (D9) all leave the table untouched.
    """
    started_at = datetime.datetime.now(datetime.timezone.utc)

    def record(status: str, reason: str | None = None, **facts: Any) -> None:
        """One row per exit; a failed record write is logged and never
        raised — the record is a history view, not a correctness
        dependency of the copy (the same stance as the trim)."""
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        try:
            record_gold_latest_copy_run(
                cfg.ch,
                mart=req.mart,
                latest_table=req.latest_table,
                status=status,
                reason=reason,
                started_at=started_at,
                finished_at=finished_at,
                **facts,
            )
        except Exception as exc:  # noqa: BLE001 - the trim's own posture
            if log:
                log.warning(
                    f"failed to record gold_latest_copy_run for {req.mart}: {exc}"
                )

    # 1. Publishing on? (D10) — checked when the run executes, not when
    # the API launched it, so switching publishing off stops both tables.
    try:
        enabled = _enabled_publication_marts(cfg)
    except Exception as exc:  # noqa: BLE001 - fixed reason, detail to the log
        if log:
            log.error(f"the publication list could not be read: {exc}")
        record("failed", REASON_PUBLICATION_STATE_UNAVAILABLE)
        return {"status": "failed", "reason": REASON_PUBLICATION_STATE_UNAVAILABLE}
    if req.mart not in enabled and req.mart not in cfg.marts:
        record("skipped", REASON_PUBLISHING_OFF)
        return {"status": "skipped", "reason": REASON_PUBLISHING_OFF}

    # 2. The catalog and the published table (a copy the trim already
    # removed fails here-or-below, honestly, not as a fake success).
    try:
        catalog = load_gold_catalog(cfg)
        main = catalog.load_table(f"gold.{req.table}")
    except Exception as exc:  # noqa: BLE001
        if log:
            log.error(f"failed to load gold.{req.table}: {exc}")
        record("failed", REASON_CATALOG_UNAVAILABLE)
        return {"status": "failed", "reason": REASON_CATALOG_UNAVAILABLE}

    # 3. The latest table, if it exists yet.
    from pyiceberg.exceptions import NoSuchTableError

    latest: Any = None
    try:
        latest = catalog.load_table(f"gold.{req.latest_table}")
    except NoSuchTableError:
        latest = None
    except Exception as exc:  # noqa: BLE001
        if log:
            log.error(f"failed to load gold.{req.latest_table}: {exc}")
        record("failed", REASON_CATALOG_UNAVAILABLE)
        return {"status": "failed", "reason": REASON_CATALOG_UNAVAILABLE}

    # 4. Fail closed both ways around the marker (D7) ...
    if latest is not None:
        if latest.properties.get(LATEST_COPY_OF_PROPERTY) != req.table:
            if log:
                log.error(
                    f"gold.{req.latest_table} exists without the marker of "
                    f"gold.{req.table}: it belongs to someone else"
                )
            record("failed", REASON_NAME_TAKEN)
            return {"status": "failed", "reason": REASON_NAME_TAKEN}
        # ... and refuse a schema this module cannot write as-is (D8).
        if _columns(latest.schema()) != _columns(main.schema()):
            if log:
                log.error(
                    f"gold.{req.latest_table} columns differ from gold.{req.table}"
                )
            record("failed", REASON_SCHEMA_INCOMPATIBLE)
            return {"status": "failed", "reason": REASON_SCHEMA_INCOMPATIBLE}
        # Never backwards (D9): the latest table holds one whole copy by
        # construction, so any row's stamp is the held copy's stamp.
        # DATA-6 review R3: the scan is a catalog/object-store read and can
        # throw; with no record row D12 would be broken. Wrap it in the
        # fixed-reason pattern, detail to the log only.
        try:
            held = latest.scan(selected_fields=("_exported_at",), limit=1).to_arrow()
        except Exception as exc:  # noqa: BLE001
            if log:
                log.error(f"the held copy of gold.{req.latest_table} could not be read: {exc}")
            record("failed", REASON_CATALOG_UNAVAILABLE)
            return {"status": "failed", "reason": REASON_CATALOG_UNAVAILABLE}
        if held.num_rows:
            held_micros = _exported_at_as_micros(held.column("_exported_at").to_pylist()[0])
            if held_micros > req.copy_exported_at_micros:
                record("superseded", REASON_NEWER_COPY_PRESENT)
                return {"status": "superseded", "reason": REASON_NEWER_COPY_PRESENT}
            if held_micros == req.copy_exported_at_micros:
                # DATA-6 review R2: this exact copy is already in place —
                # an idempotent rerun, not a move backwards. Record the
                # success with what is held; no second write and no cleanup
                # are owed, and the row says so.
                record(
                    "success",
                    copy_exported_at_micros=req.copy_exported_at_micros,
                    rows=req.rows,
                    cleanup=CLEANUP_ALREADY_PRESENT,
                )
                return {"status": "success", "cleanup": CLEANUP_ALREADY_PRESENT}

    # 5. Read exactly this publish's rows. The int-micros literal filters
    # the Timestamp column directly — proved by the acceptance gate (T6,
    # AC1), which compares the held stamp in microseconds through
    # ClickHouse.
    from pyiceberg.expressions import EqualTo

    # DATA-6 review R3: this second scan is also outside the write's
    # try/except. An object-store error here must still record exactly one
    # row (D12), with a fixed reason and the exception detail in the log.
    try:
        df = main.scan(
            row_filter=EqualTo("_exported_at", req.copy_exported_at_micros)
        ).to_arrow()
    except Exception as exc:  # noqa: BLE001
        if log:
            log.error(f"the copy of gold.{req.table} could not be read: {exc}")
        record("failed", REASON_CATALOG_UNAVAILABLE)
        return {"status": "failed", "reason": REASON_CATALOG_UNAVAILABLE}
    if df.num_rows != req.rows:
        if log:
            log.error(
                f"gold.{req.table} holds {df.num_rows} rows for this publish's "
                f"_exported_at, the publish wrote {req.rows}"
            )
        record("failed", REASON_COPY_INCOMPLETE)
        return {"status": "failed", "reason": REASON_COPY_INCOMPLETE}

    # 6. Write, in ONE commit (D3): create-with-rows for the first copy
    # (the table never exists empty, D8), one overwrite after that.
    # Never a separate delete+append: that would be two commits and a
    # reader could see the gap between them.
    from pyiceberg.exceptions import CommitFailedException, TableAlreadyExistsError

    try:
        if latest is None:
            with catalog.create_table_transaction(
                f"gold.{req.latest_table}",
                schema=main.schema(),
                properties={
                    LATEST_COPY_OF_PROPERTY: req.table,
                    "format-version": "2",
                },
            ) as tx:
                tx.append(df)
        else:
            with latest.transaction() as tx:
                tx.overwrite(df)
    except (CommitFailedException, TableAlreadyExistsError) as exc:
        if log:
            log.error(f"the one-commit write of gold.{req.latest_table} conflicted: {exc}")
        record("failed", REASON_COMMIT_CONFLICT)
        return {"status": "failed", "reason": REASON_COMMIT_CONFLICT}
    except Exception as exc:  # noqa: BLE001 - fixed reason, detail to the log
        if log:
            log.error(f"the one-commit write of gold.{req.latest_table} failed: {exc}")
        record("failed", REASON_WRITE_FAILED)
        return {"status": "failed", "reason": REASON_WRITE_FAILED}

    # 7. The committed snapshot id, off a freshly loaded table. DATA-6
    # review R2: the write has ALREADY committed here. A reload failure can
    # never be recorded as a failed replace — the retry would then see the
    # copy held and report `superseded`, while the card still showed the
    # previous copy. Record the success with what is known, skip the
    # cleanup (there is no table handle to clean) and say so in the row.
    snapshot_id: int | None = None
    try:
        latest = catalog.load_table(f"gold.{req.latest_table}")
        current = latest.current_snapshot()
        snapshot_id = current.snapshot_id if current else None
    except Exception as exc:  # noqa: BLE001
        if log:
            log.error(f"gold.{req.latest_table} could not be reloaded after the commit: {exc}")
        record(
            "success",
            copy_exported_at_micros=req.copy_exported_at_micros,
            rows=req.rows,
            snapshot_id=None,
            cleanup=CLEANUP_RELOAD_FAILED,
        )
        return {"status": "success", "cleanup": CLEANUP_RELOAD_FAILED}

    # 8. Cleanup (D11): expire every older snapshot, then remove the
    # orphan files (the trim's own helpers, T3). A failed cleanup never
    # fails the copy — the copy WAS replaced — it is recorded in its own
    # `cleanup` column, and each failure stops the cleanup there.
    cleanup: str | None = None
    if maintenance_paused():
        cleanup = "maintenance_paused"
        if log:
            log.info(
                "maintenance is paused (MAINTENANCE_PAUSED=true): no snapshot was "
                "expired and no orphan file was removed."
            )
    else:
        try:
            orphan_age = parse_gold_orphan_min_age_seconds(cfg.gold_orphan_min_age_seconds)
        except InvalidOrphanMinAgeError:
            cleanup = INVALID_ORPHAN_MIN_AGE_REASON
        if cleanup is None:
            try:
                expire_all_but_current_snapshot(latest)
            except Exception as exc:  # noqa: BLE001
                if log:
                    log.error(f"failed to expire snapshots of gold.{req.latest_table}: {exc}")
                cleanup = "expire_snapshots_failed"
        if cleanup is None:
            try:
                remove_gold_orphan_files(
                    cfg, f"gold.{req.latest_table}", orphan_age
                )
            except Exception as exc:  # noqa: BLE001
                if log:
                    log.error(f"failed to remove orphan files of gold.{req.latest_table}: {exc}")
                cleanup = "orphan_cleanup_failed"

    # 9. Success, with the copy's own facts (D12).
    record(
        "success",
        copy_exported_at_micros=req.copy_exported_at_micros,
        rows=req.rows,
        snapshot_id=snapshot_id,
        cleanup=cleanup,
    )
    return {"status": "success", "cleanup": cleanup}


@op(
    config_schema={
        "mart": str,
        "table": str,
        "latest_table": str,
        "copy_exported_at_micros": int,
        "rows": int,
    },
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_latest_copy.py::refresh_gold_latest_copy",
        reads=[
            "Iceberg gold.{table} (one copy, by _exported_at)",
            "GET /api/gold/publications",
        ],
        writes=[
            "Iceberg gold.{table}_latest (replaced in one commit)",
            "ClickHouse console.gold_latest_copy_run",
        ],
    ),
)
def refresh_gold_latest_copy(context) -> dict:
    """Replace this mart's latest-copy table with the one publish the API
    named in the run config. An invalid config records one
    `failed`/`invalid_request` row with what is known and fails without
    retry; a `failed` outcome raises `dagster.Failure` with the fixed
    reason (retryable only for the three transient reasons), so the run
    shows as failed rather than green; `skipped`/`superseded`/`success`
    return normally."""
    cfg = GoldExportConfig.from_env()
    log = context.log
    config = context.op_config or {}
    try:
        req = LatestCopyRequest.from_op_config(config)
    except InvalidLatestCopyRequest as exc:
        if log:
            log.error(f"invalid run config for the latest-copy op: {exc}")
        # Recorded with what is known (the two name columns are String,
        # NOT NULL): a field that is absent or wrongly typed is honestly
        # named rather than guessed.
        known = config if isinstance(config, dict) else {}
        try:
            record_gold_latest_copy_run(
                cfg.ch,
                mart=known.get("mart") if isinstance(known.get("mart"), str) else "<unknown>",
                latest_table=(
                    known.get("latest_table")
                    if isinstance(known.get("latest_table"), str)
                    else "<unknown>"
                ),
                status="failed",
                reason=REASON_INVALID_REQUEST,
            )
        except Exception as exc2:  # noqa: BLE001
            if log:
                log.warning(f"failed to record the invalid-request row: {exc2}")
        raise Failure(description=REASON_INVALID_REQUEST, allow_retries=False) from None

    result = refresh_latest_copy(cfg, req, log)
    if result["status"] == "failed":
        reason = result["reason"]
        raise Failure(
            description=reason, allow_retries=reason in _RETRYABLE_REASONS
        )
    return result


RUN_LIMIT_ENV = "DAGSTER_RUN_LIMIT_GOLD_LATEST_COPY_JOB_SECONDS"
DEFAULT_RUN_LIMIT_SECONDS = 3_600


@job(tags=job_tags("gold_latest_copy_job", RUN_LIMIT_ENV, DEFAULT_RUN_LIMIT_SECONDS))
def gold_latest_copy_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `gold_latest_copy_job`. One
    op, one mart, one publish. Only the API launches it (with the
    publish's run config), so it has no schedule and no sensor — the same
    reason `discover_job` has neither: there is no such thing as "the"
    publish to run it on."""
    refresh_gold_latest_copy()
