"""dagster/dispar_orchestrate/ingest_factory.py -- ONE static `ingest_job`
(config: `connector_id`) plus ONE sensor, `ingest_schedule_sensor`, that
launches it for every connector whose `scheduleCron` came due. The job
shape is `agent_runs.py`'s `agent_run_job` (a static `@job` wrapping a
`config_schema`-driven `@op`), NOT a job per connector: a per-connector
`@job`/`@op` pair built by capturing loop variables as Python default
arguments is exactly the anti-pattern this factory mirrors away from --
Dagster inspects a decorated function's PARAMETERS as op/job inputs, so a
loop-captured default argument is fragile in ways a real Dagster op config
is not.

# Schedules

Connector schedules are NOT Dagster `ScheduleDefinition`s. Those are fixed
at code-load time, so a schedule saved in the console only took effect
after the code location reloaded. Instead `ingest_schedule_sensor` asks
`GET /api/connectors/ingestible?dueAfter=...&dueUntil=...` every 30
seconds which connectors came due since its last evaluation (its cursor).
The API evaluates each cron with `croner`, the same library that computes
the "next run" the console shows, so the two cannot disagree; crons are
UTC, as Dagster schedules here always were. A saved, changed or removed
schedule therefore applies within about a minute, with no reload.

A sensor rather than one every-minute schedule: every schedule of a job is
read by `lakehouse-api` as that job's schedule -- the Pipelines list shows
its cron, and the Overview counts a schedule whose last run predates its
previous fire time as delayed, which an every-minute dispatcher would be
almost always.

A connector whose previous ingest is still queued or running is skipped
for that fire time (Bronze is append-only; two overlapping loads would
double its rows), the same rule `POST .../ingest/run` enforces with 409.

`run_ingest` re-fetches its OWN connector's spec at RUN time, so a run
always uses the connector's current settings, whoever launched it.

# SSRF and the one-connector-per-run invariant

`ssrf_guard.pinned_resolution` monkeypatches `socket.getaddrinfo`
PROCESS-GLOBALLY (see that module's own doc comment) -- justified there as
safe because each `ingest_job` RUN processes exactly one connector. This
factory is what makes that true: `run_ingest`'s op config carries exactly
one `connector_id`, and `_run_one_object` is called once per source object
of that ONE connector, sequentially, inside a single op invocation -- never
two connectors' dials in one run. If a future change ever made one run
iterate multiple connectors concurrently, this invariant (and the guard's
own safety argument) would break; it does not today.

Each per-object load resolves and pins the connector's host via
`ssrf_guard`, using the `ResolvedAddress` each adapter's `build_source`
already computed -- see `_run_one_object` below. `mysql`/`mariadb`/`files`/
`rest` resolve through `socket.getaddrinfo`, so `ssrf_guard.pinned_resolution`
gives them full protection; `mssql` pins via a raw ODBC connection string
(`adapters/sql.py::_mssql_connection_string`) with no resolver call of its
own to intercept.

`mongodb`, `oracle` (a `sql`-adapter `driver`) and `sftp` do NOT go
through that same `pinned_resolution` wrap -- each guards its own dial
already, inside its own `build_source`, before `_run_one_object` ever
sees a result: `mongodb.build_source` wraps the whole document read in
`ssrf_guard.checking_resolver()` (its `resolved` is a LIST of every seed
host, not the single address `pinned_resolution` takes); `oracle.build_source`
dials a resolved IP LITERAL inside its own `checking_resolver()` scope,
so there is no hostname left for a second wrap to protect; `sftp.build_source`
wraps its own `client.connect()` in `pinned_resolution` and reads the
file into memory before returning. `_run_one_object` dispatches all
three to their own arms specifically so it can skip the generic wrap
deliberately for each, rather than double-wrapping or passing a list
where `pinned_resolution` expects one address. `kafka` never reaches
`_run_one_object` at all -- see "Streaming" below.

# Streaming

A `kafka`-adapter connector's `ingest_mode` is `"stream"`, never
`"batch"` (`connector_ingest_mode_check`, widened by
`0043_ingest_tier2_adapters.sql`) -- `run_ingest` dispatches it to
`_run_stream_connector`/`run_kafka_stream_batch` BEFORE the per-object
loop below runs, since a streaming connector has no `sourceObjects` loop
of its own: one micro-batch, from one bounded `consumer.poll()` window,
covers the whole connector. `run_kafka_stream_batch`'s own docstring
covers its SSRF posture (`ssrf_guard_kafka.check_all_advertised_brokers`
plus a `checking_resolver()` scope around the whole poll loop) and its
at-least-once commit ordering.

# Postgres routing

`adapters/sql.py::build_source` deliberately REFUSES `driver in
("postgres", "postgresql")` (see that module's own docstring, "Open
question 2, resolved") -- the seeded `conn-pg-lakehouse` connector
(`rust/migrations/0034_seed_connector_ingest_spec.sql`, `adapter='sql'`,
`driver='postgres'`) is routed HERE instead, through
`dlt_pipeline.BronzeIngestConfig.from_dial` + `dlt_pipeline.run_bronze_ingest`,
which pins Postgres's own way (`engine_kwargs={"connect_args":
{"hostaddr": ...}}` -- `psycopg2`/libpq does not resolve through
`socket.getaddrinfo` at all, so `pinned_resolution` could never protect it).
This is the ONLY other module in this workspace that dials a
connector-supplied Postgres host, so `_run_one_object` dispatches to it
explicitly for `adapter == "sql"` with `dial["driver"] in ("postgres",
"postgresql")`, rather than guessing which of the two paths a `sql`
adapter connector needs.

# Schema changes at the source (`SRC-8`)

A batch SQL table (postgres, mysql/mariadb, mssql, oracle) is OBSERVED
BEFORE it loads: `_run_one_object` reads the columns and primary key from
the reflection `dlt` does while it builds the source (no connection of its
own, so reflection stays inside each driver's SSRF guard; see
`schema_observer.py`), posts them to `lakehouse-api` and obeys the answer.
`wait` records an `ingest_run` row with status `waiting` and returns without
loading and without raising, so the mapped step and the run succeed (feature
decision D7: a table waiting for a person is not a failure). `load` with a
column list loads only those columns. An API that cannot be asked fails the
step and loads nothing (fail closed). The run id the API stores with a change
is the Dagster run id of the mapped step.

# Retries, run reports, pause and left-out columns (`SRC-11`)

A failure is classified once and retried on a 1, 5 and 15 minute schedule
only when it is transient (`ingest_errors.py`, `_fail_or_retry`). The run
tells `lakehouse-api` about itself (`run_reports.py`): `run_ingest` reports the
start and, for the runs it carries out alone (a paused connector, a CDC
connector, a Kafka micro-batch), the end; each mapped step reports each of its
attempts. A batch run's end is reported by the API's own run sensors from
Dagster's confirmed final status. A report that cannot be sent never fails a
load (D5).

`run_ingest` reads the connector's `paused` flag from the same
`GET /api/connectors/ingestible` answer it already fetches, and a paused
connector's run loads nothing and ends `paused` (D13, `SRC-18`).

A source object's `excludedColumns` is applied inside the before-load gate
(`_leave_out_columns`), AFTER the API has been shown the whole table, so a
column left out is never reported as removed at the source (D9). Only the
batch SQL path has such a gate.

# Secrets

Resolved via `secret_resolver.resolve_secret_ref`, using
`secret_map.secret_field_names` to know which named fields the resolved
values become -- NEVER an env var name derived from the connector id (see
`secret_resolver.py`'s own module doc for the bug this replaces).
"""

from __future__ import annotations

import logging
import re
from contextvars import ContextVar
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any, NoReturn
from urllib.parse import urlparse

import requests
from dagster import (
    AssetMaterialization,
    DagsterRunStatus,
    DefaultSensorStatus,
    DynamicOut,
    DynamicOutput,
    Failure,
    Field,
    RetryRequested,
    RunRequest,
    RunsFilter,
    SensorEvaluationContext,
    SensorResult,
    job,
    op,
    sensor,
)

from kafka import KafkaConsumer, TopicPartition
from kafka.structs import OffsetAndMetadata

from dispar_orchestrate import (
    connector_catalog,
    dlt_pipeline,
    ingest_errors,
    run_reports,
    schema_observer,
    secret_resolver,
    ssrf_guard,
)
from dispar_orchestrate.adapters import dedup as dedup_adapter
from dispar_orchestrate.adapters import files as files_adapter
from dispar_orchestrate.adapters import mongodb as mongodb_adapter
from dispar_orchestrate.adapters import oracle as oracle_adapter
from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters import sftp as sftp_adapter
from dispar_orchestrate.adapters import sheets as sheets_adapter
from dispar_orchestrate.adapters import sink as sink_adapter
from dispar_orchestrate.adapters import sql as sql_adapter
from dispar_orchestrate.adapters.kafka import consume_one_batch
from dispar_orchestrate.adapters.sink import LoadPlan, UnsupportedLoadMode, load_via_sink
from dispar_orchestrate.bronze_catalog import record_ingest_offset, record_ingest_run
from dispar_orchestrate.column_gate import UnsupportedColumnType, reject_unsupported_column_types
from dispar_orchestrate.ingest_errors import UnknownAdapter, UnsupportedKafkaAuth
from dispar_orchestrate.op_metadata import source_metadata
from dispar_orchestrate.saas import vendors as saas_vendors
from dispar_orchestrate.secret_map import secret_field_names
from dispar_orchestrate.secret_resolver import SecretRefRejected

