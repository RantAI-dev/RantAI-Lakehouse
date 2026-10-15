"""ADR 0010 — Gold export to Iceberg, scheduled from Dagster.

This module is deliberately thin: it does not touch `ClickHouse` or
Lakekeeper itself. The export mechanics (`ClickHouse` MergeTree read ->
Arrow -> `iceberg-rust` append through Lakekeeper, vended credentials,
format-version 2) live in Rust — `lakehouse-iceberg` +
`lakehouse-api::gold_export`, wired to `POST /api/gold/export/{mart}`
(`lakehouse-api::routes::gold`) — because `iceberg-rust` is what G1(a)
proved works, and there is no Python client for it in this stack. This
job's whole job is to be the scheduled trigger: call that one endpoint,
per configured mart, on a cadence, and record the result through the same
`bronze_meta.*` registry mechanism `maintenance.py`/
`replication_metrics.py` already use (per R10 — reuse that mechanism,
don't invent a parallel one).

Why an HTTP call to `lakehouse-api`, not a Dagster op that talks to
`iceberg-rust`/Lakekeeper directly: this is the exact "Dagster calls the
Rust API over HTTP" shape `lakehouse-api::routes::pipelines` already uses
in the opposite direction (`lakehouse-api` calling Dagster's GraphQL API)
— Dagster is a Python process with no `iceberg-rust` binding, and
`lakehouse-api` is already the one process in this stack the task brief
designates as the catalog-operation owner (ADR 0003).

# One op per mart (PART A of `parts/1b-dagster-one-op-per-unit-and-retries.md`)

Before this split a single `run_gold_export` op looped over every
configured mart and surfaced the WHOLE job's result as one step: one
mart's HTTP 500 took down the whole nightly run, including the marts
that would have succeeded. Now `list_gold_marts` yields one
`DynamicOutput` per mart, `export_gold_mart` runs the per-mart body
in a mapped step (so each mart is its own failure unit, retryable by
`DEFAULT_RETRY_POLICY` for transient blips), and `summarize_gold_export`
logs the count. The console now sees one mapped step per mart named
`export_gold_mart[<key>]` where it previously saw the single
`run_gold_export` step.

# Which marts run (gold-publish-per-mart plan T5, DATA-1)

Before this change the mart list came only from `GOLD_EXPORT_MARTS`
(default `gold_export_smoke`) — a build-time constant no operator could
change without redeploying, and one that ignored everything the console's
per-mart switches said. Now `list_gold_marts` unions two sources, the
API's enabled publications (`GET /api/gold/publications`, what the
console's switches write — the database is the source of truth) and the
env override (an operator's escape hatch, default empty), deduplicated
by mart name; two DISTINCT marts that would collapse into one Dagster
step key are still refused. Every scheduled POST sends `ifChanged=true`, so a mart whose ClickHouse
content fingerprint (row count, wrapping 64-bit sum of cityHash64(*), and
schema hash from system.columns) matches the fingerprint recorded on its last
successful export comes back `{skipped: true}` (DATA-10). Background merges
(`OPTIMIZE TABLE ... FINAL`) and atomic full rewrites via staging + `EXCHANGE
TABLES` alter part modification times without changing table contents; the
content fingerprint distinguishes genuine data changes from storage churn,
preventing spurious re-exports — recorded as `skipped_verbs=["unchanged"]`
with no `AssetMaterialization`, since nothing was written — and the empty union
runs a zero-step job that still succeeds, which is the honest shape of
"nothing is published."

# Copy retention and trimming (DATA-2)

Each exported Gold mart retains the last N distinct copies (default 7,
configured via `GOLD_KEEP_COPIES`). Following `export_gold_mart`, the mapped
op `trim_gold_mart` prunes older copies via PyIceberg delete matching
`_exported_at < cutoff` and snapshot expiry, followed by ClickHouse
`EXECUTE remove_orphan_files` through `icecat_gold_export` with
`iceberg_orphan_files_older_than_seconds = <GOLD_ORPHAN_MIN_AGE_SECONDS>`
(default 3600 seconds, 1 hour; DATA-2 review BLOCKER 1) to delete unreferenced
Parquet files from object storage without endangering in-flight uncommitted
appends from concurrent exports. The trim therefore frees a removed copy's
files on the first trim that runs at least an hour after they were written. Every trim operation records its result (status,
copies kept, oldest kept timestamp, copies removed, snapshots expired,
files removed, and fixed failure reasons) in `console.gold_trim_run`."""

from __future__ import annotations

import datetime
import os
import uuid
import re
from dataclasses import dataclass
from typing import Any

import requests
from dagster import (
    AssetMaterialization,
    DagsterRunStatus,
    DefaultScheduleStatus,
    DefaultSensorStatus,
    Definitions,
    DynamicOut,
    DynamicOutput,
    Failure,
    RunRequest,
    RunStatusSensorContext,
    ScheduleDefinition,
    SkipReason,
    job,
    op,
    run_status_sensor,
)

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, _ch_exec, record_maintenance_run
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


