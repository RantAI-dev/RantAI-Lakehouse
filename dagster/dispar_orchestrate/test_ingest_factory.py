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


class _FakeColumn:
    def __init__(self, name: str, type_name: str, nullable: bool = True) -> None:
        self.name, self.type, self.nullable = name, type_name, nullable

    def __str__(self) -> str:  # `str(column.type)` is the database's spelling
        return self.type


class _FakeTable:
    """What `sql_database` hands a `table_adapter_callback`: a reflected
    SQLAlchemy `Table`, reduced to what `schema_observer` reads."""

    def __init__(self, columns=(("id", "INTEGER", False), ("note", "VARCHAR(40)", True)), key=("id",)) -> None:
        self._columns = [_FakeColumn(n, _FakeType(t), nullable) for n, t, nullable in columns]
        self.columns = self._columns
        by_name = {c.name: c for c in self._columns}
        self.primary_key = type("PK", (), {"columns": [by_name[k] for k in key]})()


class _FakeType:
    def __init__(self, text: str) -> None:
        self.text = text

    def __str__(self) -> str:
        return self.text


def _reflect(callback, table=None) -> None:
    """What a real `sql_database` build does: reflect, and hand the table to
    the callback (SRC-8). A fake source builder calls this."""
    if callback is not None:
        callback(table or _FakeTable())


@pytest.fixture(autouse=True)
def observations(monkeypatch):
    """Every test here runs with an API that answers `load` for every
    schema observation (SRC-8) and records what was asked. A test of the
    observation itself replaces the answer through `observations.answer`: a
    decision, an exception to raise, or a function of the object's name."""
    import dispar_orchestrate.ingest_factory as f
    from dispar_orchestrate import schema_observer

    class Recorder:
        def __init__(self) -> None:
            self.calls: list[dict] = []
            self.answer = schema_observer.Decision(action="load", columns=None, changes=[])

    recorder = Recorder()

    def fake_post(cfg, connector_id, object_name, columns, primary_key, phase, run_id):
        recorder.calls.append(
            {
                "connector_id": connector_id,
                "object": object_name,
                "columns": list(columns),
                "primary_key": list(primary_key),
                "phase": phase,
                "run_id": run_id,
            }
        )
        answer = recorder.answer(object_name) if callable(recorder.answer) else recorder.answer
        if isinstance(answer, Exception):
            raise answer
        return answer

    monkeypatch.setattr(f.schema_observer, "post_observation", fake_post)
    return recorder


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
                        lambda dial, secrets, objs, table_adapter_callback=None: calls.append(secrets)
                        or _reflect(table_adapter_callback)
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
                        lambda dial, secrets, objs, table_adapter_callback=None: (_ for _ in ()).throw(
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

    plans = []

    def fake_run_bronze_ingest(cfg, plan, gate=None):
        assert isinstance(cfg, _StubCfg)
        plans.append(plan)
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
    # A source object saved before load modes existed replaces: its next
    # run leaves one copy of the table, not one more.
    assert plans == [f.LoadPlan(mode="replace")]

    f._run_one_object(connector, {**obj, "loadMode": "incremental", "incrementalKey": "order_id"})
    assert plans[1] == f.LoadPlan(mode="incremental", cursor="order_id")


def test_load_plan_offers_incremental_to_sql_connectors_only() -> None:
    import dispar_orchestrate.ingest_factory as f

    obj = {"name": "orders", "target": "orders", "loadMode": "incremental", "incrementalKey": "updated_at"}
    assert f._load_plan("sql", obj) == f.LoadPlan(mode="incremental", cursor="updated_at")
    assert f._load_plan("rest", {"name": "orders", "target": "orders", "loadMode": "append"}) == f.LoadPlan(mode="append")
    with pytest.raises(f.UnsupportedLoadMode):
        f._load_plan("rest", obj)


def test_run_one_object_records_an_unrunnable_load_mode_as_rejected(monkeypatch) -> None:
    """A mode this build cannot run as written is a rejection the run
    history shows with its reason, and nothing is dialed or loaded."""
    import dispar_orchestrate.ingest_factory as f

    recorded = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(
        f.dlt_pipeline,
        "run_bronze_ingest",
        lambda cfg, plan: (_ for _ in ()).throw(AssertionError("must not load")),
    )
    connector = {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {"driver": "postgres", "host": "db.internal", "port": 5432, "database": "d", "user": "u"},
        "secretRef": "env:CONNECTOR_PG_PASSWORD",
        "secretRefSecondary": None,
    }
    with pytest.raises(f.UnsupportedLoadMode):
        # Incremental with no cursor column.
        f._run_one_object(connector, {"name": "public.orders", "target": "orders", "loadMode": "incremental"})
    assert recorded[0]["status"] == "rejected"
    assert "incrementalKey" in recorded[0]["error"]


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

    def fake_oracle_build_source(dial, secrets, source_objects, table_adapter_callback=None):
        oracle_calls.append((dial, secrets, source_objects))
        _reflect(table_adapter_callback)
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

    monkeypatch.setattr(f, "_run_stream_connector", lambda connector, run_id=None: None)
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
    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", lambda cfg, plan, gate=None: {"rows": 836})
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
    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", lambda cfg, plan, gate=None: {"rows": 836})

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


def test_run_ingest_reports_each_objects_measured_rows_as_a_materialization(monkeypatch) -> None:
    # PART B of `parts/1b-dagster-one-op-per-unit-and-retries.md`: the
    # materialization is now logged by the mapped `ingest_source_object`
    # op, not by `run_ingest` (which is the fan-out). Driving the full
    # `ingest_job` via `execute_in_process` -- the same path the other
    # PART B fan-out tests use -- proves the per-object materialization
    # is still emitted (only for `orders`, whose adapter returned an
    # int; `sheet` returned `None` and so is not materialized). The
    # materialization is read off the per-step
    # `event_specific_data.materialization.metadata` -- the same path
    # `test_gold_export.py::GoldExportFanOutTest` walks.
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    rows_by_object = {"orders": 42, "sheet": None}
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: rows_by_object[obj["name"]])
    connector = {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {},
        "sourceObjects": [{"name": "orders", "target": "orders"}, {"name": "sheet", "target": "sheet"}],
    }
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )
    materials = [
        (
            e.event_specific_data.materialization.asset_key.to_user_string(),
            e.event_specific_data.materialization.metadata["rows"].value,
        )
        for e in result.all_events
        if e.event_type_value == "ASSET_MATERIALIZATION"
    ]
    assert materials == [("bronze/orders", 42)]


