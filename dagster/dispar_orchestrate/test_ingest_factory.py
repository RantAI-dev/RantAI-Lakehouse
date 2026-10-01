"""Unit tests for `dagster/dispar_orchestrate/ingest_factory.py` -- the ONE
static `ingest_job`/`run_ingest` op pair (mirrors `agent_runs.py`'s
`agent_run_job`/`run_agent_employee` shape exactly: a static `@job`
wrapping a `config_schema`-driven `@op`, never a job built per connector)
plus `ingest_schedule_sensor`, which asks `GET /api/connectors/ingestible`
which connectors came due since its last evaluation and launches the SAME
static job for each, with a DIFFERENT `run_config`.

No real network: `requests.get` is always monkeypatched.
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
from datetime import datetime, timedelta, timezone

from dagster import DagsterInstance, build_sensor_context

from dispar_orchestrate.ingest_factory import (
    IngestFactoryConfig,
    UnknownAdapter,
    _fetch_due_connectors,
    _window_start,
    due_run_requests,
    ingest_schedule_sensor,
    run_kafka_stream_batch,
)


@pytest.fixture(autouse=True)
def _no_catalog_registration(monkeypatch):
    """A successful load registers its table in the catalog through
    ClickHouse (`connector_catalog.py`); no test here reaches a real one."""
    import dispar_orchestrate.ingest_factory as f

    monkeypatch.setattr(f.connector_catalog, "register_connector_table", lambda *a, **k: 0)


UNTIL = datetime(2026, 9, 30, 2, 0, tzinfo=timezone.utc)


def _due(connector_id: str, adapter: str = "sql") -> dict:
    return {"id": connector_id, "adapter": adapter, "scheduleCron": "0 2 * * *"}


class _Resp:
    def __init__(self, body) -> None:
        self.body = body

    def raise_for_status(self) -> None:
        return None

    def json(self):
        return self.body


def test_due_run_requests_launch_the_one_static_job_per_due_connector() -> None:
    requests_, skipped = due_run_requests([_due("conn-a"), _due("conn-b", "cdc"), _due("conn-c")], UNTIL, {"conn-c"})
    # cdc streams through its own Debezium service: never launched here.
    # conn-c's previous ingest is still going: skipped, not stacked.
    assert [r.run_config for r in requests_] == [{"ops": {"run_ingest": {"config": {"connector_id": "conn-a"}}}}]
    assert skipped == ["conn-c"]
    # The run key names the connector and the fire window, so evaluating
    # the same window twice never launches twice.
    assert requests_[0].run_key == "conn-a@2026-09-30T02:00:00+00:00"
    assert requests_[0].tags["lakehouse/trigger"] == "schedule"


def test_window_start_continues_from_the_cursor_within_the_catch_up_limit() -> None:
    # First evaluation: only the current minute.
    assert _window_start(None, UNTIL) == (UNTIL - timedelta(minutes=1), False)
    assert _window_start("not a time", UNTIL) == (UNTIL - timedelta(minutes=1), False)
    # A normal tick picks up exactly where the last one stopped.
    previous = UNTIL - timedelta(minutes=3)
    assert _window_start(previous.isoformat(), UNTIL) == (previous, False)
    # After a long gap, only the last hour is caught up.
    assert _window_start((UNTIL - timedelta(days=1)).isoformat(), UNTIL) == (UNTIL - timedelta(hours=1), True)


def test_fetch_due_connectors_asks_for_the_window_and_refuses_a_non_list(monkeypatch) -> None:
    calls = []

    def fake_get(url, params=None, headers=None, timeout=None):
        calls.append((url, params, headers))
        return _Resp([_due("conn-a")])

    monkeypatch.setattr(requests, "get", fake_get)
    cfg = IngestFactoryConfig(api_url="http://x", service_token="t")
    after = UNTIL - timedelta(minutes=1)
    assert _fetch_due_connectors(cfg, after, UNTIL) == [_due("conn-a")]
    assert calls == [
        (
            "http://x/api/connectors/ingestible",
            {"dueAfter": "2026-09-30T01:59:00+00:00", "dueUntil": "2026-09-30T02:00:00+00:00"},
            {"Authorization": "Bearer t"},
        )
    ]

    monkeypatch.setattr(requests, "get", lambda *a, **k: _Resp({"error": "nope"}))
    with pytest.raises(RuntimeError):
        _fetch_due_connectors(cfg, after, UNTIL)


def test_sensor_skips_with_a_reason_when_the_token_is_unset(monkeypatch) -> None:
    monkeypatch.delenv("INGEST_SERVICE_TOKEN", raising=False)
    with DagsterInstance.ephemeral() as instance:
        result = ingest_schedule_sensor(build_sensor_context(instance=instance))
    assert "INGEST_SERVICE_TOKEN" in (result.skip_reason.skip_message or "")


def test_sensor_launches_due_connectors_and_moves_its_cursor(monkeypatch) -> None:
    monkeypatch.setenv("INGEST_SERVICE_TOKEN", "t")
    asked = []

    def fake_get(url, params=None, headers=None, timeout=None):
        asked.append(params)
        return _Resp([_due("conn-a")])

    monkeypatch.setattr(requests, "get", fake_get)
    with DagsterInstance.ephemeral() as instance:
        result = ingest_schedule_sensor(build_sensor_context(instance=instance))
    assert [r.run_config["ops"]["run_ingest"]["config"]["connector_id"] for r in result.run_requests] == ["conn-a"]
    # The cursor is the window's end: the next tick starts there.
    assert result.cursor == asked[0]["dueUntil"]


def test_sensor_fails_the_tick_when_the_api_cannot_answer(monkeypatch) -> None:
    # A failed tick is visible in Dagster and keeps the cursor, so the
    # same window is asked again -- a schedule is never silently lost.
    monkeypatch.setenv("INGEST_SERVICE_TOKEN", "t")
    monkeypatch.setattr(requests, "get", lambda *a, **k: (_ for _ in ()).throw(requests.ConnectionError()))
    with DagsterInstance.ephemeral() as instance, pytest.raises(requests.ConnectionError):
        ingest_schedule_sensor(build_sensor_context(instance=instance))


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
        lambda ref: {"env:CONNECTOR_KAFKA_PASSWORD": "s3cret"}[ref],
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
        # The username is the dial's (`auth.username`); only the password is
        # a secret. This fixture used to carry a second, different username
        # as a secret ref -- two sources for one value, which disagreed.
        "secretRef": "env:CONNECTOR_KAFKA_PASSWORD",
    }
    f._run_stream_connector(connector)

    assert len(calls) == 1
    assert calls[0]["connector_id"] == "conn-kafka"
    assert calls[0]["spec"] == connector["dial"]
    assert calls[0]["secrets"] == {"password": "s3cret"}
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
    recorded = []
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
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_run",
        lambda **kw: recorded.append(kw),
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
    assert recorded[0]["status"] == "succeeded"
    assert recorded[0]["rows"] == 1  # the real SinkResult.rows count, never fabricated


def _stub_stream_batch(monkeypatch, *, rows, sink) -> list:
    """A micro-batch of `rows` whose sink write is `sink`; returns the list
    the catalog registrations are collected in."""
    import dispar_orchestrate.ingest_factory as f

    registered: list = []
    monkeypatch.setattr(f, "consume_one_batch", lambda *a, **k: BatchResult(rows=rows, offsets_to_commit={0: 5}))
    monkeypatch.setattr(f, "load_via_sink", sink)
    monkeypatch.setattr(f, "record_ingest_offset", lambda *a, **k: None)
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: None)
    monkeypatch.setattr(
        f.connector_catalog, "register_connector_table", lambda connector_id, obj: registered.append((connector_id, obj))
    )
    return registered


def test_stream_batch_registers_its_bronze_table_in_the_catalog(monkeypatch) -> None:
    """A topic's Bronze table gets the Catalog entry every batch adapter's
    table gets. The source object's own name is used when it has one, the
    topic otherwise."""
    registered = _stub_stream_batch(
        monkeypatch,
        rows=[{"id": 1}],
        sink=lambda *a, **k: SinkResult(rows=1, has_failed_jobs=False, load_info_str="ok"),
    )
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"target": "kafka_orders"}],
        consumer=_FakeStreamConsumer(),
    )
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"name": "orders.v1", "target": "kafka_orders"}],
        consumer=_FakeStreamConsumer(),
    )
    assert registered == [
        ("conn-x", {"name": "orders", "target": "kafka_orders"}),
        ("conn-x", {"name": "orders.v1", "target": "kafka_orders"}),
    ]


def test_stream_batch_registers_nothing_when_nothing_was_loaded(monkeypatch) -> None:
    # An empty poll wrote nothing; a failed sink write wrote nothing.
    registered = _stub_stream_batch(
        monkeypatch, rows=[], sink=lambda *a, **k: SinkResult(rows=0, has_failed_jobs=False, load_info_str="ok")
    )
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"target": "kafka_orders"}],
        consumer=_FakeStreamConsumer(),
    )
    registered = _stub_stream_batch(
        monkeypatch, rows=[{"id": 1}], sink=lambda *a, **k: (_ for _ in ()).throw(RuntimeError("sink unavailable"))
    )
    with pytest.raises(RuntimeError):
        run_kafka_stream_batch(
            connector_id="conn-x",
            spec={"topic": "orders"},
            secrets={},
            source_objects=[{"target": "kafka_orders"}],
            consumer=_FakeStreamConsumer(),
        )
    assert registered == []


def test_stream_batch_still_succeeds_when_the_catalog_cannot_be_reached(monkeypatch) -> None:
    # The rows are in Bronze and the offsets are committed either way: a
    # Catalog failure is reported, never raised into a failed run.
    import dispar_orchestrate.ingest_factory as f

    _stub_stream_batch(
        monkeypatch,
        rows=[{"id": 1}],
        sink=lambda *a, **k: SinkResult(rows=1, has_failed_jobs=False, load_info_str="ok"),
    )
    monkeypatch.setattr(
        f.connector_catalog,
        "register_connector_table",
        lambda *a, **k: (_ for _ in ()).throw(RuntimeError("clickhouse down")),
    )
    consumer = _FakeStreamConsumer()
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"target": "kafka_orders"}],
        consumer=consumer,
    )
    assert len(consumer.commits) == 1


def test_stream_dispatch_does_not_commit_the_offset_if_the_sink_write_fails(monkeypatch) -> None:
    committed = []
    recorded = []
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
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_run",
        lambda **kw: recorded.append(kw),
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
    assert recorded[0]["status"] == "failed"
    assert recorded[0]["rows"] is None
    assert "sink unavailable" not in recorded[0]["error"]  # never the raw message
    assert "RuntimeError" in recorded[0]["error"]  # the classified type name IS safe to record


def test_stream_dispatch_records_nothing_on_an_empty_batch(monkeypatch) -> None:
    """An empty poll is not an outcome -- nothing happened this run, so
    nothing is recorded, mirroring `_run_one_object`'s own
    per-object-only recording (never a fabricated zero-row success)."""
    recorded = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: BatchResult(rows=[], offsets_to_commit={}),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_run",
        lambda **kw: recorded.append(kw),
    )
    fake_consumer = _FakeStreamConsumer()
    run_kafka_stream_batch(
        connector_id="conn-x",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"target": "orders"}],
        consumer=fake_consumer,
    )
    assert recorded == []


def test_stream_dispatch_records_a_rejected_ingest_run_when_consume_one_batch_is_ssrf_blocked(monkeypatch) -> None:
    """The G6 gate's own reason for this fix: an SSRF refusal of an
    advertised broker (`consume_one_batch` raising `ssrf_guard.SsrfBlocked`
    from inside `check_all_advertised_brokers`/`checking_resolver`) used
    to leave `/api/governance/ingest-runs` with no row at all for the
    connector. Poison `load_via_sink` the same way the existing dispatch
    tests poison a branch that must never be reached."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: (_ for _ in ()).throw(f.ssrf_guard.SsrfBlocked("advertised broker 169.254.169.254:9092 refused")),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.load_via_sink",
        lambda *a, **k: pytest.fail("must not be called -- the poll itself was blocked"),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.record_ingest_run",
        lambda **kw: recorded.append(kw),
    )
    fake_consumer = _FakeStreamConsumer()
    with pytest.raises(f.ssrf_guard.SsrfBlocked):
        run_kafka_stream_batch(
            connector_id="conn-x",
            spec={"topic": "orders"},
            secrets={},
            source_objects=[{"target": "orders"}],
            consumer=fake_consumer,
        )
    assert len(recorded) == 1
    assert recorded[0]["status"] == "rejected"
    assert recorded[0]["rows"] is None
    assert "169.254.169.254" in recorded[0]["error"]  # SsrfBlocked's message IS safe to record verbatim


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


