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

# Which marts run (gold-publish-per-mart plan T5, DATA-1; DATA-66 D9)

The mart list comes only from `GOLD_EXPORT_MARTS` (an operator's escape
hatch, default empty) — and, since `DATA-66`, only for entries that are
NOT an enabled console publication. A table switched on in the console
follows its own sync mode ("On every update", "On a schedule", "Manual
only"; `iceberg_sync.py`), so the nightly run does not export it behind
that choice's back (feature page decision 12). Two DISTINCT env marts
that would collapse into one Dagster step key are refused. Every
scheduled POST sends `ifChanged=true`, so a mart whose `ClickHouse`
content fingerprint (row count, wrapping 64-bit sum of cityHash64(*), and
schema hash from system.columns) matches the fingerprint recorded on its last
successful export comes back `{skipped: true}` (DATA-10). Background merges
(`OPTIMIZE TABLE ... FINAL`) and atomic full rewrites via staging + `EXCHANGE
TABLES` alter part modification times without changing table contents; the
content fingerprint distinguishes genuine data changes from storage churn,
preventing spurious re-exports — recorded as `skipped_verbs=["unchanged"]`
with no `AssetMaterialization`, since nothing was written — and the empty
list runs a zero-step job that still succeeds, which is the honest shape of
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
    DefaultScheduleStatus,
    Definitions,
    DynamicOut,
    DynamicOutput,
    Failure,
    ScheduleDefinition,
    job,
    op,
)

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, _ch_exec, record_maintenance_run
from dispar_orchestrate.ingest_factory import job_tags
from dispar_orchestrate.maintenance import maintenance_paused
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

# How long one run of this job may run, in seconds. A daily export gets
# room for a slow day. Not measured on staging, 2026-11-16 -- the job has
# been failing there since 13 Nov -- so the limit follows the plan's
# default rather than a measured run.
RUN_LIMIT_ENV = "DAGSTER_RUN_LIMIT_GOLD_EXPORT_JOB_SECONDS"
DEFAULT_RUN_LIMIT_SECONDS = 7_200


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def _required_env(name: str) -> str:
    """Read `name`, failing the run when it is unset or empty.

    `SEC-37`: `from_env` used to fall back to the literal `rustfsadmin` for
    the two S3 credentials. Compose always sets them, so a missing value is
    a misconfiguration: `Failure` (config, not transient — no retry helps)
    naming the variable beats dialling the object store with a guessed key.
    """
    value = _env(name, "")
    if not value:
        raise Failure(f"{name} must be set (SEC-37: no default credentials in code)")
    return value


INVALID_KEEP_COPIES_REASON = "invalid GOLD_KEEP_COPIES"
INVALID_ORPHAN_MIN_AGE_REASON = "invalid GOLD_ORPHAN_MIN_AGE_SECONDS"

# DATA-65: the two publication zones. A Gold target keeps the bare mart
# name as its Dagster step key (so every pre-DATA-65 step key, test and
# run-history reference is unchanged); a Silver target is prefixed so
# `silver.orders` and `serving.orders` cannot collide in one mapped job.
ZONE_SILVER = "silver"
ZONE_GOLD = "gold"
DEFAULT_ZONE = ZONE_GOLD
SILVER_STEP_PREFIX = "silver__"


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
    # CREDENTIAL HYGIENE (review finding): these used to read
    # `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY` directly — RustFS's own ROOT
    # credentials, the same ones the bucket-bootstrap/admin-shaped jobs in
    # docker-compose.yml use. This module only ever reads/writes objects
    # under the warehouse bucket, so it does not need root. It reads the
    # connector-scoped names instead (`CONNECTOR_S3_ACCESS_KEY`/
    # `CONNECTOR_S3_SECRET_KEY` — see docker-compose.yml's
    # `dagster-code-location` env block, where compose points them at the
    # RustFS root key because this stack's RustFS version has no
    # non-proprietary-API way to mint a narrower S3 identity).
    #
    # `SEC-37`: REQUIRED — no default, no fallback. They used to default to
    # the literal `rustfsadmin` here and in `from_env`; a credential written
    # in the code is one an operator never chose (`AGENTS.md` rule 5).
    # `from_env` raises `Failure` naming the variable when either is unset,
    # and there is no constructor that supplies one for you.
    rustfs_access_key: str
    rustfs_secret_key: str
    gold_keep_copies: str = "7"
    gold_orphan_min_age_seconds: str = "3600"
    gold_export_token_file: str = "/tokens/gold-export.jwt"
    lakekeeper_catalog_uri: str = "http://lakekeeper:8181/catalog"
    lakekeeper_warehouse: str = "default"
    rustfs_endpoint: str = "http://rustfs:9000"
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
            rustfs_access_key=_required_env("CONNECTOR_S3_ACCESS_KEY"),
            rustfs_secret_key=_required_env("CONNECTOR_S3_SECRET_KEY"),
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