INVALID_KEEP_COPIES_REASON = "invalid GOLD_KEEP_COPIES"
INVALID_ORPHAN_MIN_AGE_REASON = "invalid GOLD_ORPHAN_MIN_AGE_SECONDS"


class InvalidKeepCopiesError(ValueError):
    """Raised when GOLD_KEEP_COPIES cannot be parsed as an integer >= 1."""


class InvalidOrphanMinAgeError(ValueError):
    """Raised when GOLD_ORPHAN_MIN_AGE_SECONDS cannot be parsed as an integer >= 0 (DATA-2 review BLOCKER 1)."""


def parse_gold_keep_copies(raw: Any) -> int:
    """Parse GOLD_KEEP_COPIES setting as an integer >= 1 (DATA-2 D4).

    Anything else (0, negative, non-numeric, empty string, None) raises
    InvalidKeepCopiesError("invalid GOLD_KEEP_COPIES").
    """
    if raw is None:
        raise InvalidKeepCopiesError(INVALID_KEEP_COPIES_REASON)
    if isinstance(raw, str):
        raw = raw.strip()
        if not raw:
            raise InvalidKeepCopiesError(INVALID_KEEP_COPIES_REASON)
    try:
        val = int(raw)
    except (ValueError, TypeError):
        raise InvalidKeepCopiesError(INVALID_KEEP_COPIES_REASON) from None
    if val < 1:
        raise InvalidKeepCopiesError(INVALID_KEEP_COPIES_REASON)
    return val


def parse_gold_orphan_min_age_seconds(raw: Any) -> int:
    """Parse GOLD_ORPHAN_MIN_AGE_SECONDS setting as an integer >= 0 (DATA-2 review BLOCKER 1).

    Anything else (negative, non-numeric, empty string, None) raises
    InvalidOrphanMinAgeError("invalid GOLD_ORPHAN_MIN_AGE_SECONDS").
    """
    if raw is None:
        raise InvalidOrphanMinAgeError(INVALID_ORPHAN_MIN_AGE_REASON)
    if isinstance(raw, str):
        raw = raw.strip()
        if not raw:
            raise InvalidOrphanMinAgeError(INVALID_ORPHAN_MIN_AGE_REASON)
    try:
        val = int(raw)
    except (ValueError, TypeError):
        raise InvalidOrphanMinAgeError(INVALID_ORPHAN_MIN_AGE_REASON) from None
    if val < 0:
        raise InvalidOrphanMinAgeError(INVALID_ORPHAN_MIN_AGE_REASON)
    return val


def _marts_from_env() -> list[str]:
    """The operator's `GOLD_EXPORT_MARTS` override, default empty: the
    scheduled mart list is owned by the per-mart publication setting
    (`GET /api/gold/publications`, gold-publish-per-mart plan T5), and
    this env is only a union-ed escape hatch for an operator who needs a
    mart exported before anyone switched it on in the console. A fresh
    deployment with nothing switched on exports nothing — on purpose."""
    raw = os.environ.get("GOLD_EXPORT_MARTS", "")
    return [m.strip() for m in raw.split(",") if m.strip()]


@dataclass(frozen=True)
class GoldExportConfig:
    ch: ClickHouseTarget
    api_url: str
    marts: list[str]
    # D4 shape (`routes::gold::check_export_token`): a shared token, set
    # identically here and on `lakehouse-api` (both read
    # `GOLD_EXPORT_RUN_TOKEN` from the same compose `.env` — see
    # `docker-compose.yml`'s `gold-export-test-runner` usage comment for
    # why a one-off override here would not also reach the already-running
    # `lakehouse-api` container). The value authenticates twice:
    # `lakehouse-api::main::bootstrap_gold_export_service` mints the
    # `gold-export-scheduler` identity + credential from it at API boot
    # (gold-publish-per-mart plan T1), `_headers` sends it as both the
    # bearer credential and the `x-run-token` header. Empty means neither
    # exists — the schedule then runs and fails loudly (`401`/`503`)
    # instead of silently, which is the honest state to ship.
    run_token: str
    gold_keep_copies: str = "7"
    gold_orphan_min_age_seconds: str = "3600"
    gold_export_token_file: str = "/tokens/gold-export.jwt"
    lakekeeper_catalog_uri: str = "http://lakekeeper:8181/catalog"
    lakekeeper_warehouse: str = "default"
    rustfs_endpoint: str = "http://rustfs:9000"
    rustfs_access_key: str = "rustfsadmin"
    rustfs_secret_key: str = "rustfsadmin"
    ch_oauth_client_id: str = ""
    ch_oauth_server_uri: str = ""

    @classmethod
    def from_env(cls) -> "GoldExportConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            marts=_marts_from_env(),
            run_token=_env("GOLD_EXPORT_RUN_TOKEN", ""),
            gold_keep_copies=_env("GOLD_KEEP_COPIES", "7"),
            gold_orphan_min_age_seconds=_env("GOLD_ORPHAN_MIN_AGE_SECONDS", "3600"),
            gold_export_token_file=_env(
                "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE", "/tokens/gold-export.jwt"
            ),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            rustfs_access_key=_env("CONNECTOR_S3_ACCESS_KEY", "rustfsadmin"),
            rustfs_secret_key=_env("CONNECTOR_S3_SECRET_KEY", "rustfsadmin"),
            ch_oauth_client_id=_env("CH_OAUTH_CLIENT_ID", ""),
            ch_oauth_server_uri=_env("CH_OAUTH_SERVER_URI", ""),
        )

    def ch_auth_settings(self) -> str:
        if not self.ch_oauth_client_id:
            return ""
        return (
            f", catalog_credential = '{self.ch_oauth_client_id}:unused', "
            f"oauth_server_uri = '{self.ch_oauth_server_uri}'"
        )