def _stub_env(monkeypatch) -> None:
    """Stub `IngestFactoryConfig.from_env` so a fan-out test does not
    reach the real `LAKEHOUSE_API_URL` env. Used by every PART B fan-out
    test below."""
    import dispar_orchestrate.ingest_factory as f

    monkeypatch.setattr(
        f.IngestFactoryConfig,
        "from_env",
        staticmethod(lambda: f.IngestFactoryConfig(api_url="http://x", service_token="t")),
    )


def _batch_connector(targets):
    return {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {},
        "sourceObjects": [{"name": t, "target": t} for t in targets],
    }


# PART B fan-out tests. `run_ingest` is now the fan-out:
# cdc -> log + no DynamicOutputs; stream -> call _run_stream_connector +
# no DynamicOutputs; batch -> one DynamicOutput per `sourceObjects` entry,
# mapped to a per-object `ingest_source_object` step. A failed object's
# step fails without taking the others down; secrets are resolved INSIDE
# the mapped op (the DynamicOutput payload carries the connector with its
# `secretRef` strings only).
def test_three_objects_yield_three_mapped_steps_with_their_keys(monkeypatch) -> None:
    """Three source objects -> three mapped steps
    `ingest_source_object[<sanitized_target>]`. Each mapped step
    calls `_run_one_object` once, in its own failure unit."""
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    monkeypatch.setattr(
        f, "_run_one_object",
        lambda connector, obj, run_id=None: {"orders": 7, "customers": 3, "invoices": 1}[obj["name"]],
    )
    connector = _batch_connector(["orders", "customers", "invoices"])
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)

    # `run_ingest` is now a `DynamicOut` op; invoke it via
    # `execute_in_process` on the real `ingest_job` so the
    # fan-out->map->collect graph runs as a job.
    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )
    assert result.success
    succeeded = {e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"}
    # The mapped op's mapping_key is the sanitized target -- three
    # distinct targets -> three distinct step keys.
    assert "ingest_source_object[orders]" in succeeded
    assert "ingest_source_object[customers]" in succeeded
    assert "ingest_source_object[invoices]" in succeeded


def test_one_failing_object_fails_only_its_step_and_records_its_failure_row(monkeypatch) -> None:
    """One object's adapter raises -> only that mapped step fails;
    the others still record `ingest_run` success rows. Without the
    fan-out, a single failure aborted the whole per-object loop
    inside `run_ingest`, and the failing object's row was its only
    observability.

    The test drives the real `_run_one_object` (so its
    `record_ingest_run` call -- in EVERY branch, including the
    `except Exception` catch -- runs for real) by patching the
    adapter's `build_source` to raise only for the `customers`
    object, mirroring how the existing `_run_one_object` tests
    inject fakes through the adapter seam (`test_adapters_sql.py`'s
    pattern)."""
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)

    class _FakeOutcome:
        def __init__(self, rows: int) -> None:
            self.rows = rows
            self.columns = ()  # a `SinkResult` always has them (SRC-8)

    class _FakeResult:
        def __init__(self, rows: int) -> None:
            self.source = object()  # identity-only; sink is monkeypatched
            self.resolved = None
            self.rows = rows

    class _FakeAdapter:
        def build_source(self, dial, secrets, objs, table_adapter_callback=None):
            _reflect(table_adapter_callback)
            if objs[0]["name"] == "customers":
                raise f.UnknownAdapter("adapter=something-not-real")
            return _FakeResult(rows={"orders": 11, "invoices": 5}[objs[0]["name"]])

    # Patch the generic-sql adapter entry on the module's adapter
    # table; mirror what `test_run_one_object_routes_*_through_*`
    # already does.
    monkeypatch.setitem(f._ADAPTERS, "sql", _FakeAdapter())
    monkeypatch.setattr(f, "_host_of", lambda dial: None)

    def fake_load(source, target, sink_config, plan):
        return _FakeOutcome(rows=42)

    # `load_via_sink` is referenced inside `_run_one_object` as
    # `sink_adapter.load_via_sink` (where `sink_adapter` is the
    # `adapters.sink` module), NOT `ingest_factory.load_via_sink`. So
    # patching `f.load_via_sink` (the local re-export) does nothing --
    # the real function would still run, drive dlt's pipeline, and
    # try to reach `lakehouse-api`. Patch the canonical attribute on
    # the sink module instead. The previous test that mocks
    # `f.load_via_sink` works because it drives `_run_one_object`
    # directly, where the same reference is used -- here we drive
    # through the job graph, so the canonical patch is the one that
    # matters.
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", fake_load)

    # Resolve the connector's declared `secretRef` so the pre-adapter
    # `_resolve_object_secrets` call inside `_run_one_object` does not
    # itself raise `SecretRefRejected` (which would otherwise be the
    # failure the test would observe, not the adapter-side
    # `UnknownAdapter` it is meant to assert). The other PART B tests
    # use a `secretRef: None` connector and skip this, because they do
    # not exercise `_run_one_object` end-to-end.
    monkeypatch.setattr(
        f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret"
    )

    # Capture every `record_ingest_run` call so the test can assert
    # the failing object's row exists alongside the two success rows.
    record_calls = []
    monkeypatch.setattr(
        f, "record_ingest_run",
        lambda **kwargs: record_calls.append(kwargs),
    )
    connector = _batch_connector(["orders", "customers", "invoices"])
    connector["secretRef"] = "env:CONNECTOR_PG_PASSWORD"
    connector["secretRefSecondary"] = None
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )
    assert not result.success
    failed = {e.step_key for e in result.all_events if e.event_type_value == "STEP_FAILURE"}
    succeeded = {e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"}
    assert "ingest_source_object[customers]" in failed
    assert "ingest_source_object[orders]" in succeeded
    assert "ingest_source_object[invoices]" in succeeded
    # At least one of the recorded `ingest_run` rows must be a
    # failure row for `customers` -- the failing object's row must
    # exist alongside the two success rows.
    statuses_by_name = {
        call["object_name"]: call["status"] for call in record_calls if "object_name" in call
    }
    assert statuses_by_name.get("customers") == "failed"
    assert statuses_by_name.get("orders") == "succeeded"
    assert statuses_by_name.get("invoices") == "succeeded"


def test_oracle_tls_config_error_from_a_mapped_step_is_wrapped_in_a_non_retryable_failure(
    monkeypatch,
) -> None:
    """PART D witness: `OracleTlsConfigError` (raised inside
    `adapters.oracle.build_source` for a bad TLS config) is one of the
    config-shaped types added to `ingest_source_object`'s except tuple
    in this round. Drive the op directly with a real
    `build_op_context`, mock `_run_one_object` to raise that type, and
    assert the surfaced `Failure` carries `allow_retries=False` with
    the original `OracleTlsConfigError` as `__cause__`. The mapped
    step's `record_ingest_run` failure row is written first (in
    `_run_one_object`'s own `except Exception` branch), so the
    governance surface sees the failure immediately -- exactly like
    the existing `UnknownAdapter` fan-out test."""
    from dagster import Failure, build_op_context

    import dispar_orchestrate.ingest_factory as f

    def fake_run_one_object(_connector, obj, run_id=None):
        raise f.oracle_adapter.OracleTlsConfigError(
            f"oracle tls config rejected for {obj.get('target')!r}"
        )

    monkeypatch.setattr(f, "_run_one_object", fake_run_one_object)
    with pytest.raises(Failure) as ctx:
        f.ingest_source_object(
            build_op_context(),
            {"connector": {"id": "c"}, "obj": {"name": "orders", "target": "orders"}},
        )
    assert ctx.value.allow_retries is False
    assert isinstance(ctx.value.__cause__, f.oracle_adapter.OracleTlsConfigError)


def test_cdc_connector_yields_no_mapped_steps(monkeypatch) -> None:
    """A `cdc`-adapter connector has NO per-object batch body to
    run (Debezium's own compose service owns its ingestion, see
    `run_ingest`'s existing cdc branch). `run_ingest` therefore
    yields no DynamicOutputs and the job runs only `run_ingest`
    itself, succeeding -- a downstream step being absent must not
    be treated as a failure."""
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: {
        "id": "conn-cdc",
        "adapter": "cdc",
        "dial": {},
        "sourceObjects": [],
    })
    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-cdc"}}}},
    )
    assert result.success
    succeeded = {e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"}
    # Only `run_ingest` itself runs -- no mapped `ingest_source_object`.
    assert "run_ingest" in succeeded
    for key in succeeded:
        assert not key.startswith("ingest_source_object["), key


def test_stream_connector_yields_no_mapped_steps(monkeypatch) -> None:
    """A `stream` ingestMode routes to `_run_stream_connector`
    (Kafka), not the per-object loop. `run_ingest` yields no
    DynamicOutputs and the stream connector runs as a single
    non-mapped body inside `run_ingest` itself."""
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: {
        "id": "conn-kafka",
        "adapter": "kafka",
        "ingestMode": "stream",
        "dial": {},
        "sourceObjects": [],
    })
    called = []
    monkeypatch.setattr(
        f, "_run_stream_connector",
        lambda connector, run_id=None: called.append(connector["id"]),
    )
    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-kafka"}}}},
    )
    assert result.success
    assert called == ["conn-kafka"]


