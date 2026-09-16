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

from dispar_orchestrate.adapters.sink import SinkConfig, _extract_rows
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