def _headers(cfg: GoldExportConfig) -> dict[str, str]:
    """Same two-header shape as `alerts_run.py::_headers` — see the
    schedule comment below for why both are needed. Empty when no token
    is configured, so a caller can tell "not configured" apart from
    "configured, but Dagster forgot to send it" without inspecting the
    response.
    """
    if not cfg.run_token:
        return {}
    return {
        "x-run-token": cfg.run_token,
        "Authorization": f"Bearer {cfg.run_token}",
    }


def export_one_mart(cfg: GoldExportConfig, mart: str) -> dict[str, Any]:
    """`ifChanged=true` (gold-publish-per-mart plan T4): the API skips
    the export — 200 `{skipped: true, reason: ...}`, nothing written,
    no history row — when the mart's `max(modification_time)` last moved
    strictly before its last successful export (equal timestamps export,
    PR slice B review B1). The manual console trigger sends no such
    parameter; only the schedule measures before it writes."""
    resp = requests.post(
        f"{cfg.api_url}/api/gold/export/{mart}",
        params={"ifChanged": "true"},
        headers=_headers(cfg),
        timeout=60,
    )
    resp.raise_for_status()
    return resp.json()


def _enabled_publication_marts(cfg: GoldExportConfig) -> list[str]:
    """The marts switched on in the console, in the order the API lists
    them. A failed fetch raises (bare `requests` exception ->
    `DEFAULT_RETRY_POLICY`) — never swallowed into "zero marts", which
    would silently skip every export; a malformed body raises a
    non-retryable `Failure`, because a retry would read the same wrong
    shape."""
    resp = requests.get(
        f"{cfg.api_url}/api/gold/publications", headers=_headers(cfg), timeout=30
    )
    resp.raise_for_status()
    publications = resp.json().get("publications")
    if not isinstance(publications, list):
        raise Failure(
            f"GET {cfg.api_url}/api/gold/publications returned no "
            f"publications list -- expected {{\"publications\": [...]}}",
            allow_retries=False,
        )
    marts: list[str] = []
    for publication in publications:
        mart = publication.get("mart") if isinstance(publication, dict) else None
        if not isinstance(mart, str) or not mart:
            raise Failure(
                f"GET {cfg.api_url}/api/gold/publications returned a "
                f"publication without a mart name: {publication!r}",
                allow_retries=False,
            )
        marts.append(mart)
    return marts


# Dagster's `DynamicOutput.mapping_key` must match `^[A-Za-z0-9_]+$`
# (`dagster._core.definitions.utils.check_valid_chars`). The exact ASCII
# rule from `parts/1b-dagster-one-op-per-unit-and-retries.md`: every
# character outside `[A-Za-z0-9_]` becomes `_`. Precompiled once at
# module load (`re`'s default cache keeps it cheap to call per input);
# this is the same rule `gold_export._sanitize_mapping_key`,
# `ingest_factory._sanitize_target`, and `maintenance._sanitize_mapping_key`
# all enforce, so a single mapping key can serve as a schedule name, an
# op name, or a mapping key consistently. Note: `str.isalnum()` would
# be wrong here -- it is True for non-ASCII letters (`"é".isalnum()`,
# `"²".isalnum()`), which would silently produce a non-ASCII mapping
# key that Dagster then rejects.
_SANITIZE_NON_ASCII = re.compile(r"[^A-Za-z0-9_]")


def _sanitize_mapping_key(name: str) -> str:
    """Dagster's `DynamicOutput.mapping_key` derived from a mart name by
    replacing every character outside `[A-Za-z0-9_]` with `_`. Two
    distinct marts that collide after this sanitization would mean a
    single mapped step trying to do two marts' work, which Dagster's
    graph itself would refuse to build -- caught here at the fan-out
    op, not at graph-build time, with both names in the message."""
    return _SANITIZE_NON_ASCII.sub("_", name)


