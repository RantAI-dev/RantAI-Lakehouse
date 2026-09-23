"""Unit tests for `dagster/dispar_orchestrate/ingest_factory.py` -- the ONE
static `ingest_job`/`run_ingest` op pair (mirrors `agent_runs.py`'s
`agent_run_job`/`run_agent_employee` shape exactly: a static `@job`
wrapping a `config_schema`-driven `@op`, never a job built per connector)
plus `build_ingest_schedules`, which reads `GET /api/connectors/ingestible`
at Dagster code-load time and builds one `ScheduleDefinition` per
cron-scheduled, non-`cdc` connector -- all targeting the SAME static job
with a DIFFERENT `run_config`, mirroring `agent_runs.py::build_agent_run_schedules`.

No real network: `requests.get` is always monkeypatched (the real
`_fetch_ingestible_connectors` degrades to `[]` on any failure -- see
`test_agent_runs.py`'s equivalent tests for the same resilience shape).
No real SSRF/adapter/sink call: `_ADAPTERS`/`secret_resolver`/`sink_adapter`
are monkeypatched per test, matching `test_adapters_sql.py`'s style of
injecting fakes rather than touching a real driver.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_ingest_factory.py -q
"""

from __future__ import annotations

import pytest
import requests

from dispar_orchestrate.adapters.kafka import BatchResult
from dispar_orchestrate.adapters.sink import SinkResult
from dispar_orchestrate.ingest_factory import (
    IngestFactoryConfig,
    UnknownAdapter,
    build_ingest_schedules,
    run_kafka_stream_batch,
)


def test_build_ingest_schedules_returns_empty_when_api_is_unreachable(monkeypatch) -> None:
    monkeypatch.setattr(requests, "get", lambda *a, **k: (_ for _ in ()).throw(requests.ConnectionError()))
    cfg = IngestFactoryConfig(api_url="http://x", service_token="t")
    assert build_ingest_schedules(cfg) == []


def test_build_ingest_schedules_returns_empty_when_token_is_unset() -> None:
    cfg = IngestFactoryConfig(api_url="http://x", service_token="")
    assert build_ingest_schedules(cfg) == []


def test_build_ingest_schedules_skips_cdc_and_connectors_with_no_cron(monkeypatch) -> None:
    connectors = [
        {
            "id": "conn-a",
            "adapter": "sql",
            "scheduleCron": "0 * * * *",
            "dial": {},
            "sourceObjects": [],
            "secretRef": "env:CONNECTOR_MYSQL_PASSWORD",
            "secretRefSecondary": None,
        },
        {
            "id": "conn-b",
            "adapter": "cdc",
            "scheduleCron": "0 * * * *",
            "dial": {},
            "sourceObjects": [],
            "secretRef": "env:CONNECTOR_PG_PASSWORD",
            "secretRefSecondary": None,
        },
        {
            "id": "conn-c",
            "adapter": "sql",
            "scheduleCron": None,
            "dial": {},
            "sourceObjects": [],
            "secretRef": "env:CONNECTOR_PG_PASSWORD",
            "secretRefSecondary": None,
        },
    ]

    class _Resp:
        def raise_for_status(self) -> None:
            return None

        def json(self):
            return connectors

    monkeypatch.setattr(requests, "get", lambda *a, **k: _Resp())
    cfg = IngestFactoryConfig(api_url="http://x", service_token="t")
    schedules = build_ingest_schedules(cfg)
    assert [s.name for s in schedules] == ["ingest_schedule__conn_a"]
    # Every schedule targets the SAME static job, with a DIFFERENT
    # run_config -- never a per-connector job (mirrors `agent_runs.py`).
    assert schedules[0].job.name == "ingest_job"


