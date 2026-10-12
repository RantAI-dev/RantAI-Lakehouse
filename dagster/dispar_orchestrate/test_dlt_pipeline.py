"""Unit tests for `dagster/dispar_orchestrate/dlt_pipeline.py` (WS3 item
20). This is the first test module covering `dlt_pipeline.py` -- Task
F1's Step 3 verified `ls dagster/dispar_orchestrate/test_*.py` had no
`dlt_pipeline.py` coverage on this branch before this file.

Two things are covered:

1. **The pin.** `run_bronze_ingest`'s Postgres `sql_database` call gains
   the SAME `hostaddr` SSRF pin `adapters/sql.py` gives every other SQL
   driver (`ssrf_guard.resolve_checked` before any credential is built,
   `engine_kwargs={"connect_args": {"hostaddr": <checked ip>}}` alongside
   the unchanged `host` -- libpq performs no DNS lookup of its own once
   `hostaddr` is set, and `host` still drives TLS/SNI + certificate
   verification).
2. **The routing decision (WS3 item 20).** `adapters/sql.py::build_source`
   deliberately refuses `driver in ("postgres", "postgresql")` (see
   `test_adapters_sql.py::test_build_source_refuses_postgres_explicitly`)
   -- a registry-supplied Postgres dial (`rust/migrations/
   0034_seed_connector_ingest_spec.sql`'s seeded `conn-pg-lakehouse` row,
   `adapter='sql'`, `driver='postgres'`) is routed HERE instead, via
   `BronzeIngestConfig.from_dial`, so a future `ingest_factory.py` (Task
   F8) has one unambiguous place to send `driver: postgres`.

No network: `sql_database` and `ssrf_guard.resolve_checked` are always
monkeypatched/injected.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_dlt_pipeline.py -q
"""

from __future__ import annotations

from datetime import timezone

import dlt
import pytest

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.dlt_pipeline import BronzeIngestConfig, run_bronze_ingest


def _base_config(**overrides) -> BronzeIngestConfig:
    fields = dict(
        source_db_host="postgres",
        source_db_port="5432",
        source_db_user="lakehouse",
        source_db_password="lakehouse",
        source_db_name="lakehouse",
        source_schema="ingest_demo",
        source_table="orders",
        bronze_table_name="orders",
        lakekeeper_catalog_uri="http://lakekeeper:8181/catalog",
        lakekeeper_warehouse="default",
        rustfs_endpoint="http://rustfs:9000",
        rustfs_access_key="k",
        rustfs_secret_key="s",
        warehouse_bucket="lakehouse-warehouse",
        lakekeeper_token="",
    )
    fields.update(overrides)
    return BronzeIngestConfig(**fields)


class _FakeSource:
    def __init__(self, table_name: str) -> None:
        self.resources = {table_name: _FakeResource()}


class _FakeResource:
    def __init__(self) -> None:
        self.hints: dict = {}
        self.maps: list = []

    def apply_hints(self, **kwargs) -> None:
        self.hints.update(kwargs)

    def add_map(self, fn) -> None:
        self.maps.append(fn)


class _FakeLoadResult:
    has_failed_jobs = False
    rows = 3


def test_run_bronze_ingest_pins_postgres_via_hostaddr(monkeypatch) -> None:
    captured = {}

    def fake_sql_database(**kwargs):
        captured.update(kwargs)
        return _FakeSource("orders")

    resolve_calls = []

    def fake_resolve_checked(host, port):
        resolve_calls.append((host, port))
        return ssrf_guard.ResolvedAddress(ip="10.0.0.9", port=port, family=2)

    monkeypatch.setattr("dispar_orchestrate.dlt_pipeline.sql_database", fake_sql_database)
    monkeypatch.setattr("dispar_orchestrate.dlt_pipeline.ssrf_guard.resolve_checked", fake_resolve_checked)
    monkeypatch.setattr(
        "dispar_orchestrate.dlt_pipeline.load_via_sink",
        lambda source, table_name, sink_config, plan: _FakeLoadResult(),
    )

    run_bronze_ingest(_base_config())

    assert resolve_calls == [("postgres", 5432)]
    assert captured["engine_kwargs"]["connect_args"]["hostaddr"] == "10.0.0.9"
    # `host` stays the real name -- TLS/SNI and certificate verification
    # must keep targeting it, only the dial address is pinned.
    assert captured["credentials"]["host"] == "postgres"