_POSTGRES_DRIVERS = ("postgres", "postgresql")

logger = logging.getLogger(__name__)


def _env(name: str, default: str) -> str:
    import os

    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class IngestFactoryConfig:
    """`api_url`/`service_token` for the `ingest:read`-scoped Dagster
    ingest service identity -- the SAME `LAKEHOUSE_API_URL` env var
    `agent_runs.py::AgentRunConfig` and `dlt_pipeline.py::BronzeIngestConfig`
    already read, plus a dedicated `INGEST_SERVICE_TOKEN` (never
    `AGENT_RUN_TOKEN`: that identity is scoped to `agent:manage`, a
    different resource entirely -- `lakehouse_auth::permissions::PermissionSet::has`
    matches resource+action exactly, so one token could not stand in for
    the other even if this module tried)."""

    api_url: str
    service_token: str

    @classmethod
    def from_env(cls) -> "IngestFactoryConfig":
        return cls(
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            service_token=_env("INGEST_SERVICE_TOKEN", ""),
        )


def _headers(cfg: IngestFactoryConfig) -> dict[str, str]:
    return {} if not cfg.service_token else {"Authorization": f"Bearer {cfg.service_token}"}


def _fetch_one_connector(cfg: IngestFactoryConfig, connector_id: str) -> dict[str, Any]:
    """Used at RUN time by `run_ingest` (below) -- a connector launched
    directly (a future `POST .../ingest/run`) may not have existed at this
    code location's last load, so the op re-fetches its OWN connector
    fresh rather than trusting a code-load-time snapshot.

    # Errors

    Propagates `requests.RequestException`/`requests.HTTPError` if the
    API is unreachable or answers non-2xx, and `StopIteration` (as a
    `RuntimeError`-shaped failure Dagster surfaces) if `connector_id`
    names no ingestible connector -- both are legitimate run failures at
    RUN time.
    """
    resp = requests.get(f"{cfg.api_url}/api/connectors/ingestible", headers=_headers(cfg), timeout=10)
    resp.raise_for_status()
    connectors = resp.json()
    return next(c for c in connectors if c["id"] == connector_id)


def _sanitize_name(connector_id: str) -> str:
    """A Dagster-legal schedule name derived from `connector_id`, mirroring
    `agent_runs.py::_schedule_name`'s hyphen-to-underscore mapping."""
    return "".join(c if c.isalnum() or c == "_" else "_" for c in connector_id)


# Dagster's `DynamicOutput.mapping_key` must match `^[A-Za-z0-9_]+$`
# (`dagster._core.definitions.utils.check_valid_chars`). The exact ASCII
# rule from `parts/1b-dagster-one-op-per-unit-and-retries.md`: every
# character outside `[A-Za-z0-9_]` becomes `_`. Precompiled once at
# module load; the same rule `gold_export._sanitize_mapping_key` and
# `maintenance._sanitize_mapping_key` enforce so a connector/target id
# can serve as a schedule name, an op name, or a mapping key
# consistently. Note: `str.isalnum()` would be wrong here -- it is True
# for non-ASCII letters (`"é".isalnum()`, `"²".isalnum()`), which would
# silently produce a non-ASCII mapping key that Dagster then rejects.
_SANITIZE_NON_ASCII = re.compile(r"[^A-Za-z0-9_]")


def _sanitize_target(target: str) -> str:
    """A Dagster-legal `DynamicOutput.mapping_key` derived from a
    source object's `target` -- the exact ASCII `[A-Za-z0-9_]+` rule
    `gold_export._sanitize_mapping_key` and
    `maintenance._sanitize_mapping_key` already enforce. Mirrors
    `_sanitize_name`; kept as a separate helper so each call site is
    explicit about which kind of identifier it is sanitizing."""
    return _SANITIZE_NON_ASCII.sub("_", target)


_ADAPTERS = {
    "sql": sql_adapter,
    "files": files_adapter,
    "rest": rest_adapter,
    "sheets": sheets_adapter,
    "mongodb": mongodb_adapter,
    "sftp": sftp_adapter,
    "saas": saas_adapter,
}


def _host_of(dial: dict) -> str | None:
    host = dial.get("host")
    if host:
        return host
    url = dial.get("baseUrl") or dial.get("endpoint")
    return urlparse(url).hostname if url else None


def _classify_exception(exc: Exception) -> str:
    """Never the raw `str(exc)` -- a driver/HTTP error can carry a host, a
    port, or (in the worst case) a credential fragment. The TYPE NAME
    alone is always safe and still useful for triage (mirrors this
    codebase's `classify_sqlx_error` discipline, Rust-side)."""
    return f"{type(exc).__module__}.{type(exc).__name__}"


# `ingest_errors.is_authentication_refusal` is the one reader of a driver's
# refused sign-in (SRC-11 D2, SRC-19); the old name stays for its callers.
_is_authentication_refusal = ingest_errors.is_authentication_refusal


def _resolve_object_secrets(connector: dict, adapter_name: str, dial: dict) -> dict[str, str]:
    """Resolve `connector["secretRef"]`/`connector["secretRefSecondary"]`
    through the allowlisted resolver ONLY -- never an env var name derived
    from `connector["id"]` (`secret_resolver.py`'s module doc describes
    the bug this closes).

    # Errors

    Raises `SecretRefRejected` if either required ref is missing, not
    allowlisted, or (for `env:` refs) unset in this environment.
    """
    # secret_map.py's `_AUTH_TYPE_KEYED_ADAPTERS` -- rest/kafka/sftp are
    # the three adapters whose secret-field count depends on
    # `dial.auth.type`; every other adapter's mapping key always pairs
    # with `None`, even for a connector whose dial happens to carry an
    # "auth" object for some other reason.
    auth_type = dial.get("auth", {}).get("type") if adapter_name in ("rest", "kafka", "sftp") else None
    if adapter_name == "saas":
        # SRC-3 AD2: the key the vendor table resolves: the browser sign-in's tag,
        # or `integration:<vendor>`. An unknown vendor is a rejected setting
        # (`secret_field_names` raises `ValueError`, classified setting_rejected).
        auth_type = _saas_auth_type(dial)
    fields = secret_field_names(adapter_name, auth_type)
    if not fields:
        # ("kafka", "none"): a PLAINTEXT broker needs no secret at all --
        # never call resolve_secret_ref on a primary slot the connector
        # was never required to set (mirrors secret_resolver.resolve_secrets).
        return {}
    values = [secret_resolver.resolve_secret_ref(connector.get("secretRef"))]
    if len(fields) == 2:
        try:
            values.append(secret_resolver.resolve_secret_ref(connector.get("secretRefSecondary")))
        except secret_resolver.SecretRefRejected:
            if auth_type != "oauth2_authorization_code":
                raise
            # SRC-12 part C: the second credential of an authorization-code
            # connector is the refresh token, written by the person's sign-in.
            # One that is missing or cannot be opened is a connector nobody has
            # signed in to (or whose token was removed): "sign in again", not a
            # rejected setting. The reason is not carried (decision 12).
            raise rest_adapter.OAuthSignInRefused() from None
    return dict(zip(fields, values))


def _saas_auth_type(dial: dict) -> str | None:
    """The `secret_map` key of a `saas` dial: `oauth2_authorization_code`, or
    `integration:<vendor id>` (`None` when the dial has no such sign-in)."""
    kind = (dial.get("auth") or {}).get("type")
    if kind == saas_vendors.OAUTH_AUTH_TYPE:
        return kind
    if kind == "integration":
        found = saas_vendors.VENDORS.get(str(dial.get("vendor", "")))
        return found.secret_key(oauth=False) if found else None
    return None


def _register_in_catalog(connector_id: str, obj: dict) -> None:
    """Put a table that just loaded into the console catalog
    (`connector_catalog.py`). The load itself already succeeded and is
    recorded, so a registration failure is reported, never raised: the data
    is in Bronze either way, and only its Catalog entry is missing."""
    try:
        connector_catalog.register_connector_table(connector_id, obj)
    except Exception as exc:  # noqa: BLE001 -- see docstring: reported, never raised
        print(
            f"WARNING: dispar_orchestrate.ingest_factory: {obj.get('target')!r} loaded, but could not be "
            f"registered in the catalog: {exc}"
        )