# The op itself only calls the API per mart; the API reads the ClickHouse
# mart and writes its Iceberg copy, which is the data flow declared here.
@op(
    out=DynamicOut(),
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::list_gold_marts",
        reads=[
            "GET /api/gold/publications (enabled marts, the source of truth)",
            "env GOLD_EXPORT_MARTS (operator override, unioned in)",
        ],
        # `writes` is the side-effect of this op: a `DynamicOutput` per
        # mart, which becomes a mapped `export_gold_mart[<key>]` step.
        # Declared here so the test that walks every op in this code
        # location and asserts non-empty `writes` (the same one
        # `authored_factory`'s ops already satisfy) does not flag this
        # fan-out as having no writes.
        writes=["DynamicOutput mapping_key per configured Gold mart"],
    ),
)
def list_gold_marts(context) -> Any:
    """Yield one `DynamicOutput` per mart to export this run: every
    publication switched on in the console (`GET /api/gold/publications`)
    unioned with the `GOLD_EXPORT_MARTS` override, deduplicated by exact
    mart name — a mart listed in both places is one step, not two.

    Two DISTINCT marts that sanitize to the same Dagster mapping key are
    refused with a non-retryable `Failure` naming both and where each
    came from -- they would otherwise collapse into one mapped step
    doing two marts' work, which is a failure mode only the fan-out can
    detect."""
    cfg = GoldExportConfig.from_env()
    enabled = _enabled_publication_marts(cfg)
    context.log.info(
        f"enabled publications: {enabled}; GOLD_EXPORT_MARTS override: {cfg.marts}"
    )
    seen_names: set[str] = set()
    seen_keys: dict[str, str] = {}
    sources = [("publication setting", m) for m in enabled]
    sources += [("GOLD_EXPORT_MARTS", m) for m in cfg.marts]
    for source, mart in sources:
        if mart in seen_names:
            continue
        key = _sanitize_mapping_key(mart)
        if key in seen_keys:
            raise Failure(
                f"two Gold marts map to the same Dagster step key {key!r}: "
                f"{seen_keys[key]!r} and {mart!r} ({source}) -- "
                "rename one so each mart maps to a unique step",
                allow_retries=False,
            )
        seen_names.add(mart)
        seen_keys[key] = mart
        yield DynamicOutput(mart, mapping_key=key)


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::export_gold_mart",
        reads=["ClickHouse serving.{mart} (via the API)"],
        writes=[
            "Iceberg gold.{mart} (via POST /api/gold/export)",
            "ClickHouse lake.bronze_meta.maintenance_run",
        ],
    ),
)
def export_gold_mart(context, mart: str) -> dict[str, Any]:
    """Run `POST /api/gold/export/{mart}` for ONE mart and record the
    outcome via `record_maintenance_run` (the same `bronze_meta.*`
    registry `maintenance.py` writes to -- `GET /api/governance/
    maintenance` already surfaces that table, so a Gold export run shows
    up there too without a new console surface).

    An HTTP error is recorded as a `maintenance_run` failure row AND
    bare re-raised (no `Failure(allow_retries=False)` wrapper) -- the
    re-raise lets `DEFAULT_RETRY_POLICY` retry a transient blip (a busy
    ClickHouse on the `lakehouse-api` side, the canonical case the
    plan names), while a persistent 4xx/5xx fails after the policy's
    two attempts. Config-shaped failures (`SecretRefRejected`,
    `UnknownAdapter`, etc.) are not raised here at all -- the
    connector-resolution path in `_fetch_one_connector` /
    `_resolve_secrets` either resolves them before this op runs or
    fails out of band. Re-raising the HTTPError bare keeps the
    partition between "retryable transient" and "non-retryable
    config/auth" honest: only the latter wraps in
    `Failure(allow_retries=False)`, and they don't originate here."""
    cfg = GoldExportConfig.from_env()
    try:
        body = export_one_mart(cfg, mart)
    except requests.HTTPError as exc:
        # PR slice C review C-B1: HTTP 409 — the API's single-flight lock
        # (an export of this mart is already running) — is a skip, not a
        # failure. Record it and return without raising; no
        # AssetMaterialization, because nothing was written in this run.
        if exc.response is not None and exc.response.status_code == 409:
            context.log.info(f"skipped {mart!r}: already running (HTTP 409)")
            record_maintenance_run(
                table_name=f"gold.{mart}",
                dry_run_metrics={},
                applied_metrics={},
                skipped_verbs=["already_running"],
                target=cfg.ch,
            )
            return {"skipped": True, "reason": "already_running"}
        # Bare re-raise of the HTTPError (no `Failure` wrap) -- lets
        # `DEFAULT_RETRY_POLICY` (max_retries=2, delay=30s, exponential
        # backoff with +/- jitter) retry a transient 5xx (busy ClickHouse
        # on the `lakehouse-api` side). A persistent failure exhausts
        # those attempts and Dagster surfaces it as the mapped step's
        # own failed step. The failure row is written BEFORE the
        # re-raise so the governance surface (`GET /api/governance/
        # maintenance`) sees the failure immediately, not only after
        # the policy's retries have all come up empty.
        context.log.error(f"export of {mart!r} failed: {exc}")
        record_maintenance_run(
            table_name=f"gold.{mart}",
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=[f"export_failed: {exc}"],
            target=cfg.ch,
        )
        raise
    # A skip is a successful no-op, not an error: the API answered 200
    # with `{skipped: true}` because the mart's `max(modification_time)`
    # last moved strictly before its last successful export (plan T4,
    # strict per PR slice B review B1). It gets
    # its `maintenance_run` row -- `skipped_verbs=["unchanged"]`, the
    # honest ledger of "looked, nothing to do" -- but no
    # `AssetMaterialization`, because nothing was written to Iceberg.
    if body.get("skipped") is True:
        context.log.info(f"skipped {mart!r}: {body.get('reason')}")
        record_maintenance_run(
            table_name=f"gold.{mart}",
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=["unchanged"],
            target=cfg.ch,
        )
        return body
    context.log.info(f"exported {mart!r}: {body}")
    # `dry_run_metrics`/`applied_metrics`' numeric fields are shaped for
    # `expire_snapshots` (data/manifest file deletion counts), which a
    # Gold export never performs -- left at their honest default of 0,
    # not repurposed to mean something else. `rowsExported` (the one
    # number this run actually produced) goes into the free-text
    # `skipped_verbs` field instead, so it is still visible via
    # `GET /api/governance/maintenance` without corrupting a column
    # whose meaning `maintenance.py`'s own readers rely on.
    record_maintenance_run(
        table_name=f"gold.{mart}",
        dry_run_metrics={},
        applied_metrics={},
        skipped_verbs=[f"rows_exported={body.get('rowsExported')}"],
        target=cfg.ch,
    )
    # The API's own count of rows it wrote to Iceberg, as a
    # materialization, so the run's step carries it and the console
    # can show rows per run. A body without an integer count reports
    # nothing rather than a guessed number.
    rows = body.get("rowsExported")
    if isinstance(rows, int) and not isinstance(rows, bool):
        context.log_event(AssetMaterialization(asset_key=f"gold.{mart}", metadata={"rows": rows}))
    return body