def test_a_sasl_plain_kafka_dial_connects_with_sasl_over_tls_and_its_resolved_password():
    from dispar_orchestrate.ingest_factory import kafka_security_kwargs

    kwargs = kafka_security_kwargs(
        {"auth": {"type": "sasl_plain", "username": "ingest"}}, {"password": "not-a-real-secret"}
    )
    assert kwargs["security_protocol"] == "SASL_SSL"
    assert kwargs["sasl_mechanism"] == "PLAIN"
    assert kwargs["sasl_plain_username"] == "ingest"
    assert kwargs["sasl_plain_password"] == "not-a-real-secret"
    assert kwargs["ssl_check_hostname"] is True


def test_a_kafka_dial_with_no_auth_connects_in_plaintext():
    from dispar_orchestrate.ingest_factory import kafka_security_kwargs

    assert kafka_security_kwargs({"auth": {"type": "none"}}, {}) == {"security_protocol": "PLAINTEXT"}


def test_an_unknown_kafka_auth_type_is_refused_by_name_without_echoing_secrets():
    from dispar_orchestrate.ingest_factory import UnsupportedKafkaAuth, kafka_security_kwargs

    with pytest.raises(UnsupportedKafkaAuth) as exc:
        kafka_security_kwargs({"auth": {"type": "sasl_scram_sha256"}}, {"password": "not-a-real-secret"})
    assert "not-a-real-secret" not in str(exc.value)