def test_a_non_ascii_letter_is_replaced_with_an_underscore_for_dagsters_charset() -> None:
    """Dagster's `check_valid_chars` requires `^[A-Za-z0-9_]+$`.
    `str.isalnum()` is True for non-ASCII letters (`"é".isalnum()`,
    `"²".isalnum()`), so the helper must use the exact ASCII rule
    -- not `c.isalnum()`. This pins that contract: the helper turns
    a non-ASCII letter into `_`, not into itself."""
    import dispar_orchestrate.ingest_factory as f

    assert f._sanitize_target("schéma") == "sch_ma"
    # Both the `.` and the `é` are outside `[A-Za-z0-9_]` and become
    # `_` independently -- two consecutive underscores, no special
    # merging behaviour. The contract is "every non-ASCII char -> `_`",
    # not "non-ASCII runs collapse".
    assert f._sanitize_target("café.orders") == "caf__orders"
    assert f._sanitize_target("plain_ok") == "plain_ok"


def test_no_dynamic_output_value_contains_a_resolved_secret(monkeypatch) -> None:
    """The `DynamicOutput` payload `run_ingest` hands to each mapped
    step is the connector dict as `_fetch_one_connector` returned
    it -- which carries only `secretRef` STRING REFERENCES, never
    a resolved secret value. Dagster's IO manager persists op
    outputs, so a resolved secret becoming one would leak it. This
    test asserts the invariant by snapshotting every yielded
    DynamicOutput's value and confirming no key looks like a
    resolved secret."""
    from dagster import build_op_context

    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    connector = {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {"host": "db.example.com", "driver": "postgres"},
        "secretRef": "env:CONNECTOR_PG_PASSWORD",
        "secretRefSecondary": None,
        "sourceObjects": [
            {"name": "orders", "target": "orders"},
            {"name": "customers", "target": "customers"},
        ],
    }
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)

    # `run_ingest` is a DynamicOut op; invoke directly with a real
    # context and consume the generator.
    gen = f.run_ingest(build_op_context(op_config={"connector_id": "conn-pg"}))
    payloads = []
    try:
        for dyn in gen:
            payloads.append(dyn.value)
    except StopIteration:
        pass

    assert len(payloads) == 2
    for payload in payloads:
        assert payload["connector"]["id"] == "conn-pg"
        # `secretRef` is the unresolved reference (string), not a
        # resolved value. The same holds for `secretRefSecondary`.
        assert payload["connector"]["secretRef"] == "env:CONNECTOR_PG_PASSWORD"
        assert payload["connector"]["secretRefSecondary"] is None
        # No field on the connector dict should hold a non-empty
        # value that looks like an actual secret -- env vars go via
        # `secret_resolver.resolve_secret_ref`, never via the
        # connector dict itself, so any non-empty `password`/
        # `token`/`key` on the connector dict would be the leak this
        # invariant is closing.
        for field in ("password", "token", "api_key", "secret"):
            assert field not in payload["connector"], f"{field!r} leaked into DynamicOutput payload"