def _load_plan(adapter_name: str, obj: dict, refresh: str | None = None) -> LoadPlan:
    """How this source object's rows meet its Bronze table
    (`adapters/sink.py`'s "Load modes"). A source object saved before load
    modes existed replaces: every run used to add the whole source again.

    `incremental` and `incremental_dedup` are offered for the `sql` adapter
    only; `replace_dedup` runs on every batch adapter, through the same
    deduplication on plain rows (SRC-10; `adapters/dedup.py`), and so does `history`
    (`adapters/history.py`: it reads the whole source every run, so it needs no cursor
    and no adapter that can filter by one). A Kafka
    micro-batch never goes through here: it has no source-object load mode
    and `load_via_sink` appends it, which suits new rows by construction. dlt can filter any
    resource by a cursor, but only the `sql_database` source's path (the
    filter pushed into the query, the cursor kept across runs) has been
    exercised; the others are refused rather than assumed to work.

    `refresh` (`"keep"` or `"remove"`, SRC-10 decision 5) is set by a run of one
    table started with a refresh action; see `LoadPlan`.

    # Errors

    Raises `UnsupportedLoadMode`, which `_run_one_object` records as a
    rejection.
    """
    plan = LoadPlan.from_source_object(obj, refresh=refresh)
    # SRC-3 AD6: a `saas` object may load incrementally when its vendor module
    # declares a cursor for it; `adapters/saas.py` refuses the object whose
    # module declares none, or another one, before anything is read.
    if plan.mode in sink_adapter.INCREMENTAL_MODES and adapter_name not in ("sql", "saas"):
        raise UnsupportedLoadMode(
            f"load mode {plan.mode!r} is only available for SQL connectors, not adapter {adapter_name!r}"
        )
    return plan


def _observe_after_load(
    connector_id: str, object_name: str, columns: tuple[sink_adapter.LoadedColumn, ...], run_id: str | None
) -> None:
    """Tell the API what a non-SQL source's table looks like now that it has
    loaded (SRC-8, decision D4: files, REST, `MongoDB`, SFTP and the Kafka
    micro-batch cannot be read before they load).

    What is sent is exactly what the load produced and nothing else: no
    primary key (`[]`: these sources have none to observe), and no column the
    load did not carry -- a column absent from a batch cannot be told from one
    that is empty in it, so this side never reports a removal; the API
    carries the previous columns over for this phase (`routes/schema_changes.rs`,
    `shape_for_phase`).

    THE ONE PLACE THE FAIL-CLOSED RULE DOES NOT APPLY (AGENTS.md principle 3):
    the rows are already in Bronze and recorded as succeeded, and the API
    never asks these sources to hold anything back, so whatever it answers
    (or fails to answer) changes nothing here. A failed post must not turn a
    successful load into a failed run; it is logged as a warning that names
    the exception type (and, for the observer's own errors, their fixed
    message), never a token or a response body.
    """
    if not columns:
        return
    try:
        schema_observer.post_observation(
            schema_observer.ObserverConfig.from_env(),
            connector_id,
            object_name,
            # SRC-8 task 11: these are the pipeline's own (already loaded) names.
            [schema_observer.ReflectedColumn(c.name, c.data_type, c.nullable, loaded_name=c.name) for c in columns],
            [],
            "after_load",
            run_id,
        )
    except Exception as exc:  # noqa: BLE001 -- see docstring: reported, never raised
        detail = str(exc) if isinstance(exc, (schema_observer.ObservationRefused, schema_observer.ObservationUnreachable)) else type(exc).__name__
        logger.warning(
            "%r loaded, but its schema could not be reported to the API (the data is in Bronze): %s",
            object_name,
            detail,
        )


def _schemas_loaded(source_objects: list[dict]) -> list[str]:
    """The schemas a connector already loads at least one table from, in the
    order they first appear: the part of each `<schema>.<table>` name before
    its first dot (the split `BronzeIngestConfig.from_dial` makes). A name
    with no dot has no schema to list."""
    schemas: list[str] = []
    for obj in source_objects:
        schema, dot, _ = obj["name"].partition(".")
        if dot and schema and schema not in schemas:
            schemas.append(schema)
    return schemas


def _discover_new_tables(connector: dict, run_id: str | None) -> bool:
    """Tell the API about tables that appeared in a schema this connector
    already loads from, once per run and only under the policy that asks for
    it (`SRC-8` decision D2: `apply_all`; PostgreSQL, MySQL/MariaDB and SQL
    Server only). Returns whether the API added any, so the caller re-reads
    the connector and loads them in THIS run.

    The listing is a connection of its own, guarded like the loads
    (`adapters/sql.py::list_tables`). It never stops the run: nothing is
    loaded unchecked because of it, and the tables already selected must keep
    loading even when a listing fails (a timeout, a revoked privilege on
    `information_schema`), so a failure is a warning that names the exception
    type (or an observer error's fixed message), never a secret, and the new
    tables are asked for again on the next run.
    """
    dial = connector.get("dial") or {}
    driver = dial.get("driver")
    if (
        connector.get("adapter") != "sql"
        or connector.get("schemaChangePolicy") != "apply_all"
        or driver not in sql_adapter.LISTABLE_DRIVERS
    ):
        return False
    objects = connector.get("sourceObjects") or []
    schemas = _schemas_loaded(objects)
    if not schemas:
        return False
    try:
        secrets = _resolve_object_secrets(connector, "sql", dial)
        listed = sql_adapter.list_tables(dial, secrets, schemas)
        selected = {obj["name"] for obj in objects}
        new = [
            f"{schema}.{table}"
            for schema in schemas
            for table in listed.get(schema, [])
            if f"{schema}.{table}" not in selected
        ]
        if not new:
            return False
        cfg = schema_observer.ObserverConfig.from_env()
        size = schema_observer.MAX_TABLES_PER_REQUEST
        added = False
        for start in range(0, len(new), size):
            answer = schema_observer.post_new_tables(cfg, connector["id"], new[start : start + size], run_id)
            added = added or bool(answer.added)
        return added
    except Exception as exc:  # noqa: BLE001 -- see docstring: reported, never raised
        detail = (
            str(exc)
            if isinstance(exc, (schema_observer.ObservationRefused, schema_observer.ObservationUnreachable))
            else _classify_exception(exc)
        )
        logger.warning(
            "new tables of connector %r could not be looked for (the tables it already has still load): %s",
            connector.get("id"),
            detail,
        )
        return False


@dataclass
class AttemptNotes:
    """What one attempt at one table learned that its return value does not
    say, for the run report: the `ingest_run` status `_record` wrote
    (`waiting`, `unsupported`, `cleared`, ...) and how many columns the source
    object's `excludedColumns` left out of the load."""

    status: str | None = None
    columns_left_out: int = 0


# Set by `ingest_source_object` around its call of `_run_one_object` and read
# back after it. A context variable rather than a new return value or
# parameter: `_run_one_object` returns the row count that callers and tests
# rely on, and a direct call (no step around it) simply has nobody to tell.
_NOTES: ContextVar[AttemptNotes | None] = ContextVar("ingest_attempt_notes", default=None)


def _leave_out_columns(
    decision: schema_observer.Decision,
    reflected: schema_observer.ReflectedTable,
    excluded: frozenset[str],
) -> tuple[schema_observer.Decision, int]:
    """Apply a source object's `excludedColumns` to the API's decision (D9).

    Runs AFTER the API was shown the whole table, so what is left out is never
    reported as removed at the source. The columns to load are the ones the
    decision names (or all reflected ones when it names none) minus the
    excluded ones, by the source's own spelling. Returns the decision to obey
    and how many columns were actually left out (a name the table does not
    have counts for nothing: the API does not check names at save time).

    # Errors

    Raises `schema_observer.ObservationRefused` when nothing would be left to
    load (every column excluded): not retried, and nothing is loaded.
    """
    if decision.action != "load" or not excluded:
        return decision, 0
    names = [c.name for c in reflected.columns] if decision.columns is None else list(decision.columns)
    kept = [name for name in names if name not in excluded]
    left_out = len(names) - len(kept)
    if left_out == 0:
        return decision, 0
    if not kept:
        raise schema_observer.ObservationRefused("every column of the table is left out, so there is nothing to load")
    return schema_observer.Decision(action=decision.action, columns=kept, changes=decision.changes), left_out


# What an `ingest_run` row says when a table waits for a schema-change
# decision (SRC-8, D7). Fixed text: nothing from the source or the API.
_WAITING_MESSAGE = "waiting for a decision on a schema change at the source"