def export_one_mart(
    cfg: GoldExportConfig, mart: str, zone: str = DEFAULT_ZONE
) -> dict[str, Any]:
    """`ifChanged=true` (gold-publish-per-mart plan T4): the API skips
    the export — 200 `{skipped: true, reason: ...}`, nothing written,
    no history row — when the mart's `max(modification_time)` last moved
    strictly before its last successful export (equal timestamps export,
    PR slice B review B1). The manual console trigger sends no such
    parameter; only the schedule measures before it writes.

    DATA-65: a Silver export adds `zone=silver` to the query. Gold sends
    exactly the old `{ifChanged: true}` params, so no existing URL test or
    deployment changes.
    """
    params = {"ifChanged": "true"}
    if zone != DEFAULT_ZONE:
        params["zone"] = zone
    resp = requests.post(
        f"{cfg.api_url}/api/gold/export/{mart}",
        params=params,
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
    targets: list[MartExportTarget] = []
    for publication in publications:
        mart = publication.get("mart") if isinstance(publication, dict) else None
        if not isinstance(mart, str) or not mart:
            raise Failure(
                f"GET {cfg.api_url}/api/gold/publications returned a "
                f"publication without a mart name: {publication!r}",
                allow_retries=False,
            )
        # DATA-65: a pre-DATA-65 API has no `zone` on an item; it was all
        # Gold. An unknown zone is a protocol error, not a silently
        # defaulted Gold publish into the wrong namespace.
        zone = publication.get("zone", DEFAULT_ZONE) if isinstance(publication, dict) else DEFAULT_ZONE
        if zone not in (ZONE_SILVER, ZONE_GOLD):
            raise Failure(
                f"GET {cfg.api_url}/api/gold/publications returned an "
                f"unknown zone {zone!r} for mart {mart!r}",
                allow_retries=False,
            )
        marts.append(mart)
        targets.append(MartExportTarget(mart, zone))
    return PublicationMarts(marts, targets)


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


@dataclass(frozen=True)
class MartExportTarget:
    """One scheduled export: a mart name plus the zone it publishes to.

    DATA-65: the zone decides both the ClickHouse source schema the API
    reads and the Iceberg namespace the table lands in. A `str` is accepted
    everywhere (and means Gold) so every pre-DATA-65 caller, test and
    in-process run config keeps working unchanged.
    """

    mart: str
    zone: str = DEFAULT_ZONE

    @classmethod
    def from_input(cls, value: Any) -> "MartExportTarget":
        if isinstance(value, cls):
            return value
        if isinstance(value, str):
            return cls(value)
        if isinstance(value, dict):
            mart = value.get("mart")
            zone = value.get("zone", DEFAULT_ZONE)
            if isinstance(mart, str) and isinstance(zone, str):
                return cls(mart, zone)
        raise Failure(
            f"not a Gold export target: {value!r}", allow_retries=False
        )

    @property
    def iceberg_table(self) -> str:
        """The catalog's two-part name of this publication's table."""
        return f"{self.zone}.{self.mart}"

    @property
    def mapping_key(self) -> str:
        """A `DynamicOutput` key unique across both zones and valid for
        Dagster's `^[A-Za-z0-9_]+$` rule. Gold keeps the bare sanitized
        name; Silver is prefixed (DATA-65)."""
        key = _sanitize_mapping_key(self.mart)
        return f"{SILVER_STEP_PREFIX}{key}" if self.zone == ZONE_SILVER else key


class PublicationMarts(list[str]):
    """The mart names returned by `_enabled_publication_marts`, plus the
    `(zone, mart)` targets they came from.

    A plain `list[str]` subclass so callers that only need names (including
    the latest-copy op and the existing tests) see exactly the same value
    they did before DATA-65. `list_gold_marts` reads the extra `targets`
    to schedule each publication with its own zone — a Silver and a Gold
    publication can share a bare mart name and are TWO entries here.
    """

    def __init__(
        self, marts: list[str], targets: list[MartExportTarget] | None = None
    ) -> None:
        super().__init__(marts)
        self.targets = targets or [MartExportTarget(mart) for mart in marts]

    def zone_of(self, mart: str) -> str:
        # The latest-copy op asks "is this mart published in my zone?"
        # after it has learned the zone; a plain list (pre-DATA-65) means
        # Gold, the only zone that existed then.
        for target in self.targets:
            if target.mart == mart:
                return target.zone
        return DEFAULT_ZONE


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
    """Yield one `DynamicOutput` per mart the nightly run still owns:
    the `GOLD_EXPORT_MARTS` override, minus every entry that is an
    enabled console publication in the Gold zone.

    `DATA-66` D9 (feature page decision 12): a table switched on in the
    console follows its own sync mode — "On every update" through the
    watcher and the pipeline hook, "On a schedule" through the watcher,
    "Manual only" through the button — so exporting it here as well
    would ignore that choice. The list still reads
    `GET /api/gold/publications` so a failed fetch fails the run
    (retryable) instead of silently exporting a console table; a Silver
    publication does not hide the Gold entry of the same bare name.

    Two DISTINCT env marts that sanitize to the same Dagster mapping key
    are refused with a non-retryable `Failure` naming both -- they would
    otherwise collapse into one mapped step doing two marts' work, which
    is a failure mode only the fan-out can detect."""
    cfg = GoldExportConfig.from_env()
    enabled = _enabled_publication_marts(cfg)
    context.log.info(
        f"enabled publications: {enabled}; GOLD_EXPORT_MARTS override: {cfg.marts}"
    )
    publication_targets = (
        enabled.targets
        if isinstance(enabled, PublicationMarts)
        else [MartExportTarget(mart) for mart in enabled]
    )
    enabled_gold = {
        target.mart for target in publication_targets if target.zone == ZONE_GOLD
    }
    seen_names: set[str] = set()
    seen_keys: dict[str, str] = {}
    # The env override is an operator escape hatch for Gold marts; it has
    # no zone syntax, exactly as before DATA-65.
    for mart in cfg.marts:
        if mart in enabled_gold:
            context.log.info(
                f"GOLD_EXPORT_MARTS entry {mart!r} is an enabled console "
                "publication; its sync mode decides, not this run"
            )
            continue
        if mart in seen_names:
            continue
        target = MartExportTarget(mart)
        key = target.mapping_key
        if key in seen_keys:
            raise Failure(
                f"two GOLD_EXPORT_MARTS entries map to the same Dagster step "
                f"key {key!r}: {seen_keys[key]!r} and {target.iceberg_table!r} "
                "-- rename one so each mart maps to a unique step",
                allow_retries=False,
            )
        seen_names.add(mart)
        seen_keys[key] = target.iceberg_table
        yield DynamicOutput(target, mapping_key=key)


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
def export_gold_mart(context, mart: Any) -> dict[str, Any]:
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
    target = MartExportTarget.from_input(mart)
    mart = target.mart
    zone = target.zone
    cfg = GoldExportConfig.from_env()
    try:
        # A Gold export keeps the exact two-argument call its tests and
        # callers have always made; only Silver adds the zone argument.
        body = export_one_mart(cfg, mart) if zone == DEFAULT_ZONE else export_one_mart(cfg, mart, zone)
    except requests.HTTPError as exc:
        # PR slice C review C-B1: HTTP 409 — the API's single-flight lock
        # (an export of this mart is already running) — is a skip, not a
        # failure. Record it and return without raising; no
        # AssetMaterialization, because nothing was written in this run.
        if exc.response is not None and exc.response.status_code == 409:
            context.log.info(f"skipped {mart!r}: already running (HTTP 409)")
            record_maintenance_run(
                table_name=target.iceberg_table,
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
            table_name=target.iceberg_table,
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
            table_name=target.iceberg_table,
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
        table_name=target.iceberg_table,
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
        context.log_event(AssetMaterialization(asset_key=target.iceberg_table, metadata={"rows": rows}))
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
    # DATA-65: Silver and Gold trims of the same bare mart name are
    # independent histories. Existing rows were all Gold.
    _ch_exec(
        target,
        "ALTER TABLE console.gold_trim_run ADD COLUMN IF NOT EXISTS zone String DEFAULT 'gold'",
    )


def record_gold_trim_run(
    target: ClickHouseTarget | None,
    *,
    mart: str,
    zone: str = DEFAULT_ZONE,
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
        f"id, mart, zone, status, copies_kept, oldest_kept_exported_at, "
        f"copies_removed, snapshots_expired, files_removed, reason, "
        f"started_at, finished_at) VALUES ("
        f"{_sql_str(run_id)}, {_sql_str(mart)}, {_sql_str(zone)}, {_sql_str(status)}, "
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


def expire_all_but_current_snapshot(table: Any) -> int:
    """Expire every snapshot of `table` except its current one, and
    return how many were expired (DATA-2's trim step 4b, extracted for
    `DATA-6` D11: the latest-copy op runs the exact same cleanup after
    its replace, so the two paths share one implementation instead of
    two copies that can drift — AGENTS.md rule 4).

    The ids are read from `table.snapshots()` BEFORE the commit, so the
    caller's `table` object stays a pre-commit view; the caller that
    needs the post-commit state reloads the table itself.
    """
    current_snapshot = table.current_snapshot()
    current_snapshot_id = current_snapshot.snapshot_id if current_snapshot else None
    older_snapshot_ids = [
        s.snapshot_id for s in table.snapshots() if s.snapshot_id != current_snapshot_id
    ]
    if older_snapshot_ids:
        table.maintenance.expire_snapshots().by_ids(older_snapshot_ids).commit()
    return len(older_snapshot_ids)


def remove_gold_orphan_files(cfg: GoldExportConfig, table_name: str, older_than_seconds: int) -> int:
    """Remove the orphan files of one `gold.*` Iceberg table through
    ClickHouse, and return how many data files were deleted (DATA-2's
    trim step 4c, extracted for `DATA-6` D11 — same shared-implementation
    reason as `expire_all_but_current_snapshot` above).

    `table_name` is the catalog's own two-part name (`gold.<mart>`), and
    `older_than_seconds` is passed straight through to the verb: DATA-2
    review BLOCKER 1 — files younger than the age stay, so a concurrent
    writer's in-flight files are never deleted. `cfg` is used only for
    its `ch_auth_settings()`/`lakekeeper_*`/`rustfs_*` fields by the
    maintenance helpers, the same fields the trim reads.
    """
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
        table_name,
        dry_run=False,
        catalog_db=catalog_db,
        older_than_seconds=older_than_seconds,
    )
    return int(orphan_res.get("deleted_data_files_count", 0))


def run_trim_for_mart(
    cfg: GoldExportConfig,
    mart: str,
    context: Any = None,
    zone: str = DEFAULT_ZONE,
) -> dict[str, Any]:
    """Execute trim logic for a single published mart (DATA-2 D1, D3, D5,
    A1). DATA-65: `zone` decides which Iceberg namespace is trimmed and
    which history row is written; Gold is the default."""
    started_at = datetime.datetime.now(datetime.timezone.utc)
    table_name = f"{zone}.{mart}"
    log = context.log if context else None

    # The pause (REL-2 S4 decision D16): the trim expires snapshots and
    # removes orphan files, and both DELETE files. While the maintenance
    # is paused they do not run: the trim is recorded as skipped, with
    # the reason, and the run does not fail. The local backup does not
    # hold the warehouse files, so a restore brings the catalogs back
    # to the moment of the backup — and a trim that deleted a file the
    # restored catalog still names would break the table.
    if maintenance_paused():
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        reason = "maintenance_paused"
        if log:
            log.info(
                "the Gold trim is paused (MAINTENANCE_PAUSED=true): no snapshot was expired and "
                "no orphan file was removed."
            )
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                zone=zone,
                status="skipped",
                reason=reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "skipped", "reason": reason}

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
                zone=zone,
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
                zone=zone,
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
        table = catalog.load_table(table_name)
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to load table {table_name}: {exc}")
        fixed_reason = "catalog_unavailable"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                zone=zone,
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
            log.error(f"failed to scan table {table_name}: {exc}")
        fixed_reason = "table_scan_failed"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                zone=zone,
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
            log.info(f"{table_name} has 0 rows; nothing to trim")
        record_gold_trim_run(
            cfg.ch,
            mart=mart,
            zone=zone,
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
                f"{table_name} has {distinct_count} copies (<= keep_copies {keep_copies}); "
                "nothing to trim"
            )
        record_gold_trim_run(
            cfg.ch,
            mart=mart,
            zone=zone,
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
            f"trimming {table_name}: {distinct_count} copies, keeping {keep_copies}, "
            f"removing {copies_to_remove} (cutoff {cutoff_time})"
        )

    # 4a. Delete rows
    try:
        from pyiceberg.expressions import LessThan

        table.delete(LessThan("_exported_at", cutoff_time))
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to delete rows from {table_name}: {exc}")
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
                zone=zone,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    # 4b. Expire snapshots (shared helper — `DATA-6` D11 reuses the trim's
    # cleanup for the latest-copy op, so this step and that op cannot drift)
    try:
        snapshots_expired = expire_all_but_current_snapshot(table)
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to expire snapshots for {table_name}: {exc}")
        fixed_reason = "expire_snapshots_failed"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                zone=zone,
                status="failed",
                reason=fixed_reason,
                started_at=started_at,
                finished_at=finished_at,
            )
        except Exception as e:
            if log:
                log.warning(f"failed to record gold_trim_run for {mart}: {e}")
        return {"status": "failed", "reason": fixed_reason}

    # 4c. ClickHouse remove_orphan_files (amendment A1, shared helper — same
    # DATA-6 D11 reasoning as the snapshot expiry above)
    files_removed = 0
    try:
        files_removed = remove_gold_orphan_files(cfg, table_name, orphan_min_age_seconds)
    except Exception as exc:
        finished_at = datetime.datetime.now(datetime.timezone.utc)
        if log:
            log.error(f"failed to remove orphan files for {table_name}: {exc}")
        fixed_reason = "orphan_cleanup_failed"
        try:
            record_gold_trim_run(
                cfg.ch,
                mart=mart,
                zone=zone,
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
            zone=zone,
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
    context, mart: Any, export_result: dict[str, Any]
) -> dict[str, Any]:
    """Trim old copies from `gold.{mart}` after a successful export (DATA-2 D1).
    A skipped or failed export does not trim.
    Always returns `export_result` untouched so the export summary is preserved.
    """
    target = MartExportTarget.from_input(mart)
    mart = target.mart
    if export_result.get("skipped") is True:
        context.log.info(f"export of {mart!r} was skipped; skipping trim")
        return export_result

    cfg = GoldExportConfig.from_env()
    try:
        trim_res = (
            run_trim_for_mart(cfg, mart, context=context)
            if target.zone == DEFAULT_ZONE
            else run_trim_for_mart(cfg, mart, context=context, zone=target.zone)
        )
        context.log.info(f"trim result for {target.iceberg_table!r}: {trim_res}")
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