# --- SRC-8 task 6: observe a batch SQL table before it loads ------------------
#
# Every test drives the real `_run_one_object` with a fake adapter that
# "reflects" the way `sql_database` does (`_reflect`), a fake sink, and the
# `observations` fixture standing in for `lakehouse-api`.


def _mysql_connector() -> dict:
    return {
        "id": "conn-mysql",
        "adapter": "sql",
        "dial": {"driver": "mysql", "host": "db.internal", "port": 3306, "database": "shop", "user": "u"},
        "secretRef": "env:CONNECTOR_MYSQL_PASSWORD",
        "secretRefSecondary": None,
    }


def _wire_mysql(monkeypatch, *, loads: list, builds: list, table=None):
    """Fake the generic SQL adapter and the sink; `loads` gets one entry per
    row load, `builds` the column names each build left selected."""
    import dispar_orchestrate.ingest_factory as f

    recorded: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f, "_host_of", lambda dial: None)

    class _Adapter:
        def build_source(self, dial, secrets, objs, table_adapter_callback=None):
            fake_table = table or _FakeTable()
            _reflect(table_adapter_callback, fake_table)
            builds.append([c.name for c in fake_table._columns])
            return type("R", (), {"source": object(), "resolved": None})()

    monkeypatch.setitem(f._ADAPTERS, "sql", _Adapter())

    def fake_load(source, target, sink_config, plan):
        loads.append(target)
        return f.sink_adapter.SinkResult(rows=5, has_failed_jobs=False, load_info_str="")

    monkeypatch.setattr(f.sink_adapter, "load_via_sink", fake_load)
    return recorded


def test_the_observation_is_posted_before_any_row_is_loaded(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    loads: list = []
    seen_at_load: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=[])
    original = f.sink_adapter.load_via_sink

    def load_after_checking(*args, **kwargs):
        seen_at_load.append(len(observations.calls))  # observations made by the time rows move
        return original(*args, **kwargs)

    monkeypatch.setattr(f.sink_adapter, "load_via_sink", load_after_checking)
    f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"}, run_id="run-7")

    assert seen_at_load == [1]
    [call] = observations.calls
    assert call["phase"] == "before_load"
    assert call["object"] == "shop.orders"
    assert call["run_id"] == "run-7"
    assert [(c.name, c.type_name, c.nullable) for c in call["columns"]] == [
        ("id", "INTEGER", False),
        ("note", "VARCHAR(40)", True),
    ]
    assert call["primary_key"] == ["id"]
    assert [r["status"] for r in recorded] == ["succeeded"]


def test_a_wait_answer_loads_nothing_records_waiting_and_does_not_raise(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    loads: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=[])
    observations.answer = f.schema_observer.Decision(action="wait", columns=None, changes=[{"kind": "type_changed"}])

    rows = f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"})

    assert rows is None
    assert loads == []
    assert [(r["status"], r["rows"], r["object_name"]) for r in recorded] == [("waiting", None, "shop.orders")]
    assert "schema change" in recorded[0]["error"]
    # A waiting table is not registered in the catalog either: it did not load.


def test_a_wait_answer_for_a_postgres_table_loads_nothing_and_records_waiting(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    recorded: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f.dlt_pipeline.BronzeIngestConfig, "from_dial", staticmethod(lambda *a: object()))
    gates: list = []

    def fake_run_bronze_ingest(cfg, plan, gate=None):
        gates.append(gate(_reflected_table()))
        return {"rows": None, "waiting": True}

    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", fake_run_bronze_ingest)
    observations.answer = f.schema_observer.Decision(action="wait", columns=None, changes=[])
    connector = {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {"driver": "postgres", "host": "pg.internal", "port": 5432, "database": "d", "user": "u"},
        "secretRef": "env:X",
        "secretRefSecondary": None,
    }

    assert f._run_one_object(connector, {"name": "public.orders", "target": "orders"}, run_id="r") is None

    assert [(r["status"], r["rows"]) for r in recorded] == [("waiting", None)]
    assert gates[0].action == "wait"
    assert observations.calls[0]["object"] == "public.orders"
    assert observations.calls[0]["run_id"] == "r"