def _make_gate(connector: dict, obj: dict, run_id: str | None):
    """The `before_load` observation for one source object (SRC-8, D4): what
    `dlt` reflected goes to the API and its decision comes back."""

    excluded_names = obj.get("excludedColumns") or []
    excluded = frozenset(n for n in excluded_names if isinstance(n, str))

    def gate(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
        decision = schema_observer.post_observation(
            schema_observer.ObserverConfig.from_env(),
            connector["id"],
            obj["name"],
            reflected.columns,
            reflected.primary_key,
            "before_load",
            run_id,
        )
        # SRC-11 D9: the observation above saw the whole table; only what is
        # loaded is narrowed.
        decision, left_out = _leave_out_columns(decision, reflected, excluded)
        notes = _NOTES.get()
        if notes is not None:
            notes.columns_left_out = left_out
        return decision

    return gate


def _build_observed(build, gate):
    """Build a SQL source, observe its reflected table, and obey the answer.

    `build(table_adapter_callback)` builds the source with that callback.
    Returns the source build result to load -- built a second time, keeping
    only the listed columns, when the API said so (the second reflection
    runs under the same guard as the first) -- or `None` when the API said
    `wait`.

    # Errors

    Raises `schema_observer.ReflectionMissing` if the build reflected no
    table (nothing to observe: refused rather than loaded unchecked), and
    whatever `gate` raises.
    """
    collector = schema_observer.ReflectionCollector()
    result = build(collector.callback)
    decision = gate(collector.only())
    if decision.action == "wait":
        return None
    if decision.columns is not None:
        result = build(schema_observer.keep_only(decision.columns))
    return result


# SRC-10 decision 5: the actions a run of ONE table can carry (the run config's
# `action`, beside `object`). A scheduled run and an old API send neither.
ACTION_REFRESH_KEEP = "refresh_keep"
ACTION_REFRESH_REMOVE = "refresh_remove"
ACTION_CLEAR = "clear"
ACTIONS = (ACTION_REFRESH_KEEP, ACTION_REFRESH_REMOVE, ACTION_CLEAR)
_REFRESH_OF_ACTION = {ACTION_REFRESH_KEEP: "keep", ACTION_REFRESH_REMOVE: "remove"}


def _single_table_request(connector_id: str, object_name: Any, action: Any) -> tuple[str, str] | None:
    """The `(object, action)` a run was started with, or `None` for an ordinary run
    of every table. Both or neither: an `object` without an `action` (or the reverse)
    is a run the API would never send, so it fails closed instead of loading
    something nobody asked for.

    # Errors

    Raises `Failure(allow_retries=False)` for one of the pair alone, an `action` that is
    not one of `ACTIONS`, or a blank `object`."""
    if object_name is None and action is None:
        return None
    if (
        not isinstance(object_name, str)
        or not object_name.strip()
        or action not in ACTIONS
    ):
        raise Failure(
            f"connector {connector_id!r}: a run of one table needs both an object and an action "
            f"({', '.join(ACTIONS)})",
            allow_retries=False,
        )
    return object_name, action


def _run_one_object(
    connector: dict, obj: dict, run_id: str | None = None, action: str | None = None
) -> int | None:
    """Ingest one source object (table/endpoint/sheet range) for one
    connector, recording the outcome via `record_ingest_run` in EVERY
    case -- a rejection (bad secret ref, SSRF-blocked host, an
    unsupported nested column type) and any other driver/HTTP/sink
    failure all get a row, never a silently-invisible run.

    # Errors

    Re-raises whatever it catches (`SecretRefRejected`,
    `ssrf_guard.SsrfBlocked`, `UnsupportedColumnType`,
    `UnsupportedLoadMode`, `schema_observer.ObservationRefused`, or any other
    exception a driver/HTTP call/sink raises, `ObservationUnreachable`
    included) after recording it -- this
    function's own run (the Dagster op) must still fail visibly, not just
    the governance-surface row. The one thing that does NOT raise is a table
    the API told to `wait` (SRC-8, D7): it is recorded as `waiting` and the
    function returns `None`.

    `run_id` is the Dagster run id sent with the schema observation.

    `action` (SRC-10 decision 5) is set only for a run of one table started with an
    action. `refresh_keep` and `refresh_remove` load as usual under a plan that
    first forgets the table's cursor and deduplication marker (and, for `remove`,
    empties the table: see `LoadPlan`). `clear` empties the table, forgets the same
    state and loads NOTHING: it resolves no secret, dials no source, makes no
    schema observation (so the table's baseline in `connector_source_schema` is not
    touched; it describes the source, not the table) and records a run with status
    `cleared` and rows 0.
    """
    connector_id, adapter_name, dial = connector["id"], connector["adapter"], connector["dial"]
    job_name = "ingest_job"
    started_at = datetime.now(timezone.utc).isoformat()

    def _record(*, rows, status, error=""):
        notes = _NOTES.get()
        if notes is not None:
            notes.status = status
        record_ingest_run(
            connector_id=connector_id,
            job=job_name,
            object_name=obj["name"],
            rows=rows,
            started_at=started_at,
            ended_at=datetime.now(timezone.utc).isoformat(),
            status=status,
            error=error,
        )

    if action == ACTION_CLEAR:
        try:
            sink_adapter.clear_table(
                obj["target"],
                sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env()),
            )
        except Exception as exc:  # noqa: BLE001 -- every failure still gets a row (never silently invisible)
            _record(rows=None, status="failed", error=_classify_exception(exc))
            raise
        _record(rows=0, status="cleared")
        return 0

    try:
        secrets = _resolve_object_secrets(connector, adapter_name, dial)
    except SecretRefRejected as exc:
        _record(rows=None, status="rejected", error=str(exc))
        raise
    except rest_adapter.OAuthSignInRefused as exc:
        # SRC-12 part C: nobody has signed in (no refresh token to read). A failure,
        # not a rejected setting; the row names the type only, like every other.
        _record(rows=None, status="failed", error=_classify_exception(exc))
        raise

    try:
        reject_unsupported_column_types(obj.get("columns", []))
        plan = _load_plan(adapter_name, obj, refresh=_REFRESH_OF_ACTION.get(action or ""))
        gate = _make_gate(connector, obj, run_id)

        if adapter_name == "sql" and dial.get("driver") in _POSTGRES_DRIVERS:
            # See this module's docstring, "Postgres routing" -- pinned
            # via hostaddr inside run_bronze_ingest itself, never through
            # ssrf_guard.pinned_resolution (psycopg2/libpq does not
            # resolve through socket.getaddrinfo).
            outcome = dlt_pipeline.run_bronze_ingest(
                dlt_pipeline.BronzeIngestConfig.from_dial(dial, secrets, [obj]),
                plan,
                gate=gate,
            )
            if outcome.get("waiting"):
                _record(rows=None, status="waiting", error=_WAITING_MESSAGE)
                return None
            _record(rows=outcome["rows"], status="succeeded")
            _register_in_catalog(connector_id, obj)
            return outcome["rows"]

        if adapter_name == "sql" and dial.get("driver") == "oracle":
            # adapters/oracle.py's own build_source ALREADY dials the
            # resolved IP literal inside its own `ssrf_guard.checking_resolver()`
            # scope (see that module's docstring) before this function
            # ever sees the result -- `resolved` here is real, but wrapping
            # the load below in `ssrf_guard.pinned_resolution` too would
            # be a second, redundant guard around a connection that (a)
            # already happened at schema-reflection time and (b) dials an
            # IP literal, never `dial["host"]` by name again. `oracle` is
            # therefore dispatched here, never through the generic
            # `_ADAPTERS` path below, precisely so this arm can skip that
            # wrap deliberately instead of it silently never firing (the
            # generic path's `_host_of` would find `dial["host"]` and
            # apply it "by accident" otherwise).
            result = _build_observed(
                lambda callback: oracle_adapter.build_source(dial, secrets, [obj], table_adapter_callback=callback),
                gate,
            )
            if result is None:
                _record(rows=None, status="waiting", error=_WAITING_MESSAGE)
                return None
            outcome = sink_adapter.load_via_sink(
                result.source,
                obj["target"],
                sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env()),
                plan,
            )
            _record(rows=outcome.rows, status="succeeded")
            _register_in_catalog(connector_id, obj)
            return outcome.rows

        observed_before = False
        adapter = _ADAPTERS.get(adapter_name)
        if adapter is None:
            raise UnknownAdapter(f"ingest_factory: no adapter dispatch for adapter={adapter_name!r}")

        if adapter_name == "sheets":
            result = adapter.build_source(dial, secrets)
            if not result.supported:
                _record(rows=None, status="unsupported", error=result.reason or "")
                return None
            source, resolved = result.source, None
        elif adapter_name == "mongodb":
            # mongodb.build_source guards its OWN read: `_collection_rows`
            # wraps the whole document iteration in
            # `ssrf_guard.checking_resolver()` (see that module's
            # docstring). `result.resolved` is a LIST of every seed
            # host's `ResolvedAddress` -- NOT the single address
            # `ssrf_guard.pinned_resolution` takes -- so `resolved` is set
            # to `None` here deliberately, the same way the `sheets` arm
            # above sets it, to keep this connector on the "already
            # guarded, do not wrap again" path below rather than passing
            # a list where one address is expected.
            result = adapter.build_source(dial, secrets, [obj])
            source, resolved = result.sources[obj["target"]], None
        elif adapter_name == "sftp":
            # sftp.build_source already wraps its OWN `client.connect(...)`
            # in `ssrf_guard.pinned_resolution(spec["host"], resolved)`
            # before it returns, AND reads the file into memory eagerly
            # inside that same call (see that module's docstring) -- by
            # the time this function has a source, the guarded dial has
            # already happened and there is no further network access
            # left for a second wrap to protect. `resolved` is set to
            # `None` here for the same "do not double-wrap" reason the
            # `mongodb` arm above states.
            result = adapter.build_source(dial, secrets, [obj])
            source, resolved = result.sources[obj["target"]], None
        elif adapter_name == "sql":
            # mysql/mariadb/mssql (SRC-8): the reflection that feeds the
            # observation happens inside `build_source`, under that driver's
            # own pin (`adapters/sql.py`), before any row is read.
            result = _build_observed(
                lambda callback: adapter.build_source(dial, secrets, [obj], table_adapter_callback=callback),
                gate,
            )
            if result is None:
                _record(rows=None, status="waiting", error=_WAITING_MESSAGE)
                return None
            source, resolved = result.source, result.resolved
        elif adapter_name == "saas":
            # SRC-3: the vendor module reads through a `SaasSession`, which
            # resolves and pins every host itself (link rule, AD3), so `resolved`
            # stays None and nothing is wrapped here. A vendor that describes
            # its fields is observed BEFORE the load through `gate` (AD7).
            saas_result = adapter.build_source(dial, secrets, [obj], connector_id=connector_id, gate=gate)
            if saas_result is None:
                _record(rows=None, status="waiting", error=_WAITING_MESSAGE)
                return None
            source, resolved = saas_result.source, None
            observed_before = saas_result.observed_before_load
        elif adapter_name == "rest":
            # SRC-12 part C: the connector's id lets a REST load report a refresh
            # token the provider rotated (`adapters/rest.py`, OAUTH).
            result = adapter.build_source(dial, secrets, [obj], connector_id=connector_id)
            source, resolved = result.source, result.resolved
        else:
            result = adapter.build_source(dial, secrets, [obj])
            source, resolved = result.source, result.resolved

        def _load():
            sink_config = sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env())
            return sink_adapter.load_via_sink(source, obj["target"], sink_config, plan)

        host = _host_of(dial)
        if resolved is not None and host is not None:
            with ssrf_guard.pinned_resolution(host, resolved):
                outcome = _load()
        else:
            outcome = _load()
        _record(rows=outcome.rows, status="succeeded")
        _register_in_catalog(connector_id, obj)
        if adapter_name != "sql" and not observed_before:
            # files / rest / mongodb / sftp (and a saas vendor that does not
            # describe its fields): observed AFTER the load (SRC-8, D4). sql
            # and a describing saas vendor were observed before it; sheets
            # returned above.
            _observe_after_load(connector_id, obj["name"], outcome.columns, run_id)
        return outcome.rows
    except (
        ssrf_guard.SsrfBlocked,
        UnsupportedColumnType,
        UnsupportedLoadMode,
        # SRC-10: a deduplicating load that cannot be done as asked (a row with
        # no key value, too many keys). Fixed message: counts and names only.
        dedup_adapter.DedupRefused,
        schema_observer.ObservationRefused,
        # SRC-3: a vendor with no module in this build, or a dial/sign-in the
        # vendor does not accept. Fixed messages.
        saas_vendors.UnsupportedVendor,
        saas_adapter.SaasSetupRefused,
    ) as exc:
        _record(rows=None, status="rejected", error=str(exc))
        raise
    except Exception as exc:  # noqa: BLE001 -- every OTHER failure still gets a row (never silently invisible)
        _record(rows=None, status="failed", error=_classify_exception(exc))
        raise