# The `dagster/max_runtime` tag is what run monitoring enforces: a run
# whose process died is cancelled and failed by the daemon, so it does not
# hold a run-queue slot forever (`OPS-35`). `ingest_factory.py` says why it
# is a tag and not `run_monitoring.max_runtime_seconds`.
@job(tags=job_tags("gold_export_job", RUN_LIMIT_ENV, DEFAULT_RUN_LIMIT_SECONDS))
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
# `DATA-66` D9 retired the after-authored sensor that used to launch an
# all-marts run here: a pipeline's own table is now asked for by its
# write (`iceberg_sync.py::request_iceberg_sync` from the authored op),
# and every other switched-on table follows its own sync mode through
# the `iceberg_sync_sensor`. This 04:00 schedule therefore covers only
# the `GOLD_EXPORT_MARTS` entries that are NOT console publications
# (`list_gold_marts`), plus its `ifChanged=true` skip for the rest.
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
#
# `default_status=RUNNING`: a schedule that ships STOPPED silently never
# fires, and from the outside, a stopped schedule looks identical to a
# healthy one (the same reasoning every other schedule/sensor in this
# code location documents). With `GOLD_EXPORT_RUN_TOKEN` unset the
# scheduled POST would still fail — but that is a loud, visible failure
# whose run entry in the Dagster UI is the signal, not a silent one.


gold_export_schedule = ScheduleDefinition(
    job=gold_export_job,
    cron_schedule="0 4 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)

gold_export_defs = Definitions(
    jobs=[gold_export_job],
    schedules=[gold_export_schedule],
)