def _reflected_table():
    from dispar_orchestrate import schema_observer

    return schema_observer.ReflectedTable(
        columns=(schema_observer.ReflectedColumn("id", "integer", False),), primary_key=("id",)
    )


def test_a_wait_answer_for_an_oracle_table_loads_nothing_and_records_waiting(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    recorded: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")

    def fake_oracle(dial, secrets, objs, table_adapter_callback=None):
        _reflect(table_adapter_callback)
        return type("R", (), {"source": iter(()), "resolved": None})()

    monkeypatch.setattr(f.oracle_adapter, "build_source", fake_oracle)
    monkeypatch.setattr(
        f.sink_adapter, "load_via_sink", lambda *a, **k: pytest.fail("a waiting table must not be loaded")
    )
    observations.answer = f.schema_observer.Decision(action="wait", columns=None, changes=[])
    connector = {
        "id": "conn-ora",
        "adapter": "sql",
        "dial": {"driver": "oracle", "host": "ora.internal", "port": 1521, "database": "O", "user": "u"},
        "secretRef": "env:X",
        "secretRefSecondary": None,
    }
    assert f._run_one_object(connector, {"name": "S.ORDERS", "target": "orders"}) is None
    assert [r["status"] for r in recorded] == ["waiting"]


def test_a_column_list_in_the_answer_rebuilds_the_source_with_only_those_columns(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    loads: list = []
    builds: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=builds)
    observations.answer = f.schema_observer.Decision(action="load", columns=["id"], changes=[])

    f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"})

    # First build reflects and keeps everything (nothing is removed yet); the
    # second, after the answer, was built keeping `id` only.
    assert builds == [["id", "note"], ["id"]]
    assert loads == ["orders"]
    assert [r["status"] for r in recorded] == ["succeeded"]
    assert len(observations.calls) == 1  # observed once, not once per build


def test_no_column_list_loads_with_a_single_build(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    builds: list = []
    _wire_mysql(monkeypatch, loads=[], builds=builds)
    f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"})
    assert builds == [["id", "note"]]


def test_an_api_that_cannot_be_reached_fails_the_object_and_loads_nothing(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    loads: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=[])
    observations.answer = f.schema_observer.ObservationUnreachable("the API could not be reached (ConnectionError)")

    with pytest.raises(f.schema_observer.ObservationUnreachable):
        f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"})

    assert loads == []
    assert [r["status"] for r in recorded] == ["failed"]


def test_a_source_build_that_reflects_no_table_is_refused_rather_than_loaded_unchecked(
    monkeypatch, observations
) -> None:
    import dispar_orchestrate.ingest_factory as f

    loads: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=[])

    class _SilentAdapter:
        def build_source(self, dial, secrets, objs, table_adapter_callback=None):
            return type("R", (), {"source": object(), "resolved": None})()  # never calls the callback

    monkeypatch.setitem(f._ADAPTERS, "sql", _SilentAdapter())
    with pytest.raises(f.schema_observer.ReflectionMissing):
        f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"})
    assert loads == [] and observations.calls == []
    assert [r["status"] for r in recorded] == ["rejected"]


def test_the_op_retries_an_unreachable_api_but_not_a_refusal(monkeypatch) -> None:
    from dagster import Failure, build_op_context

    import dispar_orchestrate.ingest_factory as f

    payload = {"connector": {"id": "c"}, "obj": {"name": "orders", "target": "orders"}}

    def raises(exc):
        def fake(_connector, _obj, run_id=None):
            raise exc

        return fake

    monkeypatch.setattr(f, "_run_one_object", raises(f.schema_observer.ObservationUnreachable("down")))
    with pytest.raises(Failure) as unreachable:
        f.ingest_source_object(build_op_context(), payload)
    assert unreachable.value.allow_retries is True  # DEFAULT_RETRY_POLICY keeps retrying a transient failure

    monkeypatch.setattr(f, "_run_one_object", raises(f.schema_observer.ObservationRefused("HTTP 403")))
    with pytest.raises(Failure) as refused:
        f.ingest_source_object(build_op_context(), payload)
    assert refused.value.allow_retries is False


def test_the_op_sends_its_dagster_run_id_with_the_observation(monkeypatch) -> None:
    from dagster import build_op_context

    import dispar_orchestrate.ingest_factory as f

    seen: list = []
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: seen.append(run_id) or None)
    context = build_op_context()
    f.ingest_source_object(context, {"connector": {"id": "c"}, "obj": {"name": "o", "target": "o"}})
    assert seen == [context.run_id]


# --- SRC-8 task 7: observe after loading (files, rest, mongodb, sftp, kafka) --


def _loaded(*columns: tuple[str, str]):
    """A `SinkResult` for a load that produced these columns."""
    from dispar_orchestrate.adapters.sink import LoadedColumn

    return SinkResult(
        rows=3,
        has_failed_jobs=False,
        load_info_str="",
        columns=tuple(LoadedColumn(name, dtype, True) for name, dtype in columns),
    )


def _wire_files(monkeypatch, result) -> list:
    import dispar_orchestrate.ingest_factory as f

    recorded: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f, "_host_of", lambda dial: None)

    class _Adapter:
        def build_source(self, dial, secrets, objs):
            return type("R", (), {"source": object(), "resolved": None})()

    monkeypatch.setitem(f._ADAPTERS, "files", _Adapter())
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: result)
    return recorded