def kafka_security_kwargs(spec: dict, secrets: dict) -> dict:
    """The `KafkaConsumer` security settings the dial's `auth` declares.

    A connector declared `sasl_plain` must connect with SASL/PLAIN over TLS;
    one declared `none` connects in plaintext. Building the consumer without
    these made every connector plaintext and unauthenticated whatever it
    declared -- against a permissive broker it would connect without the
    credentials it had just resolved, and without TLS. TLS here verifies
    the broker's certificate against its hostname (`ssl_check_hostname`),
    which still holds under `checking_resolver`: that guard checks each
    resolved address, it does not replace the name the client connects by.

    Never puts a secret into an error message.

    # Errors

    Raises `UnsupportedKafkaAuth` for any auth type other than `none` and
    `sasl_plain`, and `KeyError` naming only the missing FIELD if a
    `sasl_plain` dial lacks its username or the password was not resolved.
    """
    auth = spec.get("auth") or {}
    auth_type = auth.get("type")
    if auth_type == "none":
        return {"security_protocol": "PLAINTEXT"}
    if auth_type == "sasl_plain":
        return {
            "security_protocol": "SASL_SSL",
            "sasl_mechanism": "PLAIN",
            "sasl_plain_username": auth["username"],
            "sasl_plain_password": secrets["password"],
            "ssl_check_hostname": True,
        }
    raise UnsupportedKafkaAuth(f"kafka auth type {auth_type!r} is not supported by this consumer")


def run_kafka_stream_batch(
    *,
    connector_id: str,
    spec: dict,
    secrets: dict,
    source_objects: list[dict],
    consumer: "KafkaConsumer | None" = None,
    run_id: str | None = None,
) -> int:
    """One scheduled micro-batch for a `kafka`-adapter, `ingest_mode='stream'`
    connector. At-least-once, stated explicitly:
    `consume_one_batch` returns rows plus the last offset
    seen per partition, WITHOUT committing anything. This function calls
    `load_via_sink` on those rows FIRST -- and only once that write
    succeeds does it commit the batch's offsets, both to Kafka's own
    broker-side consumer-group offset (`consumer.commit`, `enable_auto_commit
    =False` so nothing commits on its own) AND to `bronze_meta.ingest_offset`
    (`record_ingest_offset`, this build's own durable record, since a
    fresh `KafkaConsumer` is constructed per scheduled run rather than kept
    alive between runs). A crash between the sink write and either commit
    call re-delivers the same batch on the next scheduled run -- a
    deliberate, documented at-least-once gap (`adapters/sink.py`'s Iceberg
    append path is not deduplicated), never a silent skip.

    `run_id` is the Dagster run id sent with the after-load schema observation.

    Returns the rows the micro-batch loaded (0 for an empty poll), for the run
    report (SRC-11 D5).

    A micro-batch that loaded also registers (or refreshes) its Bronze
    table in the console catalog (`_register_in_catalog`), like every batch
    adapter's table; a registration failure is reported, never raised.

    An empty batch (`not batch.rows`) returns without writing or
    committing anything -- there is nothing to commit an offset FOR, and
    (mirroring `_run_one_object`'s own per-object-only recording) nothing
    is recorded to governance for an empty poll: an empty micro-batch is
    not an outcome, it is nothing having happened this run.

    GOVERNANCE (the bug this function used to have): unlike every batch
    adapter path (`_run_one_object`'s own `_record` calls, via
    `record_ingest_run`), this function used to record NOTHING --
    `/api/governance/ingest-runs` showed no row for a Kafka connector's
    micro-batch, success or failure, including an SSRF refusal of an
    advertised broker (`consume_one_batch` raising `ssrf_guard.SsrfBlocked`
    from inside `check_all_advertised_brokers`/`checking_resolver`). Now
    records exactly one run per non-empty micro-batch, matching
    `_run_one_object`'s three statuses: `"succeeded"` (the real row count
    `load_via_sink`'s own `SinkResult.rows` measured -- never a fabricated
    number), `"rejected"` (an `ssrf_guard.SsrfBlocked` from the poll
    itself), or `"failed"` (any other exception, classified by TYPE NAME
    only via `_classify_exception`, never the raw message -- same
    discipline as `_run_one_object`'s own catch-all).

    `consumer` is injectable (DEFAULT `None` builds a real
    `KafkaConsumer` from `spec`) so a test can drive this function with a
    fake driver and touch no network at all -- the same "inject the thing
    that would otherwise touch the network" discipline
    `adapters/mongodb.py::_collection_rows`'s `checking_resolver` param
    and `adapters/kafka.py::consume_one_batch`'s `resolve_checked`/
    `checking_resolver` params already establish in this workstream. When
    this function owns the consumer (constructed it itself), it also
    closes it in `finally`; a caller-supplied consumer is the caller's to
    close.
    """
    owns_consumer = consumer is None
    if owns_consumer:
        consumer = KafkaConsumer(
            bootstrap_servers=spec["bootstrapServers"],
            group_id=spec["groupId"],
            enable_auto_commit=False,
            value_deserializer=lambda v: v,  # raw bytes -- consume_one_batch does its own json.loads
            **kafka_security_kwargs(spec, secrets),
        )
        consumer.subscribe([spec["topic"]])
    topic = spec.get("topic", "")
    job_name = "ingest_job"
    object_name = source_objects[0]["target"] if source_objects else topic
    started_at = datetime.now(timezone.utc).isoformat()

    def _record(*, rows, status, error=""):
        record_ingest_run(
            connector_id=connector_id,
            job=job_name,
            object_name=object_name,
            rows=rows,
            started_at=started_at,
            ended_at=datetime.now(timezone.utc).isoformat(),
            status=status,
            error=error,
        )

    try:
        try:
            batch = consume_one_batch(consumer, topic=topic, max_seconds=spec.get("microBatchSeconds", 60))
            if not batch.rows:
                return 0
            sink_config = sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env())
            outcome = load_via_sink(batch.rows, source_objects[0]["target"], sink_config)
            # Committed ONLY after the sink write above returned
            # successfully (an exception there propagates out of this
            # try block before this point is ever reached -- see this
            # function's own docstring).
            consumer.commit(
                {
                    TopicPartition(topic, partition): OffsetAndMetadata(offset + 1)
                    for partition, offset in batch.offsets_to_commit.items()
                }
            )
            for partition, offset in batch.offsets_to_commit.items():
                record_ingest_offset(connector_id, topic, partition, offset)
            _record(rows=outcome.rows, status="succeeded")
            # The same Catalog entry every batch adapter's table gets
            # (`_run_one_object`): without it a topic's Bronze table held
            # rows but never showed in the Catalog. An empty poll and a
            # failed batch wrote nothing, so they register nothing.
            _register_in_catalog(
                connector_id,
                {"name": source_objects[0].get("name") or topic, "target": source_objects[0]["target"]},
            )
            # SRC-8 (D4): a micro-batch is observed AFTER it loads, like every
            # source whose columns cannot be read first. The offsets are
            # already committed; a failed post changes nothing (see
            # `_observe_after_load`).
            _observe_after_load(connector_id, source_objects[0].get("name") or topic, outcome.columns, run_id)
            return outcome.rows
        except ssrf_guard.SsrfBlocked as exc:
            _record(rows=None, status="rejected", error=str(exc))
            raise
        except Exception as exc:  # noqa: BLE001 -- every OTHER failure still gets a row (never silently invisible)
            _record(rows=None, status="failed", error=_classify_exception(exc))
            raise
    finally:
        if owns_consumer:
            consumer.close()