def test_run_one_object_resolves_secrets_via_the_allowlisted_resolver(monkeypatch) -> None:
    # No env var name derived from the connector id -- only the
    # connector's DECLARED secretRef is ever read (the exact bug
    # `secret_resolver.py`'s module doc describes closing).
    import dispar_orchestrate.ingest_factory as f

    calls = []
    # This test isolates the secret-resolution call shape only -- the
    # "succeeded" outcome still reaches `_record`, which would otherwise
    # call the REAL `record_ingest_run` (a live ClickHouse write); that is
    # a separate concern `bronze_catalog.py`'s own tests own, so it is
    # monkeypatched here to a no-op, the same isolation
    # `test_run_one_object_records_rejected_on_a_bad_secret_ref`/
    # `..._records_failed_and_reraises_on_any_other_exception` (below) use
    # deliberately, to assert ON it instead.
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: None)
    monkeypatch.setattr(
        f,
        "_ADAPTERS",
        {
            "sql": type(
                "A",
                (),
                {
                    "build_source": staticmethod(
                        lambda dial, secrets, objs: calls.append(secrets)
                        or type("R", (), {"source": iter(()), "resolved": None})()
                    )
                },
            )()
        },
    )
    monkeypatch.setattr(
        f.secret_resolver, "resolve_secret_ref", lambda ref: {"env:CONNECTOR_MYSQL_PASSWORD": "s3cret"}[ref]
    )
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: f.sink_adapter.SinkResult(
        rows=1, has_failed_jobs=False, load_info_str=""
    ))
    connector = {
        "id": "conn-a",
        "adapter": "sql",
        "dial": {"host": "db.internal"},
        "secretRef": "env:CONNECTOR_MYSQL_PASSWORD",
        "secretRefSecondary": None,
    }
    f._run_one_object(connector, {"name": "orders", "target": "orders"})
    assert calls == [{"password": "s3cret"}]


def test_run_one_object_records_rejected_on_a_bad_secret_ref(monkeypatch) -> None:
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    connector = {
        "id": "conn-a",
        "adapter": "sql",
        "dial": {"host": "db.internal"},
        "secretRef": None,
        "secretRefSecondary": None,
    }
    with pytest.raises(f.secret_resolver.SecretRefRejected):
        f._run_one_object(connector, {"name": "orders", "target": "orders"})
    assert recorded[0]["status"] == "rejected"
    assert recorded[0]["rows"] is None


def test_run_one_object_records_failed_and_reraises_on_any_other_exception(monkeypatch) -> None:
    # A driver/HTTP/sink error is NOT silently invisible -- it gets an
    # ingest_run row (status="failed") AND still fails the op (re-raised),
    # classified by exception TYPE NAME only, never the raw exception
    # message (which can carry a host/credential fragment).
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(
        f,
        "_ADAPTERS",
        {
            "sql": type(
                "A",
                (),
                {
                    "build_source": staticmethod(
                        lambda dial, secrets, objs: (_ for _ in ()).throw(
                            ConnectionError("db.internal:5432 refused")
                        )
                    )
                },
            )()
        },
    )
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    connector = {
        "id": "conn-a",
        "adapter": "sql",
        "dial": {"host": "db.internal"},
        "secretRef": "env:CONNECTOR_MYSQL_PASSWORD",
        "secretRefSecondary": None,
    }
    with pytest.raises(ConnectionError):
        f._run_one_object(connector, {"name": "orders", "target": "orders"})
    assert recorded[0]["status"] == "failed"
    assert recorded[0]["rows"] is None
    assert "db.internal:5432 refused" not in recorded[0]["error"]  # never the raw message
    assert "ConnectionError" in recorded[0]["error"]  # the classified type name IS safe to record


