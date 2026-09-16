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

from dispar_orchestrate.ingest_factory import IngestFactoryConfig, build_ingest_schedules


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