def _run_stream_connector(connector: dict, run_id: str | None = None) -> int:
    """Dispatch a `kafka`-adapter, `ingest_mode="stream"` connector
    (`connector_ingest_mode_check`, widened to admit `"stream"` by
    `0043_ingest_tier2_adapters.sql`) to `run_kafka_stream_batch` -- the
    ONLY connector shape that function is built for (see its own
    docstring: "one scheduled micro-batch for a kafka-adapter,
    ingest_mode='stream' connector"). It is never reached through
    `_run_one_object`/`_ADAPTERS`: a streaming connector has no per-object
    loop, one micro-batch covers every `sourceObjects` entry via a single
    `KafkaConsumer` subscription.

    Secrets resolve through the SAME `secret_resolver.resolve_secrets`
    helper (never `_resolve_object_secrets`, which is `_run_one_object`'s
    own private helper) -- `("kafka", "none")`'s empty fields tuple means
    a PLAINTEXT broker resolves zero secrets, never a `SecretRefRejected`
    for a `secretRef` the connector was never required to set.
    """
    dial = connector["dial"]
    auth_type = dial.get("auth", {}).get("type")
    secrets = secret_resolver.resolve_secrets(
        "kafka", auth_type, connector.get("secretRef"), connector.get("secretRefSecondary")
    )
    return run_kafka_stream_batch(
        connector_id=connector["id"],
        spec=dial,
        secrets=secrets,
        source_objects=connector.get("sourceObjects", []),
        run_id=run_id,
    )


def _retry_number(context: Any) -> int:
    """Which attempt of this step is running (0 for the first). A context built
    for a direct call of the op in a unit test has no step behind it and so no
    attempt count: it is the first attempt."""
    try:
        return int(context.retry_number)
    except AttributeError:
        return 0


def _fetch_connector_for_run(cfg: IngestFactoryConfig, connector_id: str, retry_number: int) -> dict[str, Any]:
    """`_fetch_one_connector` for `run_ingest`, with its failures named by the
    one classification (SRC-11 D2): the product's own API not answering is
    `api_unreachable` and is retried; an answer that cannot run the connector
    (a 4xx, a connector that is no longer ingestible) is not.

    # Errors

    Raises what `_fail_or_retry` raises.
    """
    try:
        return _fetch_one_connector(cfg, connector_id)
    except requests.HTTPError as exc:
        status = getattr(exc.response, "status_code", None)
        if isinstance(status, int) and 400 <= status < 500:
            refused: Exception = ingest_errors.ApiRefused("the API refused the connector request")
        else:
            refused = ingest_errors.ApiUnreachable("the API failed on the connector request")
        _fail_or_retry(refused, label=repr(connector_id), retry_number=retry_number)
    except StopIteration:
        _fail_or_retry(
            ingest_errors.ApiRefused("the connector is not ingestible"), label=repr(connector_id), retry_number=retry_number
        )
    except requests.RequestException:
        _fail_or_retry(
            ingest_errors.ApiUnreachable("the API could not be reached"), label=repr(connector_id), retry_number=retry_number
        )


@dataclass(frozen=True)
class RetryDecision:
    """What `_decide` says about a failed attempt: its verdict, and the wait
    before the next attempt when there is one (`None`: the step ends)."""

    verdict: ingest_errors.Classified
    wait_seconds: int | None


def _decide(exc: BaseException, retry_number: int) -> RetryDecision:
    """Classify `exc` and pick the wait for the next attempt (SRC-11 D1, D2)."""
    verdict = ingest_errors.classify(exc)
    wait = ingest_errors.RetrySettings.from_env().delay_for(retry_number) if verdict.retried else None
    return RetryDecision(verdict, wait)


def _fail_or_retry(
    exc: Exception, *, label: str, retry_number: int, decision: RetryDecision | None = None
) -> NoReturn:
    """Turn a failed attempt into what Dagster should do next (SRC-11 D1, D2).

    `ingest_errors.classify` is the only place that decides. A transient
    class with a retry left raises `RetryRequested` carrying that attempt's
    wait (`INGEST_RETRY_DELAYS_SECONDS`, 1, 5 and 15 minutes by default);
    Dagster's `RetryPolicy` cannot express a different wait per attempt. The
    step is the unit that retries, so for the mapped per-table step only the
    failing table is tried again; the tables that loaded are not touched.

    Everything else ends the step: a refused credential with a fixed text
    (nothing from the driver: it names the user and the client address), the
    other classes with the message their own type gave (as before SRC-11), a
    transient class whose retries are used up with the class's fixed phrase,
    and an unexplained failure (`other`) as itself, so its traceback is in the
    step log.

    `decision` is the caller's own `_decide(exc, retry_number)` when it has
    already used it (to report the attempt), so both see the same answer.

    # Errors

    Always raises: `RetryRequested`, `Failure(allow_retries=False)`, or `exc`.
    """
    decision = decision or _decide(exc, retry_number)
    verdict, wait = decision.verdict, decision.wait_seconds
    if wait is not None:
        raise RetryRequested(
            max_retries=ingest_errors.RetrySettings.from_env().max_retries, seconds_to_wait=wait
        ) from exc
    phrase = ingest_errors.CLASS_PHRASES[verdict.error_class]
    if verdict.error_class == ingest_errors.SIGN_IN_REFUSED:
        raise Failure(description=f"ingest of {label}: {phrase}", allow_retries=False) from exc
    if verdict.retried:
        raise Failure(
            description=f"ingest of {label}: {phrase}; gave up after {retry_number + 1} attempts",
            allow_retries=False,
        ) from exc
    if verdict.error_class != ingest_errors.OTHER:
        raise Failure(description=f"ingest of {label}: {exc}", allow_retries=False) from exc
    raise exc


def _trigger_of(context: Any, action: Any) -> str:
    """What started this run, one of `run_reports.TRIGGERS` (SRC-11 D7).

    A run of one table carries its action in the run config, which says
    `refresh` or `clear` whoever pressed the button. Otherwise the run's
    `lakehouse/trigger` tag, which `ingest_schedule_sensor` sets to `schedule`.
    A run launched through `POST .../ingest/run` carries `user` (the API tags
    its launches, SRC-11 review 1 SHOULD-FIX 1); one launched from the
    orchestrator's own screen carries no tag and is reported as `other`: this
    function invents nothing.
    """
    if action == ACTION_CLEAR:
        return "clear"
    if action in _REFRESH_OF_ACTION:
        return "refresh"
    try:
        tag = (context.dagster_run.tags or {}).get("lakehouse/trigger")
    except Exception:  # noqa: BLE001 -- a directly invoked op has no run, and Dagster raises its own error type
        tag = None
    return tag if tag in run_reports.TRIGGERS else "other"


def _table_label(connector: dict) -> str:
    """The name a stream connector's one table goes by in a report: its first
    source object, else its topic, else the connector."""
    objects = connector.get("sourceObjects") or []
    first = objects[0].get("name") if objects else None
    return first or (connector.get("dial") or {}).get("topic") or connector["id"]