def test_run_one_object_routes_a_postgres_driver_sql_connector_through_dlt_pipeline(monkeypatch) -> None:
    """`adapters/sql.py::build_source` refuses `driver in ("postgres",
    "postgresql")` outright (its own module docstring, "Open question 2,
    resolved") -- this factory must route that case through
    `dlt_pipeline.BronzeIngestConfig.from_dial` + `dlt_pipeline.run_bronze_ingest`
    instead, never through `_ADAPTERS["sql"]`. This is what makes the one
    real seeded connector (`rust/migrations/0034_seed_connector_ingest_spec.sql`'s
    `conn-pg-lakehouse`, `adapter='sql'`, `driver='postgres'`) actually
    runnable through this factory."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")

    from_dial_calls = []

    class _StubCfg:
        pass

    def fake_from_dial(dial, secrets, source_objects):
        from_dial_calls.append((dial, secrets, source_objects))
        return _StubCfg()

    def fake_run_bronze_ingest(cfg):
        assert isinstance(cfg, _StubCfg)
        return {"rows": 42, "bronze_table_name": "orders", "source_schema": "public", "source_table": "orders"}

    monkeypatch.setattr(f.dlt_pipeline.BronzeIngestConfig, "from_dial", staticmethod(fake_from_dial))
    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", fake_run_bronze_ingest)
    # `_ADAPTERS["sql"]` must never be reached for a postgres-driver dial --
    # poison it so this test fails loudly if the routing branch is missed.
    monkeypatch.setattr(
        f,
        "_ADAPTERS",
        {
            "sql": type(
                "A",
                (),
                {"build_source": staticmethod(lambda dial, secrets, objs: (_ for _ in ()).throw(AssertionError(
                    "adapters/sql.py must never be called for a postgres-driver connector"
                )))},
            )()
        },
    )

    connector = {
        "id": "conn-pg-lakehouse",
        "adapter": "sql",
        "dial": {
            "driver": "postgres",
            "host": "db.internal",
            "port": 5432,
            "database": "orders",
            "user": "reader",
        },
        "secretRef": "env:CONNECTOR_MYSQL_PASSWORD",
        "secretRefSecondary": None,
    }
    obj = {"name": "public.orders", "target": "orders"}
    f._run_one_object(connector, obj)

    assert from_dial_calls == [(connector["dial"], {"password": "s3cret"}, [obj])]
    assert recorded[0]["status"] == "succeeded"
    assert recorded[0]["rows"] == 42


def test_run_one_object_routes_an_oracle_driver_sql_connector_through_the_oracle_adapter(monkeypatch) -> None:
    """`adapters/sql.py::build_source` has no `oracle` branch at all
    (`_DRIVERNAMES` only names `mysql`/`mariadb`) -- Oracle is dispatched
    to `adapters/oracle.py::build_source` instead, and the load is NEVER
    wrapped in `ssrf_guard.pinned_resolution`: oracle's own `build_source`
    already dialed the resolved IP literal inside its own
    `checking_resolver()` scope before returning."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")

    oracle_calls = []
    pinned_calls = []

    def fake_oracle_build_source(dial, secrets, source_objects):
        oracle_calls.append((dial, secrets, source_objects))
        return type("R", (), {"source": iter(()), "resolved": f.ssrf_guard.ResolvedAddress("10.0.0.9", 1521, 2)})()

    monkeypatch.setattr(f.oracle_adapter, "build_source", fake_oracle_build_source)
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: f.sink_adapter.SinkResult(
        rows=7, has_failed_jobs=False, load_info_str=""
    ))

    def poisoned_pinned_resolution(host, resolved):
        pinned_calls.append((host, resolved))
        raise AssertionError("oracle's own build_source already guarded the dial -- must not be wrapped again")

    monkeypatch.setattr(f.ssrf_guard, "pinned_resolution", poisoned_pinned_resolution)
    # `_ADAPTERS["sql"]` (adapters/sql.py) must never be reached for
    # driver="oracle" -- poison it so this test fails loudly if the
    # routing branch is missed.
    monkeypatch.setattr(
        f,
        "_ADAPTERS",
        {
            "sql": type(
                "A",
                (),
                {"build_source": staticmethod(lambda dial, secrets, objs: (_ for _ in ()).throw(AssertionError(
                    "adapters/sql.py must never be called for an oracle-driver connector"
                )))},
            )()
        },
    )

    connector = {
        "id": "conn-oracle",
        "adapter": "sql",
        "dial": {"driver": "oracle", "host": "ora.internal", "port": 1521, "database": "ORCLPDB1", "user": "reader"},
        "secretRef": "env:CONNECTOR_ORACLE_PASSWORD",
        "secretRefSecondary": None,
    }
    obj = {"name": "SCHEMA.ORDERS", "target": "orders"}
    f._run_one_object(connector, obj)

    assert oracle_calls == [(connector["dial"], {"password": "s3cret"}, [obj])]
    assert pinned_calls == []  # never wrapped -- oracle guards its own dial
    assert recorded[0]["status"] == "succeeded"
    assert recorded[0]["rows"] == 7