_FILES_CONNECTOR = {
    "id": "conn-files",
    "adapter": "files",
    "dial": {"endpoint": "https://files.internal"},
    "secretRef": "env:X",
    "secretRefSecondary": None,
}
_FILES_OBJECT = {"name": "exports/orders.csv", "target": "orders"}


def test_the_columns_of_a_file_load_are_posted_after_the_load_succeeded(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    recorded = _wire_files(monkeypatch, _loaded(("id", "bigint"), ("note", "text")))
    assert f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT, run_id="run-9") == 3

    [call] = observations.calls
    assert (call["phase"], call["object"], call["run_id"]) == ("after_load", "exports/orders.csv", "run-9")
    assert [(c.name, c.type_name) for c in call["columns"]] == [("id", "bigint"), ("note", "text")]
    # No key is observed for these sources.
    assert call["primary_key"] == []
    assert [r["status"] for r in recorded] == ["succeeded"]


def test_the_columns_of_a_file_load_carry_their_own_name_as_the_loaded_name(monkeypatch, observations) -> None:
    """SRC-8 task 11: after a load the pipeline's names are already the loaded
    ones, so `loadedName` is the name itself."""
    import dispar_orchestrate.ingest_factory as f

    _wire_files(monkeypatch, _loaded(("order_date", "date"), ("note", "text")))
    f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT, run_id="run-9")

    [call] = observations.calls
    assert [(c.name, c.loaded_name) for c in call["columns"]] == [("order_date", "order_date"), ("note", "note")]


def test_the_request_is_just_what_was_loaded_so_a_missing_column_is_not_reported_at_all(
    monkeypatch, observations
) -> None:
    """Run 1 loaded `id` and `note`; run 2 loads a batch with `id` only. This
    side reports `id` and says nothing about `note`: the API carries the
    previous columns over for this phase (D4), so no removal can come from
    here."""
    import dispar_orchestrate.ingest_factory as f

    _wire_files(monkeypatch, _loaded(("id", "bigint"), ("note", "text")))
    f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT)
    _wire_files(monkeypatch, _loaded(("id", "bigint")))
    f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT)

    assert [c.name for c in observations.calls[1]["columns"]] == ["id"]
    assert not any(key in observations.calls[1] for key in ("removed", "missing", "absent"))


def test_dlts_bookkeeping_and_the_ingested_at_column_are_left_out_of_what_a_load_reports(monkeypatch) -> None:
    """Tested where the columns are read (`sink._loaded_columns`) against a real
    `dlt` schema: `_dlt_id`, `_dlt_load_id` and `_ingested_at` belong to the
    pipeline, not to the source, so a first observation must not carry them."""
    import dlt
    from dlt.destinations import filesystem

    from dispar_orchestrate.adapters import sink as sink_module

    for name in (
        "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
    ):
        monkeypatch.delenv(name, raising=False)
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        pipeline = dlt.pipeline(
            pipeline_name="after_load_columns",
            pipelines_dir=f"{tmp}/p",
            destination=filesystem(bucket_url=f"{tmp}/b"),
            dataset_name="bronze",
        )
        rows = [{"id": 1, "note": "x", "_ingested_at": datetime.now(timezone.utc)}]
        pipeline.run(rows, table_name="orders", write_disposition="append")
        columns = sink_module._loaded_columns(pipeline, "orders")

    assert [(c.name, c.data_type) for c in columns] == [("id", "bigint"), ("note", "text")]


def test_a_failed_post_never_turns_a_successful_file_load_into_a_failed_run(monkeypatch, observations, caplog) -> None:
    import logging

    import dispar_orchestrate.ingest_factory as f

    recorded = _wire_files(monkeypatch, _loaded(("id", "bigint")))
    observations.answer = f.schema_observer.ObservationUnreachable("the API could not be reached (ConnectionError)")
    with caplog.at_level(logging.WARNING):
        assert f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT) == 3
    assert [r["status"] for r in recorded] == ["succeeded"]  # recorded once, as a success
    assert "the data is in Bronze" in caplog.text
    assert "ConnectionError" in caplog.text

    # Even an exception the observer never raises on purpose is swallowed.
    observations.answer = RuntimeError("secret-looking detail")
    caplog.clear()
    with caplog.at_level(logging.WARNING):
        assert f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT) == 3
    assert "RuntimeError" in caplog.text and "secret-looking detail" not in caplog.text