def test_run_bronze_ingest_refuses_a_blocked_host_before_building_credentials(monkeypatch) -> None:
    def fake_resolve_checked(host, port):
        raise ssrf_guard.SsrfBlocked("refused")

    monkeypatch.setattr(
        "dispar_orchestrate.dlt_pipeline.sql_database",
        lambda **_: pytest.fail("must not be called"),
    )
    monkeypatch.setattr("dispar_orchestrate.dlt_pipeline.ssrf_guard.resolve_checked", fake_resolve_checked)

    with pytest.raises(ssrf_guard.SsrfBlocked):
        run_bronze_ingest(_base_config(source_db_host="169.254.169.254"))


def test_from_dial_builds_a_config_from_a_registry_supplied_postgres_dial() -> None:
    # Mirrors rust/migrations/0034_seed_connector_ingest_spec.sql's
    # seeded conn-pg-lakehouse row shape exactly (dial.driver == "postgres",
    # not "postgresql" -- SqlDriver's #[serde(rename_all = "snake_case")]).
    dial = {
        "driver": "postgres",
        "host": "postgres",
        "port": 5432,
        "database": "lakehouse",
        "user": "lakehouse",
        "sslMode": "disable",
    }
    source_objects = [{"name": "ingest_demo.orders", "incrementalKey": "created_at", "target": "orders"}]

    config = BronzeIngestConfig.from_dial(dial, secrets={"password": "s3cret"}, source_objects=source_objects)

    assert config.source_db_host == "postgres"
    assert config.source_db_port == "5432"
    assert config.source_db_user == "lakehouse"
    assert config.source_db_password == "s3cret"
    assert config.source_db_name == "lakehouse"
    assert config.source_schema == "ingest_demo"
    assert config.source_table == "orders"
    assert config.bronze_table_name == "orders"


def test_from_dial_refuses_a_non_postgres_driver() -> None:
    # WS3 item 20's routing decision only covers driver='postgres'/
    # 'postgresql' -- every other sql-adapter driver routes through
    # adapters/sql.py::build_source instead. Sending the wrong driver here
    # must fail loudly, not silently build a Postgres config for a MySQL
    # dial.
    dial = {"driver": "mysql", "host": "db.internal", "port": 3306, "database": "d", "user": "u"}
    with pytest.raises(ValueError, match="from_dial only routes driver='postgres'"):
        BronzeIngestConfig.from_dial(dial, secrets={"password": "x"}, source_objects=[{"name": "s.t"}])


def test_from_dial_requires_at_least_one_source_object() -> None:
    dial = {"driver": "postgres", "host": "postgres", "port": 5432, "database": "d", "user": "u"}
    with pytest.raises(ValueError, match="at least one source object"):
        BronzeIngestConfig.from_dial(dial, secrets={"password": "x"}, source_objects=[])


class _FakeLoadInfo:
    has_failed_jobs = False


class _FakePipeline:
    """Stand-in for `dlt.pipeline(...)` -- the same capture-what-reaches-it
    approach `test_sink.py`'s own `_FakePipeline` uses, duplicated here
    (not imported) since this test targets `run_bronze_ingest`'s own,
    real, end-to-end path THROUGH the shared sink, not the sink module in
    isolation."""

    def __init__(self, **kwargs) -> None:
        self.materialized_rows: list[dict] = []
        self.last_trace = None
        self.state: dict = {}

    def run(self, source, table_name, table_format, write_disposition):
        self.materialized_rows = list(source)
        return _FakeLoadInfo()


def test_run_bronze_ingest_still_produces_stamped_rows_now_via_the_shared_sink(monkeypatch) -> None:
    # `_stamp_ingested_at` used to be called by THIS module's own
    # `resource.add_map(...)`, before `load_via_sink` was reached. It
    # moved into `adapters/sink.py` (ADR 0004's ONE owner of the column,
    # for every ingest adapter, not just this one) -- this test proves the
    # column still reaches the materialized rows end-to-end through
    # `run_bronze_ingest` -> the REAL `load_via_sink` -> a faked
    # `dlt.pipeline`, rather than mocking `load_via_sink` away entirely
    # the way the hostaddr-pin test above does.
    @dlt.resource(name="orders")
    def orders():
        yield {"id": 1}
        yield {"id": 2}

    def fake_sql_database(**kwargs):
        @dlt.source(name="fake_sql_database")
        def _src():
            return [orders()]

        return _src()

    resolve_calls = []

    def fake_resolve_checked(host, port):
        resolve_calls.append((host, port))
        return ssrf_guard.ResolvedAddress(ip="10.0.0.9", port=port, family=2)

    fake_pipelines: list[_FakePipeline] = []

    def fake_pipeline_factory(**kwargs):
        pipeline = _FakePipeline(**kwargs)
        fake_pipelines.append(pipeline)
        return pipeline

    monkeypatch.setattr("dispar_orchestrate.dlt_pipeline.sql_database", fake_sql_database)
    monkeypatch.setattr("dispar_orchestrate.dlt_pipeline.ssrf_guard.resolve_checked", fake_resolve_checked)
    monkeypatch.setattr("dispar_orchestrate.adapters.sink.dlt.pipeline", fake_pipeline_factory)

    run_bronze_ingest(_base_config())

    materialized = fake_pipelines[0].materialized_rows
    assert len(materialized) == 2
    for row in materialized:
        assert "_ingested_at" in row
        assert row["_ingested_at"].tzinfo == timezone.utc