@op(
    config_schema={
        "connector_id": Field(str, description="The connector.id (adapter IS NOT NULL) to ingest."),
        # SRC-10 decision 5, both OPTIONAL so a scheduled run and an API that
        # predates them still validate: a run of ONE table with an action.
        "object": Field(
            str,
            is_required=False,
            description="Source object name (sourceObjects[].name) of the one table to run; needs `action`.",
        ),
        "action": Field(
            str,
            is_required=False,
            description="refresh_keep | refresh_remove | clear, for the table named in `object`.",
        ),
    },
    out=DynamicOut(),
    # SRC-11 D1: no `retry_policy`. `_fail_or_retry` raises `RetryRequested` with
    # the attempt's own wait (a policy cannot), and everything else ends the step.
    tags=source_metadata(
        "dispar_orchestrate/ingest_factory.py::run_ingest",
        reads=["Connector source objects (per its ingest spec)"],
        # `writes` is the side-effect of this op: a `DynamicOutput` per
        # source object, which becomes a mapped
        # `ingest_source_object[<target>]` step (and a cdc/stream
        # connector yields zero). Declared here so the every-op walk
        # test that requires non-empty `writes` does not flag this
        # fan-out as having no writes.
        writes=["DynamicOutput per sourceObjects entry (batch connectors only)"],
    ),
)
def run_ingest(context) -> Any:
    """PART B of `parts/1b-dagster-one-op-per-unit-and-retries.md`: the
    fan-out for `ingest_job`. Three branches, all keeping the existing
    `connector_id` op config:

    1. `adapter == "cdc"` -- Debezium's compose service owns the
       connector's ingestion end-to-end (ADR 0008's
       `snapshot.mode=initial`), so this op has no body to run.
       Yields no `DynamicOutput`s; the job completes after this op.
    2. `ingestMode == "stream"` -- a single bounded
       `_run_stream_connector(connector)` call owns the connector's
       rows (Kafka micro-batch loop), not the per-object loop. Yields
       no `DynamicOutput`s; the job completes after this op.
    3. Otherwise (batch) -- yields one `DynamicOutput` per
       `sourceObjects` entry, with `mapping_key` = the sanitized
       `target` (the same `[A-Za-z0-9_]+` rule
       `gold_export._sanitize_mapping_key` enforces, refactored to the
       shared `_sanitize_target` helper below).

    The connector dict carried in each `DynamicOutput` is the SAME one
    `_fetch_one_connector` returned -- it holds `secretRef` STRING
    REFERENCES, never resolved secret values, so Dagster's IO manager
    persisting the output does not leak a resolved secret. The
    `ingest_source_object` mapped op resolves secrets INSIDE its own
    body via the existing `_resolve_object_secrets`/`secret_resolver`
    path; the fan-out's payload never crosses that line.

    Before the fan-out, a connector on the `apply_all` schema-change policy
    asks the API to add the tables that appeared in a schema it already
    loads (`_discover_new_tables`, `SRC-8` decision D2). Tables the API adds
    are loaded in this same run: the connector is read again after the add, so
    the fan-out sees them. A failure of that step never stops the run.

    Two source objects whose sanitized targets collide raise
    `Failure(allow_retries=False)` -- they would otherwise collapse
    into one mapped step doing two objects' work, which Dagster's
    graph would refuse at build time and which this fan-out catches
    first with both names in the message.

    SRC-10 decision 5: with `object` and `action` in the config the op fans out over
    that ONE source object only, and the mapped step carries the action
    (`refresh_keep`, `refresh_remove`, `clear`; see `_run_one_object`). New tables
    are not discovered in such a run (it touches one table). A cdc or stream
    connector, and an object the connector does not select, fail closed with
    `Failure(allow_retries=False)`; the API refuses both before launching.
    """
    connector_id: str = context.op_config["connector_id"]
    # SRC-11 D5, D7: the run's start, before anything can fail. A retry of this
    # op sends it again; the API answers `recorded: false` and changes nothing.
    reporter = run_reports.RunReporter.for_run(connector_id, context.run_id)
    trigger = _trigger_of(context, context.op_config.get("action"))
    # SRC-11 review 1 BLOCKER 2: a refresh or clear names its table in the
    # start report, so the run list can say "Clear · public.orders"
    # (SRC-10 decision 5). Other runs are about every table and send none.
    object_name = context.op_config.get("object")
    reporter.start(trigger, table=object_name if trigger in ("refresh", "clear") else None)
    single = _single_table_request(connector_id, context.op_config.get("object"), context.op_config.get("action"))
    cfg = IngestFactoryConfig.from_env()
    connector = _fetch_connector_for_run(cfg, connector_id, _retry_number(context))
    if connector.get("paused") is True:
        # SRC-11 D13, SRC-18: a paused connector loads nothing, whoever started
        # the run (a run queued before the pause, one started from the
        # orchestrator's screen). Only an explicit `true` pauses: the flag is
        # the API's own boolean, and an older API that does not send it has no
        # pause to honour.
        context.log.info(f"connector {connector_id!r} is paused; this run loads nothing")
        reporter.end("paused", "connector_paused")
        return
    if single is not None and (connector["adapter"] == "cdc" or connector.get("ingestMode") == "stream"):
        raise Failure(
            f"connector {connector_id!r} streams its rows; a run of one table with an action is for batch connectors",
            allow_retries=False,
        )
    if connector["adapter"] == "cdc":
        context.log.info(
            f"connector {connector_id!r} is adapter=cdc -- no job body to run: Debezium's own "
            "snapshot.mode=initial (ADR 0008) runs ingestion automatically once its compose "
            "service starts, outside this job entirely"
        )
        reporter.end("succeeded", "completed")
        return
    if connector.get("ingestMode") == "stream":
        # The only ingest_mode a batch-shaped per-object loop below
        # cannot run: a kafka connector's rows arrive from one bounded
        # consumer.poll() loop over the whole topic, not from a
        # build_source() call per sourceObjects entry.
        attempt_number = _retry_number(context) + 1
        table = _table_label(connector)
        started_at = run_reports.now()
        try:
            rows = _run_stream_connector(connector, run_id=context.run_id)
        except Exception as exc:  # noqa: BLE001 -- classified by `_decide`, then `_fail_or_retry` always raises
            # The Kafka path gets the same rule as a table (SRC-11 D2).
            decision = _decide(exc, attempt_number - 1)
            reporter.attempt(
                table=table,
                attempt=attempt_number,
                started_at=started_at,
                ended_at=run_reports.now(),
                error_class=decision.verdict.error_class,
                wait_seconds=decision.wait_seconds,
            )
            if decision.wait_seconds is None:
                reporter.end("failed", "table_failed")
            _fail_or_retry(exc, label=repr(connector_id), retry_number=attempt_number - 1, decision=decision)
        reporter.attempt(
            table=table,
            attempt=attempt_number,
            started_at=started_at,
            ended_at=run_reports.now(),
            rows_loaded=rows if isinstance(rows, int) else 0,
        )
        reporter.end("succeeded", "completed")
        return
    if single is None and _discover_new_tables(connector, context.run_id):
        # Added in this run, loaded in this run: the API wrote them to the
        # connector's tables, so one more read of the connector (the same call
        # that started this run) fans out over the new list.
        connector = _fetch_connector_for_run(cfg, connector_id, _retry_number(context))
    objects = connector.get("sourceObjects", [])
    action = None
    if single is not None:
        object_name, action = single
        objects = [obj for obj in objects if obj.get("name") == object_name]
        if not objects:
            raise Failure(
                f"connector {connector_id!r} does not select the object {object_name!r}",
                allow_retries=False,
            )
    seen: dict[str, str] = {}
    for obj in objects:
        target = obj["target"]
        key = _sanitize_target(target)
        if key in seen:
            raise Failure(
                f"connector {connector_id!r} has two sourceObjects whose targets "
                f"sanitize to the same Dagster step key {key!r}: "
                f"{seen[key]!r} and {target!r} -- rename one so each source object "
                "maps to a unique mapped step",
                allow_retries=False,
            )
        seen[key] = target
        yield DynamicOutput(
            value={"connector": connector, "obj": obj, "action": action},
            mapping_key=key,
        )


def _report_attempt(
    reporter: run_reports.RunReporter,
    obj: dict,
    attempt: int,
    started_at: str,
    *,
    rows: int,
    error_class: str | None = None,
    wait_seconds: int | None = None,
    columns_left_out: int = 0,
) -> None:
    """Report one attempt at one table and the log lines it earned (D5, D9).
    Never raises (`RunReporter` swallows its own failures)."""
    table = obj["name"]
    reporter.attempt(
        table=table,
        attempt=attempt,
        started_at=started_at,
        ended_at=run_reports.now(),
        rows_loaded=rows,
        error_class=error_class,
        wait_seconds=wait_seconds,
    )
    if error_class == ingest_errors.SCHEMA_DECISION_PENDING:
        reporter.log("schema_decision_pending", table=table)
    if columns_left_out > 0:
        reporter.log("columns_left_out", table=table, count=columns_left_out)