def test_a_load_whose_columns_could_not_be_read_posts_nothing(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    _wire_files(monkeypatch, _loaded())
    f._run_one_object(_FILES_CONNECTOR, _FILES_OBJECT)
    assert observations.calls == []


def test_a_sheets_connector_is_not_observed_at_all(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    recorded: list = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    unsupported = type("R", (), {"supported": False, "reason": "not available", "source": None})()
    monkeypatch.setattr(f.sheets_adapter, "build_source", lambda dial, secrets: unsupported)
    connector = {"id": "c", "adapter": "sheets", "dial": {}, "secretRef": "env:X", "secretRefSecondary": None}
    assert f._run_one_object(connector, {"name": "A1:B2", "target": "sheet"}) is None
    assert [r["status"] for r in recorded] == ["unsupported"]
    assert observations.calls == []


def test_a_sql_load_is_observed_once_before_and_not_again_after(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    builds: list = []
    _wire_mysql(monkeypatch, loads=[], builds=builds)
    monkeypatch.setattr(f.sink_adapter, "load_via_sink", lambda *a, **k: _loaded(("id", "bigint")))
    f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders"})
    assert [c["phase"] for c in observations.calls] == ["before_load"]


def test_a_kafka_micro_batch_is_observed_after_it_loaded_and_committed(monkeypatch, observations) -> None:
    order: list = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: BatchResult(rows=[{"id": 1}], offsets_to_commit={0: 5}),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.load_via_sink",
        lambda *a, **k: order.append("load") or _loaded(("id", "bigint"), ("note", "text")),
    )
    monkeypatch.setattr("dispar_orchestrate.ingest_factory.record_ingest_offset", lambda *a, **k: order.append("offset"))
    monkeypatch.setattr("dispar_orchestrate.ingest_factory.record_ingest_run", lambda **kw: order.append(kw["status"]))

    class _Consumer(_FakeStreamConsumer):
        def commit(self, offsets):
            order.append("commit")

    run_kafka_stream_batch(
        connector_id="conn-k",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"name": "orders", "target": "orders"}],
        consumer=_Consumer(),
        run_id="run-k",
    )

    [call] = observations.calls
    assert (call["phase"], call["object"], call["run_id"], call["primary_key"]) == (
        "after_load",
        "orders",
        "run-k",
        [],
    )
    assert [c.name for c in call["columns"]] == ["id", "note"]
    assert order == ["load", "commit", "offset", "succeeded"]


def test_a_failing_post_never_fails_a_kafka_micro_batch(monkeypatch, observations) -> None:
    import dispar_orchestrate.ingest_factory as f

    recorded: list = []
    monkeypatch.setattr(
        "dispar_orchestrate.ingest_factory.consume_one_batch",
        lambda *a, **k: BatchResult(rows=[{"id": 1}], offsets_to_commit={0: 5}),
    )
    monkeypatch.setattr("dispar_orchestrate.ingest_factory.load_via_sink", lambda *a, **k: _loaded(("id", "bigint")))
    monkeypatch.setattr("dispar_orchestrate.ingest_factory.record_ingest_offset", lambda *a, **k: None)
    monkeypatch.setattr("dispar_orchestrate.ingest_factory.record_ingest_run", lambda **kw: recorded.append(kw))
    observations.answer = f.schema_observer.ObservationRefused("HTTP 403")

    run_kafka_stream_batch(
        connector_id="conn-k",
        spec={"topic": "orders"},
        secrets={},
        source_objects=[{"name": "orders", "target": "orders"}],
        consumer=_FakeStreamConsumer(),
    )
    assert [r["status"] for r in recorded] == ["succeeded"]


# --- SRC-8 task 8: new tables under "apply all" (decision D2) -----------------


def _apply_all_connector(driver: str = "postgres", policy: str = "apply_all", names=("public.orders", "sales.items")):
    return {
        "id": "conn-pg",
        "adapter": "sql",
        "schemaChangePolicy": policy,
        "dial": {"driver": driver, "host": "db.internal", "port": 5432, "database": "shop", "user": "u"},
        "secretRef": "env:X",
        "secretRefSecondary": None,
        "sourceObjects": [{"name": n, "target": n.replace(".", "_")} for n in names],
    }


class _Discovery:
    def __init__(self) -> None:
        self.listed: list = []
        self.posts: list = []
        self.tables = {"public": ["orders", "customers"], "sales": ["items", "returns"]}
        self.answer_added = True


@pytest.fixture
def discovery(monkeypatch):
    import dispar_orchestrate.ingest_factory as f
    from dispar_orchestrate import schema_observer

    d = _Discovery()
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")

    def fake_list_tables(spec, secrets, schemas, **kwargs):
        d.listed.append((spec["driver"], secrets, list(schemas)))
        return {schema: d.tables[schema] for schema in schemas}

    def fake_post_new_tables(cfg, connector_id, tables, run_id):
        d.posts.append((connector_id, list(tables), run_id))
        return schema_observer.NewTablesAnswer(added=list(tables) if d.answer_added else [], not_added=[])

    monkeypatch.setattr(f.sql_adapter, "list_tables", fake_list_tables)
    monkeypatch.setattr(f.schema_observer, "post_new_tables", fake_post_new_tables)
    return d


def test_the_tables_of_each_schema_already_loaded_are_listed_and_the_new_ones_posted(discovery) -> None:
    import dispar_orchestrate.ingest_factory as f

    added = f._discover_new_tables(_apply_all_connector(), "run-3")

    assert added is True
    assert discovery.listed == [("postgres", {"password": "s3cret"}, ["public", "sales"])]
    # Only tables not already selected, in schema order, with the run id.
    assert discovery.posts == [("conn-pg", ["public.customers", "sales.returns"], "run-3")]


@pytest.mark.parametrize("driver", ["postgres", "mysql", "mariadb", "mssql"])
def test_apply_all_connectors_of_the_four_listable_drivers_are_looked_at(discovery, driver) -> None:
    import dispar_orchestrate.ingest_factory as f

    assert f._discover_new_tables(_apply_all_connector(driver), None) is True
    assert discovery.listed[0][0] == driver


@pytest.mark.parametrize(
    "connector",
    [
        _apply_all_connector(policy="apply_non_breaking"),
        _apply_all_connector(policy="ask_first"),
        _apply_all_connector(policy="pause"),
        _apply_all_connector("oracle"),
        {**_apply_all_connector(), "adapter": "files"},
        {k: v for k, v in _apply_all_connector().items() if k != "schemaChangePolicy"},
        _apply_all_connector(names=("orders",)),  # no schema to list
    ],
)
def test_nothing_is_listed_for_any_other_policy_driver_adapter_or_schemaless_tables(discovery, connector) -> None:
    import dispar_orchestrate.ingest_factory as f

    assert f._discover_new_tables(connector, None) is False
    assert discovery.listed == [] and discovery.posts == []


