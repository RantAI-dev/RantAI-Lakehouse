"""Unit tests for `dagster/dispar_orchestrate/adapters/sink.py` -- the
shared Lakekeeper/Iceberg sink every ingest adapter (SQL, files, REST)
writes through (see `docs/superpowers/plans/2026-09-11-ws3-ingestion-tier1.md`,
the sink-extraction task).

Exercising `load_via_sink` for real needs a live Lakekeeper (covered by
`ops/g3a/g3a_test.py` and Phase I's `ops/g6`), so this module only covers
the PURE, no-network parts: `SinkConfig.from_bronze_ingest_config`'s field
extraction, and `_extract_rows`'s row-count logic.

# Where the row count really comes from (WS3 plan review Z9)

`_extract_rows` reads `pipeline.last_trace.last_normalize_info.row_counts`
(`dlt.pipeline.trace.PipelineTrace.last_normalize_info`, a `NormalizeInfo`
whose `row_counts` property sums `table_metrics[table].items_count` across
every normalize package -- verified against `dlt` 1.30.0, the version
installed in this branch's Dagster venv) keyed by Bronze table name -- the
real count of rows dlt's normalize stage wrote, not an estimate. It is
`None`, never a fabricated `0` (AGENTS.md rule 2, "never fabricate"), in
the three cases below tested individually: the table name absent from
`row_counts`, and `trace` itself being `None`. (The third case named in
the plan -- a `LoadInfo` with `has_failed_jobs` true -- is exercised at
the `load_via_sink` level, not `_extract_rows`, since `has_failed_jobs`
lives on `LoadInfo`, not on the trace `_extract_rows` reads.)

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_sink.py -q
"""

from __future__ import annotations

from datetime import datetime, timezone

import dlt
from dlt.extract.resource import DltResource

from dispar_orchestrate.adapters.sink import (
    SinkConfig,
    _extract_rows,
    _stamp_for_load,
    _stamp_ingested_at,
    load_via_sink,
)
from dispar_orchestrate.dlt_pipeline import BronzeIngestConfig


def test_sink_config_from_bronze_ingest_config_carries_only_sink_fields() -> None:
    cfg = BronzeIngestConfig.from_env()
    sink_cfg = SinkConfig.from_bronze_ingest_config(cfg)
    assert sink_cfg.warehouse_bucket == cfg.warehouse_bucket
    assert sink_cfg.lakekeeper_catalog_uri == cfg.lakekeeper_catalog_uri
    assert sink_cfg.lakekeeper_warehouse == cfg.lakekeeper_warehouse
    assert sink_cfg.rustfs_endpoint == cfg.rustfs_endpoint
    assert sink_cfg.rustfs_access_key == cfg.rustfs_access_key
    assert sink_cfg.rustfs_secret_key == cfg.rustfs_secret_key
    assert sink_cfg.lakekeeper_token == cfg.lakekeeper_token
    assert not hasattr(sink_cfg, "source_db_host")


# WS3 plan review Z9 -- the real row-count source, tested without a live
# pipeline via a minimal stand-in shaped like dlt's own
# PipelineTrace/NormalizeInfo (row_counts is a plain dict property, so a
# stand-in with the same attribute path is enough to pin the EXTRACTION
# logic without a live Lakekeeper).
class _StubNormalizeInfo:
    def __init__(self, row_counts: dict[str, int]) -> None:
        self.row_counts = row_counts


class _StubTrace:
    def __init__(self, row_counts: dict[str, int]) -> None:
        self.last_normalize_info = _StubNormalizeInfo(row_counts)


def test_extract_rows_returns_the_real_normalize_row_count() -> None:
    trace = _StubTrace({"orders": 42})
    assert _extract_rows(trace, "orders") == 42


def test_extract_rows_returns_none_when_the_table_wrote_no_rows_this_run() -> None:
    # Absent from row_counts, NOT the same as "the load never ran" --
    # both are legitimately unknowable from this dict alone, so this is
    # None, never a fabricated 0 (WS3 plan review Z9).
    trace = _StubTrace({"other_table": 5})
    assert _extract_rows(trace, "orders") is None


def test_extract_rows_returns_none_when_there_is_no_trace_at_all() -> None:
    assert _extract_rows(None, "orders") is None


# ADR 0004: `_ingested_at` is stamped by this module, the ONE owner of the
# column, for every shape `load_via_sink` is called with. The bug this
# closes (found by the G6 gate against MySQL/MSSQL): only
# `dlt_pipeline.py::run_bronze_ingest` used to stamp it (via its own
# `add_map`, before reaching this shared sink), so every other adapter
# (SQL/REST/mongodb/files/sftp/sheets/oracle, and the Kafka micro-batch)
# wrote Bronze rows with no `_ingested_at` at all -- `iceberg_adapter`'s
# `day("_ingested_at")` partition spec then failed outright for a
# `sql_database` source with `ValueError: Could not find field with name
# _ingested_at`, and silently produced an under-partitioned/columnless
# table for every adapter that "succeeded" anyway.


def test_stamp_ingested_at_overwrites_a_value_already_on_the_record() -> None:
    # ADR 0004: the column is stamped, never read from the source -- a
    # record that already has its own `_ingested_at` (e.g. a Kafka payload
    # that happens to carry a same-named field) must not keep it.
    stale = datetime(2000, 1, 1, tzinfo=timezone.utc)
    record = {"id": 1, "_ingested_at": stale}

    stamped = _stamp_ingested_at(record)

    assert stamped["_ingested_at"] != stale
    assert stamped["_ingested_at"].tzinfo is not None