def test_run_one_object_routes_a_mongodb_connector_through_the_mongodb_adapter_without_wrapping_in_pinned_resolution(
    monkeypatch,
) -> None:
    """`mongodb` was entirely absent from `_ADAPTERS` -- a mongodb
    connector had no dispatch arm at all. Once added, its load must never
    go through `ssrf_guard.pinned_resolution`: `build_source.resolved` is
    a LIST of every seed host's `ResolvedAddress`, not the single address
    `pinned_resolution` takes, and the adapter's own `_collection_rows`
    already guards the whole read with `checking_resolver()`."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")

    mongo_calls = []
    pinned_calls = []

    def fake_mongo_build_source(dial, secrets, source_objects):
        mongo_calls.append((dial, secrets, source_objects))
        return type(
            "R",
            (),
            {
                "sources": {source_objects[0]["target"]: iter([{"_id": 1}])},
                "resolved": [f.ssrf_guard.ResolvedAddress("10.0.0.5", 27017, 2)],
            },
        )()

    monkeypatch.setattr(f.mongodb_adapter, "build_source", fake_mongo_build_source)
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: f.sink_adapter.SinkResult(
        rows=1, has_failed_jobs=False, load_info_str=""
    ))

    def poisoned_pinned_resolution(host, resolved):
        pinned_calls.append((host, resolved))
        raise AssertionError("mongodb's own checking_resolver already guards the read -- must not be wrapped again")

    monkeypatch.setattr(f.ssrf_guard, "pinned_resolution", poisoned_pinned_resolution)

    connector = {
        "id": "conn-mongo",
        "adapter": "mongodb",
        "dial": {"hosts": ["mongo.internal:27017"], "database": "app", "username": "reader"},
        "secretRef": "env:CONNECTOR_MONGO_PASSWORD",
        "secretRefSecondary": None,
    }
    obj = {"name": "orders", "target": "orders"}
    f._run_one_object(connector, obj)

    assert mongo_calls == [(connector["dial"], {"password": "s3cret"}, [obj])]
    assert pinned_calls == []
    assert recorded[0]["status"] == "succeeded"
    assert recorded[0]["rows"] == 1


def test_run_one_object_routes_an_sftp_connector_through_the_sftp_adapter_without_wrapping_in_pinned_resolution(
    monkeypatch,
) -> None:
    """`sftp` was entirely absent from `_ADAPTERS`. Once added, its load
    must never go through `ssrf_guard.pinned_resolution` a second time:
    `adapters/sftp.py::build_source` already wraps its own
    `client.connect()` in that same context manager, and reads the file
    fully into memory, before this function ever sees a source."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")

    sftp_calls = []
    pinned_calls = []

    def fake_sftp_build_source(dial, secrets, source_objects):
        sftp_calls.append((dial, secrets, source_objects))
        return type(
            "R",
            (),
            {
                "sources": {source_objects[0]["target"]: iter([{"a": "1"}])},
                "resolved": f.ssrf_guard.ResolvedAddress("10.0.0.7", 22, 2),
            },
        )()

    monkeypatch.setattr(f.sftp_adapter, "build_source", fake_sftp_build_source)
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: f.sink_adapter.SinkResult(
        rows=1, has_failed_jobs=False, load_info_str=""
    ))

    def poisoned_pinned_resolution(host, resolved):
        pinned_calls.append((host, resolved))
        raise AssertionError("sftp's own build_source already pinned the connect -- must not be wrapped again")

    monkeypatch.setattr(f.ssrf_guard, "pinned_resolution", poisoned_pinned_resolution)

    connector = {
        "id": "conn-sftp",
        "adapter": "sftp",
        "dial": {
            "host": "sftp.internal",
            "port": 22,
            "user": "lakehouse",
            "hostKeyFingerprint": "SHA256:deadbeef",
            "path": "/outbox",
            "fileFormat": "csv",
            "auth": {"type": "password"},
        },
        "secretRef": "env:CONNECTOR_SFTP_PASSWORD",
        "secretRefSecondary": None,
    }
    obj = {"name": "orders.csv", "target": "orders"}
    f._run_one_object(connector, obj)

    assert sftp_calls == [(connector["dial"], {"password": "s3cret"}, [obj])]
    assert pinned_calls == []
    assert recorded[0]["status"] == "succeeded"
    assert recorded[0]["rows"] == 1