# --- SRC-8 task 6: the gate between reflecting and loading -------------------


class _GateTable:
    """What `sql_database` hands `table_adapter_callback`."""

    def __init__(self) -> None:
        columns = [("id", "INTEGER", False), ("note", "VARCHAR(40)", True)]
        self._columns = [_GateColumn(n, t, nullable) for n, t, nullable in columns]
        self.columns = self._columns
        self.primary_key = type("PK", (), {"columns": [self._columns[0]]})()


class _GateColumn:
    def __init__(self, name: str, type_text: str, nullable: bool) -> None:
        self.name, self.nullable = name, nullable
        self.type = type_text  # `str(column.type)` is the database's spelling


def _gate_env(monkeypatch):
    builds: list = []
    loads: list = []

    def fake_sql_database(**kwargs):
        table = _GateTable()
        callback = kwargs.get("table_adapter_callback")
        if callback is not None:
            callback(table)
        builds.append({"hostaddr": kwargs["engine_kwargs"]["connect_args"]["hostaddr"], "columns": [c.name for c in table._columns]})
        return _FakeSource("orders")

    monkeypatch.setattr("dispar_orchestrate.dlt_pipeline.sql_database", fake_sql_database)
    monkeypatch.setattr(
        "dispar_orchestrate.dlt_pipeline.ssrf_guard.resolve_checked",
        lambda host, port: ssrf_guard.ResolvedAddress(ip="10.0.0.9", port=port, family=2),
    )
    monkeypatch.setattr(
        "dispar_orchestrate.dlt_pipeline.load_via_sink",
        lambda source, table_name, sink_config, plan: loads.append(table_name) or _FakeLoadResult(),
    )
    return builds, loads


def test_the_gate_sees_the_reflected_table_before_anything_is_loaded(monkeypatch) -> None:
    from dispar_orchestrate import schema_observer

    builds, loads = _gate_env(monkeypatch)
    seen = []

    def gate(table):
        seen.append((table, list(loads)))  # nothing loaded when the gate runs
        return schema_observer.Decision(action="load", columns=None, changes=[])

    outcome = run_bronze_ingest(_base_config(), gate=gate)

    [(table, loads_at_gate)] = seen
    assert [(c.name, c.type_name, c.nullable) for c in table.columns] == [
        ("id", "INTEGER", False),
        ("note", "VARCHAR(40)", True),
    ]
    assert table.primary_key == ("id",)
    assert loads_at_gate == []
    assert loads == ["orders"] and outcome["rows"] == 3
    assert len(builds) == 1


def test_a_wait_from_the_gate_loads_nothing_and_returns_a_waiting_summary(monkeypatch) -> None:
    from dispar_orchestrate import schema_observer

    builds, loads = _gate_env(monkeypatch)
    outcome = run_bronze_ingest(
        _base_config(), gate=lambda table: schema_observer.Decision(action="wait", columns=None, changes=[])
    )
    assert loads == []
    assert outcome["waiting"] is True and outcome["rows"] is None


def test_a_column_list_from_the_gate_rebuilds_the_source_under_the_same_hostaddr_pin(monkeypatch) -> None:
    from dispar_orchestrate import schema_observer

    builds, loads = _gate_env(monkeypatch)
    run_bronze_ingest(
        _base_config(), gate=lambda table: schema_observer.Decision(action="load", columns=["id"], changes=[])
    )
    # Both builds (the reflection and the one that loads) dial the checked address.
    assert [b["hostaddr"] for b in builds] == ["10.0.0.9", "10.0.0.9"]
    assert [b["columns"] for b in builds] == [["id", "note"], ["id"]]
    assert loads == ["orders"]


def test_without_a_gate_the_source_is_built_once_and_without_a_callback(monkeypatch) -> None:
    builds, loads = _gate_env(monkeypatch)
    run_bronze_ingest(_base_config())
    assert len(builds) == 1 and loads == ["orders"]