def test_stamp_for_load_on_a_dlt_source_stamps_every_row_of_every_resource() -> None:
    @dlt.resource(name="orders")
    def orders():
        yield {"id": 1}
        yield {"id": 2}

    @dlt.resource(name="customers")
    def customers():
        yield {"id": 9}

    @dlt.source(name="two_table_source")
    def two_table_source():
        return [orders(), customers()]

    source = two_table_source()

    stamped_source = _stamp_for_load(source)
    rows = list(stamped_source)

    assert len(rows) == 3
    for row in rows:
        assert "_ingested_at" in row
        assert row["_ingested_at"].tzinfo is not None
        assert row["_ingested_at"].tzinfo == timezone.utc


def test_stamp_for_load_on_a_bare_dlt_resource_stamps_its_rows() -> None:
    @dlt.resource(name="orders")
    def orders():
        yield {"id": 1}

    resource = orders()

    stamped_resource = _stamp_for_load(resource)
    rows = list(stamped_resource)

    assert len(rows) == 1
    assert rows[0]["_ingested_at"].tzinfo == timezone.utc


def test_stamp_for_load_on_a_plain_list_of_dicts_stamps_without_mutating_the_callers_list() -> None:
    # The Kafka micro-batch shape (`ingest_factory.py::run_kafka_stream_batch`'s
    # `batch.rows`) -- a plain list of dicts, not a dlt source/resource.
    original_rows = [{"id": 1}, {"id": 2}]

    stamped_rows = list(_stamp_for_load(original_rows))

    assert len(stamped_rows) == 2
    for row in stamped_rows:
        assert row["_ingested_at"].tzinfo == timezone.utc
    # The caller's own dicts are never touched -- `batch.rows` is read
    # again by `run_kafka_stream_batch` after this call (its own
    # `_record(rows=...)` and the offset-commit bookkeeping), so mutating
    # them in place would be a hidden side effect on that later read.
    assert original_rows == [{"id": 1}, {"id": 2}]


class _FakeLoadInfo:
    has_failed_jobs = False


class _FakePipeline:
    """Stand-in for `dlt.pipeline(...)` that materializes whatever
    `load_via_sink` hands its `run` method, without touching a network or
    a real Lakekeeper -- the same "capture what reaches it" approach this
    module's own row-count tests use for `_extract_rows`."""

    def __init__(self, **kwargs) -> None:
        self.init_kwargs = kwargs
        self.materialized_rows: list[dict] = []
        self.last_trace = None
        self.run_source = None

    def run(self, source, table_name, table_format):
        self.run_source = source
        self.materialized_rows = list(source)
        return _FakeLoadInfo()


def _sink_config() -> SinkConfig:
    return SinkConfig(
        lakekeeper_catalog_uri="http://lakekeeper:8181/catalog",
        lakekeeper_warehouse="default",
        rustfs_endpoint="http://rustfs:9000",
        rustfs_access_key="k",
        rustfs_secret_key="s",
        warehouse_bucket="lakehouse-warehouse",
        lakekeeper_token="",
    )


def test_load_via_sink_stamps_a_kafka_shaped_batch_of_plain_dicts(monkeypatch) -> None:
    fake_pipelines: list[_FakePipeline] = []

    def fake_pipeline_factory(**kwargs):
        pipeline = _FakePipeline(**kwargs)
        fake_pipelines.append(pipeline)
        return pipeline

    monkeypatch.setattr("dispar_orchestrate.adapters.sink.dlt.pipeline", fake_pipeline_factory)

    batch_rows = [{"id": 1}, {"id": 2}]
    result = load_via_sink(batch_rows, "orders", _sink_config())

    assert result.has_failed_jobs is False
    materialized = fake_pipelines[0].materialized_rows
    assert len(materialized) == 2
    for row in materialized:
        assert row["_ingested_at"].tzinfo == timezone.utc
    # The list the Kafka caller still holds is untouched -- see the
    # `_stamp_for_load` test above for why that matters.
    assert batch_rows == [{"id": 1}, {"id": 2}]


def test_load_via_sink_runs_plain_rows_as_a_resource_carrying_the_bronze_partition_spec(monkeypatch) -> None:
    fake_pipelines: list[_FakePipeline] = []

    def fake_pipeline_factory(**kwargs):
        pipeline = _FakePipeline(**kwargs)
        fake_pipelines.append(pipeline)
        return pipeline

    monkeypatch.setattr("dispar_orchestrate.adapters.sink.dlt.pipeline", fake_pipeline_factory)

    load_via_sink([{"id": 1}], "orders", _sink_config())

    run_source = fake_pipelines[0].run_source
    assert isinstance(run_source, DltResource)
    hints = run_source._hints["additional_table_hints"]
    assert hints["x-iceberg-partition"] == [{"transform": "day", "source_column": "_ingested_at"}]
    assert hints["x-iceberg-table-properties"] == {"format-version": "2"}


def test_load_via_sink_stamps_a_single_dlt_resource(monkeypatch) -> None:
    fake_pipelines: list[_FakePipeline] = []

    def fake_pipeline_factory(**kwargs):
        pipeline = _FakePipeline(**kwargs)
        fake_pipelines.append(pipeline)
        return pipeline

    monkeypatch.setattr("dispar_orchestrate.adapters.sink.dlt.pipeline", fake_pipeline_factory)

    @dlt.resource(name="orders")
    def orders():
        yield {"id": 1}
        yield {"id": 2}

    result = load_via_sink(orders(), "orders", _sink_config())

    assert result.has_failed_jobs is False
    materialized = fake_pipelines[0].materialized_rows
    assert len(materialized) == 2
    for row in materialized:
        assert row["_ingested_at"].tzinfo == timezone.utc