def test_run_one_object_registers_a_loaded_table_in_the_catalog(monkeypatch) -> None:
    import dispar_orchestrate.ingest_factory as f

    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: None)
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f.dlt_pipeline.BronzeIngestConfig, "from_dial", staticmethod(lambda *a: object()))
    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", lambda cfg: {"rows": 836})
    registered = []
    monkeypatch.setattr(
        f.connector_catalog, "register_connector_table", lambda cid, obj: registered.append((cid, obj)) or 1672
    )

    connector = {
        "id": "conn-northwind",
        "adapter": "sql",
        "dial": {"driver": "postgres", "host": "192.168.18.205", "port": 55432, "database": "northwind", "user": "u"},
        "secretRef": "file:/run/secrets/connector_managed_conn_northwind_password",
        "secretRefSecondary": None,
    }
    obj = {"name": "public.orders", "target": "northwind_orders"}
    f._run_one_object(connector, obj)

    assert registered == [("conn-northwind", obj)]


def test_a_catalog_registration_failure_never_fails_a_load_that_succeeded(monkeypatch, capsys) -> None:
    """The data is already in Bronze and recorded as succeeded; only its
    Catalog entry is missing, and that is reported, not raised."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f.dlt_pipeline.BronzeIngestConfig, "from_dial", staticmethod(lambda *a: object()))
    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", lambda cfg: {"rows": 836})

    def _broken(*a, **k):
        raise RuntimeError("clickhouse is down")

    monkeypatch.setattr(f.connector_catalog, "register_connector_table", _broken)

    connector = {
        "id": "conn-northwind",
        "adapter": "sql",
        "dial": {"driver": "postgres", "host": "192.168.18.205", "port": 55432, "database": "northwind", "user": "u"},
        "secretRef": "file:/run/secrets/connector_managed_conn_northwind_password",
        "secretRefSecondary": None,
    }
    f._run_one_object(connector, {"name": "public.orders", "target": "northwind_orders"})

    assert [r["status"] for r in recorded] == ["succeeded"]
    assert "could not be registered in the catalog: clickhouse is down" in capsys.readouterr().out