def test_nothing_is_posted_when_every_listed_table_is_already_selected(discovery) -> None:
    import dispar_orchestrate.ingest_factory as f

    discovery.tables = {"public": ["orders"], "sales": ["items"]}
    assert f._discover_new_tables(_apply_all_connector(), None) is False
    assert discovery.posts == []


def test_a_batch_larger_than_the_api_accepts_is_sent_in_chunks(discovery) -> None:
    import dispar_orchestrate.ingest_factory as f

    discovery.tables = {"public": [f"t{i:05d}" for i in range(4500)], "sales": []}
    f._discover_new_tables(_apply_all_connector(names=("public.orders",)), None)
    assert [len(tables) for _, tables, _ in discovery.posts] == [2000, 2000, 500]


def test_the_api_adding_nothing_means_the_run_goes_on_with_the_tables_it_has(discovery) -> None:
    import dispar_orchestrate.ingest_factory as f

    discovery.answer_added = False
    assert f._discover_new_tables(_apply_all_connector(), None) is False


def test_a_failing_listing_or_post_never_stops_the_run_and_logs_only_a_safe_text(
    monkeypatch, discovery, caplog
) -> None:
    import logging

    import dispar_orchestrate.ingest_factory as f

    monkeypatch.setattr(
        f.sql_adapter, "list_tables", lambda *a, **k: (_ for _ in ()).throw(ConnectionError("db.internal:5432 s3cret"))
    )
    with caplog.at_level(logging.WARNING):
        assert f._discover_new_tables(_apply_all_connector(), None) is False
    assert "ConnectionError" in caplog.text and "the tables it already has still load" in caplog.text
    assert "s3cret" not in caplog.text and "db.internal" not in caplog.text


def test_a_refused_new_table_report_never_stops_the_run_either(monkeypatch, discovery, caplog) -> None:
    import logging

    import dispar_orchestrate.ingest_factory as f

    monkeypatch.setattr(f.sql_adapter, "list_tables", lambda spec, secrets, schemas, **k: {s: ["new"] for s in schemas})

    def refused(*args, **kwargs):
        raise f.schema_observer.ObservationRefused("the API refused the new-table report (HTTP 409)")

    monkeypatch.setattr(f.schema_observer, "post_new_tables", refused)
    with caplog.at_level(logging.WARNING):
        assert f._discover_new_tables(_apply_all_connector(), None) is False
    assert "HTTP 409" in caplog.text


def test_tables_the_api_added_are_loaded_in_the_same_run(monkeypatch, discovery) -> None:
    """After the add, `run_ingest` reads the connector again, so the fan-out
    covers the new tables: three mapped steps, not two."""
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    before = _apply_all_connector(names=("public.orders",))
    after = {
        **before,
        "sourceObjects": before["sourceObjects"]
        + [{"name": "public.customers", "target": "bronze_customers", "loadMode": "replace"}],
    }
    reads: list = []
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: reads.append(cid) or (before if len(reads) == 1 else after))
    discovery.tables = {"public": ["orders", "customers"]}
    ran: list = []
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: ran.append(obj["name"]) or 1)

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )

    assert result.success
    assert reads == ["conn-pg", "conn-pg"]
    assert sorted(ran) == ["public.customers", "public.orders"]


def test_a_connector_that_gained_no_tables_is_read_once(monkeypatch, discovery) -> None:
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    discovery.answer_added = False
    reads: list = []
    connector = _apply_all_connector(names=("public.orders",))
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: reads.append(cid) or connector)
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: 1)
    discovery.tables = {"public": ["orders", "customers"]}
    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-pg"}}}},
    )
    assert result.success and reads == ["conn-pg"]


# --- SRC-8 task 9: a waiting table is not a failure (decision D7) -------------


def test_a_run_where_one_table_waits_and_another_loads_ends_without_raising_from_either_step(
    monkeypatch, observations
) -> None:
    """The real job graph and the real `_run_one_object`: `shop.orders` is
    told to wait, `shop.items` loads. Both mapped steps SUCCEED, so the run
    does, which is what keeps the run-success sensor from stamping a failure
    and the repeated-failure streak from growing (D7). The waiting table is
    recorded as `waiting`, loads nothing, and is not a materialization."""
    import dispar_orchestrate.ingest_factory as f

    _stub_env(monkeypatch)
    loads: list = []
    recorded = _wire_mysql(monkeypatch, loads=loads, builds=[])
    connector = {
        **_mysql_connector(),
        "sourceObjects": [
            {"name": "shop.orders", "target": "orders"},
            {"name": "shop.items", "target": "items"},
        ],
    }
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)
    observations.answer = lambda name: f.schema_observer.Decision(
        action="wait" if name == "shop.orders" else "load", columns=None, changes=[]
    )

    result = f.ingest_job.execute_in_process(
        raise_on_error=False,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-mysql"}}}},
    )

    assert result.success
    failed = {e.step_key for e in result.all_events if e.event_type_value == "STEP_FAILURE"}
    succeeded = {e.step_key for e in result.all_events if e.event_type_value == "STEP_SUCCESS"}
    assert failed == set()
    assert {"ingest_source_object[orders]", "ingest_source_object[items]"} <= succeeded
    assert {r["object_name"]: r["status"] for r in recorded} == {
        "shop.orders": "waiting",
        "shop.items": "succeeded",
    }
    assert loads == ["items"]
    materialized = [
        e.event_specific_data.materialization.asset_key.to_user_string()
        for e in result.all_events
        if e.event_type_value == "ASSET_MATERIALIZATION"
    ]
    assert materialized == ["bronze/items"]
