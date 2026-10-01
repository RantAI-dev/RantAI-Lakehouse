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

# Secrets

Resolved via `secret_resolver.resolve_secret_ref`, using
`secret_map.secret_field_names` to know which named fields the resolved
values become -- NEVER an env var name derived from the connector id (see
`secret_resolver.py`'s own module doc for the bug this replaces).
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any
from urllib.parse import urlparse

import requests
from dagster import (
    DagsterRunStatus,
    DefaultSensorStatus,
    Field,
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

from dispar_orchestrate import connector_catalog, dlt_pipeline, secret_resolver, ssrf_guard
from dispar_orchestrate.adapters import files as files_adapter
from dispar_orchestrate.adapters import mongodb as mongodb_adapter
from dispar_orchestrate.adapters import oracle as oracle_adapter
from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters import sftp as sftp_adapter
from dispar_orchestrate.adapters import sheets as sheets_adapter
from dispar_orchestrate.adapters import sink as sink_adapter
from dispar_orchestrate.adapters import sql as sql_adapter
from dispar_orchestrate.adapters.kafka import consume_one_batch
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.bronze_catalog import record_ingest_offset, record_ingest_run
from dispar_orchestrate.column_gate import UnsupportedColumnType, reject_unsupported_column_types
from dispar_orchestrate.secret_map import secret_field_names
from dispar_orchestrate.secret_resolver import SecretRefRejected

_POSTGRES_DRIVERS = ("postgres", "postgresql")


class UnknownAdapter(Exception):
    """`connector["adapter"]` names no dispatch arm this factory knows how
    to run -- raised BY NAME rather than left to surface as a bare
    `KeyError` from a dict lookup, so a future adapter added to the
    `connector_type`/`connector` CHECK constraints (`0033_connector_ingest_spec.sql`,
    `0043_ingest_tier2_adapters.sql`) without a matching dispatch arm here
    fails loudly and legibly at run time, never a silent no-op."""


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


_ADAPTERS = {
    "sql": sql_adapter,
    "files": files_adapter,
    "rest": rest_adapter,
    "sheets": sheets_adapter,
    "mongodb": mongodb_adapter,
    "sftp": sftp_adapter,
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
    fields = secret_field_names(adapter_name, auth_type)
    if not fields:
        # ("kafka", "none"): a PLAINTEXT broker needs no secret at all --
        # never call resolve_secret_ref on a primary slot the connector
        # was never required to set (mirrors secret_resolver.resolve_secrets).
        return {}
    values = [secret_resolver.resolve_secret_ref(connector.get("secretRef"))]
    if len(fields) == 2:
        values.append(secret_resolver.resolve_secret_ref(connector.get("secretRefSecondary")))
    return dict(zip(fields, values))


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


def _run_one_object(connector: dict, obj: dict) -> None:
    """Ingest one source object (table/endpoint/sheet range) for one
    connector, recording the outcome via `record_ingest_run` in EVERY
    case -- a rejection (bad secret ref, SSRF-blocked host, an
    unsupported nested column type) and any other driver/HTTP/sink
    failure all get a row, never a silently-invisible run.

    # Errors

    Re-raises whatever it catches (`SecretRefRejected`,
    `ssrf_guard.SsrfBlocked`, `UnsupportedColumnType`, or any other
    exception a driver/HTTP call/sink raises) after recording it -- this
    function's own run (the Dagster op) must still fail visibly, not just
    the governance-surface row.
    """
    connector_id, adapter_name, dial = connector["id"], connector["adapter"], connector["dial"]
    job_name = "ingest_job"
    started_at = datetime.now(timezone.utc).isoformat()

    def _record(*, rows, status, error=""):
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

    try:
        secrets = _resolve_object_secrets(connector, adapter_name, dial)
    except SecretRefRejected as exc:
        _record(rows=None, status="rejected", error=str(exc))
        raise

    try:
        reject_unsupported_column_types(obj.get("columns", []))

        if adapter_name == "sql" and dial.get("driver") in _POSTGRES_DRIVERS:
            # See this module's docstring, "Postgres routing" -- pinned
            # via hostaddr inside run_bronze_ingest itself, never through
            # ssrf_guard.pinned_resolution (psycopg2/libpq does not
            # resolve through socket.getaddrinfo).
            outcome = dlt_pipeline.run_bronze_ingest(
                dlt_pipeline.BronzeIngestConfig.from_dial(dial, secrets, [obj])
            )
            _record(rows=outcome["rows"], status="succeeded")
            _register_in_catalog(connector_id, obj)
            return

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
            result = oracle_adapter.build_source(dial, secrets, [obj])
            outcome = sink_adapter.load_via_sink(
                result.source,
                obj["target"],
                sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env()),
            )
            _record(rows=outcome.rows, status="succeeded")
            _register_in_catalog(connector_id, obj)
            return

        adapter = _ADAPTERS.get(adapter_name)
        if adapter is None:
            raise UnknownAdapter(f"ingest_factory: no adapter dispatch for adapter={adapter_name!r}")

        if adapter_name == "sheets":
            result = adapter.build_source(dial, secrets)
            if not result.supported:
                _record(rows=None, status="unsupported", error=result.reason or "")
                return
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
        else:
            result = adapter.build_source(dial, secrets, [obj])
            source, resolved = result.source, result.resolved

        def _load():
            sink_config = sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env())
            return sink_adapter.load_via_sink(source, obj["target"], sink_config)

        host = _host_of(dial)
        if resolved is not None and host is not None:
            with ssrf_guard.pinned_resolution(host, resolved):
                outcome = _load()
        else:
            outcome = _load()
        _record(rows=outcome.rows, status="succeeded")
        _register_in_catalog(connector_id, obj)
    except (ssrf_guard.SsrfBlocked, UnsupportedColumnType) as exc:
        _record(rows=None, status="rejected", error=str(exc))
        raise
    except Exception as exc:  # noqa: BLE001 -- every OTHER failure still gets a row (never silently invisible)
        _record(rows=None, status="failed", error=_classify_exception(exc))
        raise


class UnsupportedKafkaAuth(Exception):
    """The dial names a Kafka auth type this consumer cannot honour. Raised
    before any connection is attempted."""


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
) -> None:
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
                return
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
        except ssrf_guard.SsrfBlocked as exc:
            _record(rows=None, status="rejected", error=str(exc))
            raise
        except Exception as exc:  # noqa: BLE001 -- every OTHER failure still gets a row (never silently invisible)
            _record(rows=None, status="failed", error=_classify_exception(exc))
            raise
    finally:
        if owns_consumer:
            consumer.close()


