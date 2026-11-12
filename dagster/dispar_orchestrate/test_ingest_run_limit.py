"""The time limit every ingest run carries (`SRC-39`, QA `TC-1702`).

A run that never finishes holds its connector's lock for days: the API
refuses the next run with 409 while the orchestrator lists one of the
connector's runs as queued or going, and only a run that really ends frees
it. The orchestrator's run monitoring is what ends a stuck run; it reads
the `dagster/max_runtime` tag this module puts on every `ingest_job` run.

The tag is a tag on `ingest_job` alone, never
`run_monitoring.max_runtime_seconds`: that field would cap every job of the
instance. Six hours is the bound the console already uses to treat a run
with no end as stopped (`STALE_RUN_MS`); a load that needs more raises the
setting. It is only enforced while run monitoring is on, which
`dagster/dagster.yaml` turns on (T10).

No network and no Dagster instance: the setting and the job's tags only.
"""

from __future__ import annotations

import logging

import pytest

from dispar_orchestrate import ingest_factory as f

TAG = "dagster/max_runtime"
SIX_HOURS = 21_600


@pytest.fixture(autouse=True)
def _no_run_limit(monkeypatch):
    monkeypatch.delenv(f.RUN_LIMIT_ENV, raising=False)


def test_an_ingest_run_is_limited_to_six_hours_by_default() -> None:
    assert f.RunLimitSettings.from_env().max_runtime_seconds == SIX_HOURS
    assert f._ingest_job_tags() == {TAG: str(SIX_HOURS)}


def test_the_run_limit_comes_from_the_setting(monkeypatch) -> None:
    monkeypatch.setenv(f.RUN_LIMIT_ENV, "900")
    assert f.RunLimitSettings.from_env().max_runtime_seconds == 900
    assert f._ingest_job_tags() == {TAG: "900"}


@pytest.mark.parametrize("bad", ["abc", "0", "-5", "1.5", "  "])
def test_a_run_limit_that_cannot_be_read_falls_back_to_six_hours(monkeypatch, caplog, bad) -> None:
    monkeypatch.setenv(f.RUN_LIMIT_ENV, bad)
    with caplog.at_level(logging.WARNING, logger=f.logger.name):
        settings = f.RunLimitSettings.from_env()
    assert settings.max_runtime_seconds == SIX_HOURS
    if bad.strip():  # a blank value is the same as unset: nothing to complain about
        assert any(f.RUN_LIMIT_ENV in r.getMessage() and "default" in r.getMessage() for r in caplog.records)
        # The value itself is never echoed. The default happens to contain the
        # digit `0`, so the bad value is looked for with the default removed.
        assert bad.strip() not in caplog.text.replace(str(SIX_HOURS), "")


def test_the_ingest_job_and_its_runs_carry_the_limit_tag() -> None:
    assert TAG in f.ingest_job.tags
    assert TAG in f.ingest_job.run_tags