def _sql_str(s: str | None) -> str:
    if s is None:
        return "NULL"
    escaped = s.replace("\\", "\\\\").replace("'", "\\'")
    return f"'{escaped}'"


def _sql_num(n: int | None) -> str:
    if n is None:
        return "NULL"
    return str(int(n))


def _sql_dt64(dt_or_ms: Any) -> str:
    if dt_or_ms is None:
        return "NULL"
    if isinstance(dt_or_ms, (int, float)):
        return f"fromUnixTimestamp64Milli({int(dt_or_ms)})"
    if hasattr(dt_or_ms, "timestamp"):
        ms = int(dt_or_ms.timestamp() * 1000)
        return f"fromUnixTimestamp64Milli({ms})"
    return f"parseDateTime64BestEffortOrNull({_sql_str(str(dt_or_ms))}, 3)"


def ensure_gold_trim_run_table(target: ClickHouseTarget) -> None:
    """Ensure `console.gold_trim_run` exists (DATA-2 D5 + A1)."""
    _ch_exec(target, "CREATE DATABASE IF NOT EXISTS console")
    _ch_exec(
        target,
        """CREATE TABLE IF NOT EXISTS console.gold_trim_run (
            id String,
            mart String,
            status String,
            copies_kept Nullable(UInt32),
            oldest_kept_exported_at Nullable(DateTime64(3)),
            copies_removed Nullable(UInt32),
            snapshots_expired Nullable(UInt32),
            files_removed Nullable(UInt32),
            reason Nullable(String),
            started_at DateTime64(3),
            finished_at DateTime64(3)
        ) ENGINE = MergeTree ORDER BY (mart, started_at)""",
    )
    _ch_exec(
        target,
        "ALTER TABLE console.gold_trim_run ADD COLUMN IF NOT EXISTS files_removed Nullable(UInt32)",
    )


def record_gold_trim_run(
    target: ClickHouseTarget | None,
    *,
    mart: str,
    status: str,
    copies_kept: int | None = None,
    oldest_kept_exported_at: Any = None,
    copies_removed: int | None = None,
    snapshots_expired: int | None = None,
    files_removed: int | None = None,
    reason: str | None = None,
    started_at: Any = None,
    finished_at: Any = None,
) -> None:
    """Record one trim run into `console.gold_trim_run` (DATA-2 D5 + A1)."""
    if target is None:
        return
    ensure_gold_trim_run_table(target)
    run_id = str(uuid.uuid4())
    started_at = started_at or datetime.datetime.now(datetime.timezone.utc)
    finished_at = finished_at or datetime.datetime.now(datetime.timezone.utc)
    sql = (
        f"INSERT INTO console.gold_trim_run ("
        f"id, mart, status, copies_kept, oldest_kept_exported_at, "
        f"copies_removed, snapshots_expired, files_removed, reason, "
        f"started_at, finished_at) VALUES ("
        f"{_sql_str(run_id)}, {_sql_str(mart)}, {_sql_str(status)}, "
        f"{_sql_num(copies_kept)}, {_sql_dt64(oldest_kept_exported_at)}, "
        f"{_sql_num(copies_removed)}, {_sql_num(snapshots_expired)}, "
        f"{_sql_num(files_removed)}, {_sql_str(reason)}, "
        f"{_sql_dt64(started_at)}, {_sql_dt64(finished_at)})"
    )
    _ch_exec(target, sql)