def _run_stream_connector(connector: dict) -> None:
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
    run_kafka_stream_batch(
        connector_id=connector["id"],
        spec=dial,
        secrets=secrets,
        source_objects=connector.get("sourceObjects", []),
    )


@op(config_schema={"connector_id": Field(str, description="The connector.id (adapter IS NOT NULL) to ingest.")})
def run_ingest(context) -> None:
    connector_id: str = context.op_config["connector_id"]
    cfg = IngestFactoryConfig.from_env()
    connector = _fetch_one_connector(cfg, connector_id)
    if connector["adapter"] == "cdc":
        context.log.info(
            f"connector {connector_id!r} is adapter=cdc -- no job body to run: Debezium's own "
            "snapshot.mode=initial (ADR 0008) runs ingestion automatically once its compose "
            "service starts, outside this job entirely"
        )
        return
    if connector.get("ingestMode") == "stream":
        # The only ingest_mode a batch-shaped per-object loop below
        # cannot run: a kafka connector's rows arrive from one bounded
        # consumer.poll() loop over the whole topic, not from a
        # build_source() call per sourceObjects entry.
        _run_stream_connector(connector)
        return
    for obj in connector.get("sourceObjects", []):
        _run_one_object(connector, obj)


@job(name="ingest_job")
def ingest_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `ingest_job` -- the SAME
    static job for every ingestible connector, launched with run config
    `{"ops": {"run_ingest": {"config": {"connector_id": "..."}}}}`
    (mirrors `agent_run_job`'s `_employee_run_config` shape exactly)."""
    run_ingest()


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
    `POST .../ingest/run` carries no tag, only that config)."""
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