@op(
    # SRC-11 D1: no `retry_policy`; see `run_ingest` and `_fail_or_retry`.
    tags=source_metadata(
        "dispar_orchestrate/ingest_factory.py::ingest_source_object",
        reads=["Connector source object spec"],
        writes=[
            "Iceberg bronze.{target} per source object",
            "ClickHouse lake.bronze_meta.ingest_run",
        ],
    ),
)
def ingest_source_object(context, payload: dict[str, Any]) -> Any:
    """One mapped step per source object of a batch connector. Receives
    the connector dict with its `secretRef` strings only -- resolves
    secrets INSIDE this op (so Dagster's IO manager persisting the
    `DynamicOutput` payload never sees a resolved secret value, the
    invariant the PART B fan-out design closes). The per-object
    `record_ingest_run` row and the `AssetMaterialization` for a
    measured integer row count are both the same ones the pre-fan-out
    `run_ingest` emitted; the per-step failure unit is the only thing
    that changed.

A failure is classified by `ingest_errors.classify` and either retried or
    ends the step (`_fail_or_retry`, SRC-11 D1 and D2): up to three retries of
    THIS table, after 1, 5 and 15 minutes, for an unreachable source, a
    timeout, a busy or failing source or an unreachable product API; never for
    a refused credential (PostgreSQL's included, SRC-19), a refused address, an
    unsupported table or mode, a rejected setting or an unexplained failure.
    The other tables of the run are separate steps and are not repeated."""
    connector, obj = payload["connector"], payload["obj"]
    retry_number = _retry_number(context)
    reporter = run_reports.RunReporter.for_run(connector["id"], context.run_id)
    notes = AttemptNotes()
    token = _NOTES.set(notes)
    started_at = run_reports.now()
    try:
        # `action` only when set: an ordinary run calls `_run_one_object` exactly as
        # it did before SRC-10.
        action = payload.get("action")
        rows = _run_one_object(connector, obj, run_id=context.run_id, **({"action": action} if action else {}))
    except Exception as exc:  # noqa: BLE001 -- classified by `_decide`, then `_fail_or_retry` always raises
        decision = _decide(exc, retry_number)
        _report_attempt(
            reporter,
            obj,
            retry_number + 1,
            started_at,
            rows=0,
            error_class=decision.verdict.error_class,
            wait_seconds=decision.wait_seconds,
            columns_left_out=notes.columns_left_out if decision.wait_seconds is None else 0,
        )
        _fail_or_retry(exc, label=repr(obj.get("target")), retry_number=retry_number, decision=decision)
    finally:
        _NOTES.reset(token)
    # A table that waits for a schema decision, and one a source cannot load
    # (`unsupported`: a Google Sheet), return without raising and the step
    # succeeds (SRC-8 D7); the report says why no rows came.
    held = {"waiting": ingest_errors.SCHEMA_DECISION_PENDING, "unsupported": ingest_errors.UNSUPPORTED}.get(
        notes.status or ""
    )
    _report_attempt(
        reporter,
        obj,
        retry_number + 1,
        started_at,
        rows=rows if isinstance(rows, int) else 0,
        error_class=held,
        columns_left_out=notes.columns_left_out,
    )
    if isinstance(rows, int):
        context.log_event(
            AssetMaterialization(asset_key=f"bronze.{obj['target']}", metadata={"rows": rows})
        )
    return rows


@job(name="ingest_job")
def ingest_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `ingest_job` -- the SAME
    static job for every ingestible connector, launched with run config
    `{"ops": {"run_ingest": {"config": {"connector_id": "..."}}}}`
    (mirrors `agent_run_job`'s `_employee_run_config` shape exactly).
    PART B: `run_ingest` is now a fan-out (cdc/stream yield no mapped
    steps; batch yields one mapped `ingest_source_object[<target>]`
    per source object). The `.map(...)` form is the mandatory DSL
    shape here -- `ingest_source_object(run_ingest())` would emit an
    "uninvoked op" warning then a graph-build error, because the
    output of `run_ingest()` is a `DynamicOutput` (a generator of
    output specs, not a value the mapped op can consume directly)."""
    run_ingest().map(lambda payload: ingest_source_object(payload)).collect()


def _ingest_run_config(connector_id: str) -> dict[str, Any]:
    return {"ops": {"run_ingest": {"config": {"connector_id": connector_id}}}}


# How far back a gap in evaluations (the daemon down, the API unreachable)
# is caught up: a connector that came due within the last hour still runs,
# once; anything older is skipped rather than launched late in a burst.
_MAX_CATCH_UP = timedelta(hours=1)

_IN_PROGRESS = [
    DagsterRunStatus.QUEUED,
    DagsterRunStatus.NOT_STARTED,
    DagsterRunStatus.STARTING,
    DagsterRunStatus.STARTED,
]


def _minute(moment: datetime) -> datetime:
    """`moment` in UTC, cut to the start of its minute: cron fire times."""
    return moment.astimezone(timezone.utc).replace(second=0, microsecond=0)


def _fetch_due_connectors(cfg: IngestFactoryConfig, after: datetime, until: datetime) -> list[dict[str, Any]]:
    """`GET /api/connectors/ingestible?dueAfter=&dueUntil=`: the connectors
    whose schedule fires in `(after, until]`.

    # Errors

    Propagates any request failure, a non-2xx answer and a body that is
    not a list. The sensor tick then fails visibly in Dagster, and its
    cursor does not move, so the same window is asked again next tick.
    """
    resp = requests.get(
        f"{cfg.api_url}/api/connectors/ingestible",
        params={"dueAfter": after.isoformat(), "dueUntil": until.isoformat()},
        headers=_headers(cfg),
        timeout=10,
    )
    resp.raise_for_status()
    connectors = resp.json()
    if not isinstance(connectors, list):
        raise RuntimeError("GET /api/connectors/ingestible?dueAfter= did not return a list")
    return connectors


def _busy_connector_ids(instance: Any) -> set[str]:
    """Every connector with an `ingest_job` run still queued or going,
    read from each run's own `connector_id` config (a run launched by
    `POST .../ingest/run` carries only the `user` trigger tag, not the
    connector's id)."""
    busy: set[str] = set()
    for run in instance.get_runs(filters=RunsFilter(job_name=ingest_job.name, statuses=_IN_PROGRESS)):
        config = (run.run_config or {}).get("ops", {}).get("run_ingest", {}).get("config", {})
        connector_id = config.get("connector_id")
        if isinstance(connector_id, str):
            busy.add(connector_id)
    return busy


def due_run_requests(
    connectors: list[dict[str, Any]], until: datetime, busy: set[str]
) -> tuple[list[RunRequest], list[str]]:
    """One `RunRequest` per due connector, and the ids skipped because an
    ingest of theirs is still going. `cdc` never runs here: its Debezium
    service streams on its own (ADR 0008), so there is no batch to launch.

    The run key names the connector and the window's end, so a tick that
    is evaluated twice (a daemon restart mid-tick) never launches twice."""
    run_requests: list[RunRequest] = []
    skipped: list[str] = []
    for connector in connectors:
        connector_id = connector["id"]
        if connector.get("adapter") == "cdc":
            continue
        if connector_id in busy:
            skipped.append(connector_id)
            continue
        run_requests.append(
            RunRequest(
                run_key=f"{connector_id}@{until.isoformat()}",
                run_config=_ingest_run_config(connector_id),
                tags={"lakehouse/connector_id": connector_id, "lakehouse/trigger": "schedule"},
            )
        )
    return run_requests, skipped


def _window_start(cursor: str | None, until: datetime) -> tuple[datetime, bool]:
    """Where this evaluation's window starts: the previous window's end,
    kept within `_MAX_CATCH_UP` (the flag says it had to be). With no, or
    an unreadable, cursor -- the sensor's first evaluation -- only the
    current minute is checked."""
    first = until - timedelta(minutes=1)
    if not cursor:
        return first, False
    try:
        previous = datetime.fromisoformat(cursor)
    except ValueError:
        return first, False
    earliest = until - _MAX_CATCH_UP
    return (previous, False) if previous >= earliest else (earliest, True)


@sensor(
    name="ingest_schedule_sensor",
    job=ingest_job,
    minimum_interval_seconds=30,
    default_status=DefaultSensorStatus.RUNNING,
    description=(
        "Runs ingest_job for every connector whose schedule came due. Schedules are read from "
        "lakehouse-api on every evaluation, so one saved in the console applies within a minute."
    ),
)
def ingest_schedule_sensor(context: SensorEvaluationContext) -> SensorResult:
    """See this module's "Schedules" section."""
    cfg = IngestFactoryConfig.from_env()
    if not cfg.service_token:
        return SensorResult(skip_reason="INGEST_SERVICE_TOKEN is unset, so connector schedules cannot be read")
    until = _minute(datetime.now(timezone.utc))
    after, clipped = _window_start(context.cursor, until)
    if after >= until:
        return SensorResult(skip_reason=f"{until:%H:%M} UTC was already checked")
    if clipped:
        context.log.warning(
            f"Schedules were last checked up to {context.cursor}; only those due after {after.isoformat()} "
            "are caught up, older ones are skipped"
        )
    connectors = _fetch_due_connectors(cfg, after, until)
    run_requests, skipped = due_run_requests(connectors, until, _busy_connector_ids(context.instance))
    for connector_id in skipped:
        context.log.info(f"{connector_id} is due but its previous ingest is still going; skipped this time")
    if not run_requests:
        return SensorResult(
            skip_reason=f"No connector due between {after:%H:%M} and {until:%H:%M} UTC", cursor=until.isoformat()
        )
    return SensorResult(run_requests=run_requests, cursor=until.isoformat())