def test_run_one_object_raises_unknown_adapter_by_name_for_an_unrecognized_adapter(monkeypatch) -> None:
    # A future adapter value that reaches this factory with no dispatch
    # arm must fail loudly and NAME the adapter -- never a silent skip
    # (AGENTS.md principle 3, fail closed) and never a bare KeyError.
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f, "secret_field_names", lambda adapter, auth_type: ("password",))
    connector = {
        "id": "conn-future",
        "adapter": "smtp",
        "dial": {},
        "secretRef": "env:CONNECTOR_SMTP_PASSWORD",
        "secretRefSecondary": None,
    }
    with pytest.raises(UnknownAdapter, match="smtp"):
        f._run_one_object(connector, {"name": "x", "target": "x"})
    assert recorded[0]["status"] == "failed"


def test_run_ingest_dispatches_a_stream_mode_kafka_connector_to_run_kafka_stream_batch(monkeypatch) -> None:
    """`run_ingest` had no arm for `ingest_mode == "stream"` at all -- a
    kafka connector's op body fell straight into the per-object
    `_run_one_object` loop, which has no `kafka` dispatch of its own
    either. `_run_stream_connector` is the arm `run_ingest` now calls
    BEFORE that loop for any `ingestMode == "stream"` connector."""
    import dispar_orchestrate.ingest_factory as f

    calls = []
    monkeypatch.setattr(f, "run_kafka_stream_batch", lambda **kw: calls.append(kw))
    monkeypatch.setattr(
        f.secret_resolver,
        "resolve_secret_ref",
        lambda ref: {"env:CONNECTOR_KAFKA_USERNAME": "svc-reader", "env:CONNECTOR_KAFKA_PASSWORD": "s3cret"}[ref],
    )

    connector = {
        "id": "conn-kafka",
        "adapter": "kafka",
        "ingestMode": "stream",
        "dial": {
            "bootstrapServers": ["broker.internal:9092"],
            "topic": "orders",
            "auth": {"type": "sasl_plain", "username": "orders-reader"},
            "groupId": "lakehouse-orders-consumer",
            "microBatchSeconds": 30,
        },
        "sourceObjects": [{"name": "orders", "target": "orders"}],
        "secretRef": "env:CONNECTOR_KAFKA_USERNAME",
        "secretRefSecondary": "env:CONNECTOR_KAFKA_PASSWORD",
    }
    f._run_stream_connector(connector)

    assert len(calls) == 1
    assert calls[0]["connector_id"] == "conn-kafka"
    assert calls[0]["spec"] == connector["dial"]
    assert calls[0]["secrets"] == {"username": "svc-reader", "password": "s3cret"}
    assert calls[0]["source_objects"] == connector["sourceObjects"]