def load_gold_catalog(cfg: GoldExportConfig) -> Any:
    """Instantiate a PyIceberg `RestCatalog` with the `gold-export` token."""
    token = ""
    if cfg.gold_export_token_file and os.path.exists(cfg.gold_export_token_file):
        with open(cfg.gold_export_token_file, encoding="utf-8") as f:
            token = f.read().strip()
    from pyiceberg.catalog.rest import RestCatalog

    return RestCatalog(
        name="lakekeeper",
        warehouse=cfg.lakekeeper_warehouse,
        uri=cfg.lakekeeper_catalog_uri,
        token=token,
        **{
            "s3.endpoint": cfg.rustfs_endpoint,
            "s3.access-key-id": cfg.rustfs_access_key,
            "s3.secret-access-key": cfg.rustfs_secret_key,
            "s3.region": "us-east-1",
            "s3.path-style-access": "true",
        },
    )


def run_trim_for_mart(
    cfg: GoldExportConfig, mart: str, context: Any = None
) -> dict[str, Any]:
    """Execute trim logic for a single Gold mart (DATA-2 D1, D3, D5, A1)."""
    started_at = datetime.datetime.now(datetime.timezone.utc)
    log = context.log if context else None

    # 1. Parse keep_copies (D4) and orphan_min_age_seconds (DATA-2 review BLOCKER 1)
    try:
        keep_copies = parse_gold_keep_copies(cfg.gold_keep_copies)
    except InvalidKeepCopiesError:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"invalid GOLD_KEEP_COPIES: {cfg.gold_keep_copies!r}")
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=INVALID_KEEP_COPIES_REASON,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": INVALID_KEEP_COPIES_REASON}

    try:
        orphan_min_age_seconds = parse_gold_orphan_min_age_seconds(
            cfg.gold_orphan_min_age_seconds
        )
    except InvalidOrphanMinAgeError:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(
                f"invalid GOLD_ORPHAN_MIN_AGE_SECONDS: {cfg.gold_orphan_min_age_seconds!r}"
            )
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=INVALID_ORPHAN_MIN_AGE_REASON,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": INVALID_ORPHAN_MIN_AGE_REASON}

    # 2. Connect to catalog and load table
    try:
        catalog = load_gold_catalog(cfg)
        table = catalog.load_table(f"gold.{mart}")
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to load table gold.{mart}: {exc}")
        fixed_reason = "catalog_unavailable"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    # 3. Check table contents
    try:
        df = table.scan().to_arrow()
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to scan table gold.{mart}: {exc}")
        fixed_reason = "table_scan_failed"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    if len(df) == 0:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.info(f"gold.{mart} has 0 rows; nothing to trim")
        record_gold_trim_run(
            cfg.ch,
            mart=mart,
            status="nothing_to_trim",
            copies_kept=0,
            copies_removed=0,
            snapshots_expired=0,
            files_removed=0,
            reason="at_or_below_keep_copies",
            started_at=started_at,
            finished_at=finished_at,
        )
        return {"status": "nothing_to_trim", "copies_kept": 0}

    exported_at_col = df["_exported_at"].to_pylist()
    distinct_exported_at = sorted(set(exported_at_col), reverse=True)
    distinct_count = len(distinct_exported_at)

    if distinct_count <= keep_copies:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        oldest_kept = distinct_exported_at[-1]
        if log:
            log.info(
                f"gold.{mart} has {distinct_count} copies (<= keep_copies {keep_copies}); "
                "nothing to trim"
            )
        record_gold_trim_run(
            cfg.ch,
            mart=mart,
            status="nothing_to_trim",
            copies_kept=distinct_count,
            oldest_kept_exported_at=oldest_kept,
            copies_removed=0,
            snapshots_expired=0,
            files_removed=0,
            reason="at_or_below_keep_copies",
            started_at=started_at,
            finished_at=finished_at,
        )
        return {"status": "nothing_to_trim", "copies_kept": distinct_count}

    # 4. Trimming: delete copies strictly older than the keep_copies-th newest
    cutoff_time = distinct_exported_at[keep_copies - 1]
    copies_to_remove = distinct_count - keep_copies
    if log:
        log.info(
            f"trimming gold.{mart}: {distinct_count} copies, keeping {keep_copies}, "
            f"removing {copies_to_remove} (cutoff {cutoff_time})"
        )

    # 4a. Delete rows
    try:
        from pyiceberg.expressions import LessThan

        table.delete(LessThan("_exported_at", cutoff_time))
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to delete rows from gold.{mart}: {exc}")
        from pyiceberg.exceptions import CommitFailedException

        fixed_reason = (
            "commit_conflict"
            if isinstance(exc, CommitFailedException)
            else "delete_failed"
        )
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    # 4b. Expire snapshots
    try:
        current_snapshot = table.current_snapshot()
        current_snapshot_id = (
            current_snapshot.snapshot_id if current_snapshot else None
        )
        older_snapshot_ids = [
            s.snapshot_id
            for s in table.snapshots()
            if s.snapshot_id != current_snapshot_id
        ]
        if older_snapshot_ids:
            table.maintenance.expire_snapshots().by_ids(older_snapshot_ids).commit()
        snapshots_expired = len(older_snapshot_ids)
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to expire snapshots for gold.{mart}: {exc}")
        fixed_reason = "expire_snapshots_failed"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    # 4c. ClickHouse remove_orphan_files (amendment A1)
    files_removed = 0
    try:
        from dispar_orchestrate.maintenance import (
            ensure_catalog_database,
            run_remove_orphan_files,
        )

        catalog_db = "icecat_gold_export"
        ensure_catalog_database(cfg, db_name=catalog_db)
        # DATA-2 review BLOCKER 1: pass orphan_min_age_seconds to prevent
        # deleting in-flight uncommitted files from concurrent exports.
        orphan_res = run_remove_orphan_files(
            cfg,
            f"gold.{mart}",
            dry_run=False,
            catalog_db=catalog_db,
            older_than_seconds=orphan_min_age_seconds,
        )
        files_removed = int(orphan_res.get("deleted_data_files_count", 0))
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to remove orphan files for gold.{mart}: {exc}")
        fixed_reason = "orphan_cleanup_failed"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    # 5. Record success
    finished_at = datetime.datetime.now(datetime.timezone.utc)
    try:
        record_gold_trim_run(
            cfg.ch,
            mart=mart,
            status="success",
            copies_kept=keep_copies,
            oldest_kept_exported_at=cutoff_time,
            copies_removed=copies_to_remove,
            snapshots_expired=snapshots_expired,
            files_removed=files_removed,
            reason=None,
            started_at=started_at,
            finished_at=finished_at,
        )
    except Exception as e:
        if log:
            log.warning(f"failed to record gold_trim_run for {mart}: {e}")

    return {
        "status": "success",
        "copies_kept": keep_copies,
        "copies_removed": copies_to_remove,
        "snapshots_expired": snapshots_expired,
        "files_removed": files_removed,
    }


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::trim_gold_mart",
        reads=["Iceberg gold.{mart}"],
        writes=[
            "Iceberg gold.{mart} (delete older copies and expire snapshots)",
            "ClickHouse console.gold_trim_run",
        ],
    ),
)
def trim_gold_mart(
    context, mart: str, export_result: dict[str, Any]
) -> dict[str, Any]:
    """Trim old copies from `gold.{mart}` after a successful export (DATA-2 D1).
    A skipped or failed export does not trim.
    Always returns `export_result` untouched so the export summary is preserved.
    """
    if export_result.get("skipped") is True:
        context.log.info(f"export of {mart!r} was skipped; skipping trim")
        return export_result

    cfg = GoldExportConfig.from_env()
    try:
        trim_res = run_trim_for_mart(cfg, mart, context=context)
        context.log.info(f"trim result for {mart!r}: {trim_res}")
    except Exception as exc:
        context.log.error(f"unexpected exception trimming {mart!r}: {exc}")

    return export_result


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::summarize_gold_export",
        # The output of this op is the run's summary count, surfaced
        # through `add_output_metadata({"marts_exported": N})`; declared
        # as a `writes` phrase so the every-op walk test (which refuses
        # empty `writes`) treats this no-input collect step as having a
        # real artifact.
        reads=["results list from trim_gold_mart[*]"],
        writes=["output metadata marts_exported"],
    ),
)
def summarize_gold_export(context, results: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Collect every mapped `export_gold_mart` step's result and log the
    marts-exporter count (the single number this whole job owes to the
    rest of the code location -- a per-mart rows count is on each
    `export_gold_mart` step's own materialization, never aggregated
    here)."""
    context.log.info(f"exported {len(results)} Gold mart(s) this run")
    context.add_output_metadata({"marts_exported": len(results)})
    return results


@job
def gold_export_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `gold_export_job`. A fan-out
    op (`list_gold_marts`) yields one `DynamicOutput` per mart; the
    mapped `export_gold_mart` runs the per-mart export, followed by
    `trim_gold_mart` which keeps the newest N copies in `gold.<mart>` (DATA-2);
    `summarize_gold_export` collects every mapped step's result for the run summary."""
    summarize_gold_export(
        list_gold_marts()
        .map(lambda mart: trim_gold_mart(mart, export_gold_mart(mart)))
        .collect()
    )


# Daily at 04:00 — after `bronze_maintenance_job`'s 03:00 slot, since a
# freshly-maintained Bronze table is what most Gold marts are ultimately
# built from; arbitrary but conservative cadence, same reasoning
# `bronze_maintenance_schedule` gives.
#
# `default_status=DefaultScheduleStatus.RUNNING`, matching every other
# schedule this code location registers (`bronze_maintenance_schedule`,
# `capacity_snapshot_schedule`, `alerts_run_schedule`): a schedule created
# stopped never fires until someone notices and manually flips it on in
# the Dagster UI, and from outside, a stopped schedule looks identical to
# a healthy one — there is no dashboard signal that distinguishes "off on
# purpose" from "off because nobody remembered." Registering RUNNING makes
# the eventual failure loud (a run appears in the Dagster UI's run list,
# failed, every night) instead of silent (no run ever appears at all).
# `default_status` must be the `DefaultScheduleStatus` enum member, not a
# bare string — the installed Dagster version raises
# `dagster._core.errors.ParameterCheckError` for `default_status="RUNNING"`
# (confirmed directly against this repo's `~/.cache/rantai-dagster-venv`;
# a different task in this programme hit exactly this).
#
# Auth (gold-publish-per-mart plan T1): `POST /api/gold/export/{mart}`'s
# `Policy::RequiresAuth` floor is enforced by `auth_gate` BEFORE the
# handler (and `check_export_token`'s own `x-run-token` check) ever runs,
# so a real credential is required — `_headers` sends the shared token two
# ways, exactly like `alerts_run.py`: `Authorization: Bearer
# <GOLD_EXPORT_RUN_TOKEN>` clears the floor via the `gold-export-scheduler`
# identity that `lakehouse-api::main::bootstrap_gold_export_service`
# idempotently mints from the same env value at API boot, and
# `x-run-token: <GOLD_EXPORT_RUN_TOKEN>` satisfies `check_export_token`'s
# token branch, which is what actually gates the export. With the token
# unset, no identity is seeded and `_headers` returns `{}` — the schedule
# below is still registered and every run fails loudly (`401`/`503`),
# which is the honest, visible failure mode, not a silent one.
# PR slice C review C-S1: each authored pipeline SUCCESS launches one
# all-marts `gold_export_job` run. With T4's `ifChanged=true` in place,
# unchanged marts cost one cheap check each and are skipped (no duplicate
# copy); a mart is skipped when its `max(modification_time)` last moved
# strictly before its last successful export (PR slice B review B1).
# Dagster's `run_key=<upstream run id>` prevents a single pipeline
# success from launching two Gold runs. The nightly 04:00 schedule is
# the safety net, not the only trigger; with all marts already published
# by the sensor, the nightly run produces mainly skips.
#
# `monitored_jobs=None` (watch every job in this code location) with a
# filter in the body: only `authored__<id>` jobs (the same prefix
# `authored_factory.py` gives them at code-load time) trigger; all other
# jobs — `gold_export_job` itself, maintenance, backup, alerts, capacity,
# agent, ingest — yield `SkipReason` so the sensor tick stays green.
#
# `default_status=RUNNING`: a sensor that ships STOPPED silently never
# fires, and from the outside, a stopped sensor looks identical to a
# healthy one (the same reasoning every other schedule/sensor in this
# code location documents). With `GOLD_EXPORT_RUN_TOKEN` unset the
# scheduled POST would still fail — but that is a loud, visible failure
# whose run entry in the Dagster UI is the signal, not a silent one.


def evaluate_authored_success(context: RunStatusSensorContext):
    """The sensor body, split out from the decorator so unit tests can
    drive it with a `RunStatusSensorContext` stub — the same pattern
    `pipeline_events.py::evaluate_finished_run` uses (plan 1f)."""
    job_name = context.dagster_run.job_name
    if not job_name.startswith("authored__"):
        yield SkipReason(
            f"skipped {job_name!r}: sensor only fires on authored-pipeline jobs"
        )
        return
    yield RunRequest(run_key=context.dagster_run.run_id)


@run_status_sensor(
    run_status=DagsterRunStatus.SUCCESS,
    monitored_jobs=None,
    request_job=gold_export_job,
    default_status=DefaultSensorStatus.RUNNING,
    name="gold_export_after_authored_sensor",
)
def gold_export_after_authored_sensor(
    context: RunStatusSensorContext,
):
    """gold-publish-per-mart plan T6: after an authored pipeline succeeds,
    triggers `gold_export_job` so the nightly cadence is the safety net,
    not the only trigger. With T4's `ifChanged=true` in place, unchanged
    marts cost one cheap check each."""
    yield from evaluate_authored_success(context)


gold_export_schedule = ScheduleDefinition(
    job=gold_export_job,
    cron_schedule="0 4 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)

gold_export_defs = Definitions(
    jobs=[gold_export_job],
    schedules=[gold_export_schedule],
)
