"""Unit tests for `dagster/dispar_orchestrate/assets.py` -- the P3
Bronze-ingest job (`ingest_bronze_table` -> `register_in_catalog`).

No network: `run_bronze_ingest` is monkeypatched with a fake, per this
package's no-network testing rule (`test_maintenance.py`,
`test_capacity_snapshot.py`, `test_backup_job.py` all do the same for
their own HTTP/process-backed functions). The wrap at the op boundary
(`ValueError` from `dlt_pipeline.BronzeIngestConfig.from_dial` ->
`Failure(allow_retries=False)`) is the PART D coverage this file
exists to provide.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_assets.py -q`
"""

from __future__ import annotations

import pytest
from dagster import Failure, build_op_context

from dispar_orchestrate import assets


def test_ingest_bronze_table_wraps_a_from_dial_value_error_in_a_non_retryable_failure(
    monkeypatch,
) -> None:
    """PART D witness: `dlt_pipeline.BronzeIngestConfig.from_dial`
    raises `ValueError` when the dial's `driver` is not
    `postgres`/`postgresql` (`dlt_pipeline.py:198`) or when
    `source_objects` is empty (`dlt_pipeline.py:200`) -- both are
    config rejections, retrying with the same dial gets the same
    answer. `ingest_bronze_table` wraps that raise in
    `Failure(allow_retries=False)`, with the original `ValueError`
    preserved as `__cause__` (the message names the rejected field).
    `RuntimeError` from `dlt_pipeline.run_bronze_ingest`
    (`dlt_pipeline.py:300`, failed dlt load jobs) is NOT wrapped --
    it stays retryable, which is the right shape for a transient /
    load-side failure."""
    def fake_run_bronze_ingest():
        raise ValueError("from_dial only routes driver='postgres' connectors, got 'mysql'")

    monkeypatch.setattr(assets, "run_bronze_ingest", fake_run_bronze_ingest)
    with pytest.raises(Failure) as ctx:
        assets.ingest_bronze_table(build_op_context())
    assert ctx.value.allow_retries is False
    assert isinstance(ctx.value.__cause__, ValueError)
    assert "from_dial only routes driver" in str(ctx.value.__cause__)


def test_ingest_bronze_table_does_not_wrap_a_runtime_error(monkeypatch) -> None:
    """The `RuntimeError` path from `run_bronze_ingest`'s
    `result.has_failed_jobs` branch (`dlt_pipeline.py:300`) is left
    retryable -- a transient load-side failure, not a config rejection.
    The mapped step's `Failure` short-circuit would only block
    retries on config/auth/SSRF/column failures, not on load jobs."""
    def fake_run_bronze_ingest():
        raise RuntimeError("dlt load had failed jobs: synthetic transient")

    monkeypatch.setattr(assets, "run_bronze_ingest", fake_run_bronze_ingest)
    with pytest.raises(RuntimeError, match="dlt load had failed jobs"):
        assets.ingest_bronze_table(build_op_context())