def test_run_ingest_never_calls_run_one_object_for_a_stream_mode_connector(monkeypatch) -> None:
    # Proves the dispatch order inside run_ingest itself: a stream-mode
    # connector must never fall into the per-object batch loop, even if
    # it happens to carry a non-empty sourceObjects list.
    import dispar_orchestrate.ingest_factory as f
    from dagster import build_op_context

    monkeypatch.setattr(f, "_run_stream_connector", lambda connector: None)
    monkeypatch.setattr(
        f, "_run_one_object", lambda *a, **k: (_ for _ in ()).throw(
            AssertionError("a stream-mode connector must never reach the batch per-object loop")
        )
    )
    connector = {
        "id": "conn-kafka",
        "adapter": "kafka",
        "ingestMode": "stream",
        "dial": {},
        "sourceObjects": [{"name": "orders", "target": "orders"}],
        "secretRef": None,
        "secretRefSecondary": None,
    }
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, connector_id: connector)
    monkeypatch.setattr(f.IngestFactoryConfig, "from_env", staticmethod(lambda: f.IngestFactoryConfig(api_url="http://x", service_token="t")))
    context = build_op_context(op_config={"connector_id": "conn-kafka"})
    f.run_ingest(context)


# ── run_kafka_stream_batch ────────────────────────────────────────────────
#
# `consumer` is passed explicitly as a fake -- these are unit tests, so no
# real `KafkaConsumer` is ever constructed and no network is touched.
# `consume_one_batch`/`load_via_sink`/`record_ingest_offset` are
# monkeypatched by their bare (imported) names in `ingest_factory`'s own
# module namespace, the same style `test_run_one_object_*` above already
# uses for `_ADAPTERS`/`secret_resolver`/`sink_adapter`.


class _FakeStreamConsumer:
    """The minimal surface `run_kafka_stream_batch` calls on a
    caller-supplied consumer: `.commit(...)`. `close()` is intentionally
    NOT exercised here -- a caller-supplied consumer is the CALLER's to
    close (see `run_kafka_stream_batch`'s own docstring), so this fake
    records a commit call is enough to prove the code path."""

    def __init__(self):
        self.commits = []

    def commit(self, offsets):
        self.commits.append(offsets)


def test_stream_dispatch_commits_offset_only_after_a_successful_sink_write(monkeypatch) -> None:
    committed = []
    written = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: BatchResult(rows=[{"id": 1}], offsets_to_commit={0: 5}),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.load_via_sink",
        lambda *a, **k: written.append(True) or SinkResult(rows=1, has_failed_jobs=False, load_info_str="ok"),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_offset",
        lambda *a, **k: committed.append(a),
    )
    fake_consumer = _FakeStreamConsumer()
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"target": "orders"}],
        consumer=fake_consumer,
    )
    assert written == [True]
    assert len(committed) == 1  # committed AFTER the write, never before
    assert len(fake_consumer.commits) == 1  # the broker-side consumer-group commit also happened


def test_stream_dispatch_does_not_commit_the_offset_if_the_sink_write_fails(monkeypatch) -> None:
    committed = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: BatchResult(rows=[{"id": 1}], offsets_to_commit={0: 5}),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.load_via_sink",
        lambda *a, **k: (_ for _ in ()).throw(RuntimeError("sink unavailable")),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_offset",
        lambda *a, **k: committed.append(a),
    )
    fake_consumer = _FakeStreamConsumer()
    with pytest.raises(RuntimeError):
        run_kafka_stream_batch(
            connector_id="conn-x",
            spec={"topic": "orders"},
            secrets={},
            source_objects=[{"target": "orders"}],
            consumer=fake_consumer,
        )
    assert committed == []  # never committed -- the batch is retried whole next run
    assert fake_consumer.commits == []  # the broker-side commit never happened either


def test_stream_dispatch_returns_without_writing_or_committing_on_an_empty_batch(monkeypatch) -> None:
    written = []
    committed = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: BatchResult(rows=[], offsets_to_commit={}),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.load_via_sink",
        lambda *a, **k: written.append(True),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_offset",
        lambda *a, **k: committed.append(a),
    )
    fake_consumer = _FakeStreamConsumer()
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"target": "orders"}],
        consumer=fake_consumer,
    )
    assert written == []
    assert committed == []
    assert fake_consumer.commits == []
