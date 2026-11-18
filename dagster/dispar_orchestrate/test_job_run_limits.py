"""Every job of this code location carries its own run time limit (OPS-35).

A Dagster run whose process died stays STARTED in the run database: ten
such runs filled staging's ten-run concurrency limit from 13 Nov to 16 Nov
2026 and 570 runs queued behind them. Run monitoring ends a run only past
its `dagster/max_runtime` tag, and until this change only `ingest_job`
carried one (`SRC-39`).

Every job reads its own limit through the ONE helper
`ingest_factory.run_limit_tags`: its own variable, its own default, its own
tag. The parsing rule therefore stays in one place (`AGENTS.md` rule 4).
The tag sits on ONE job, never on `run_monitoring.max_runtime_seconds`,
which would cap every job of the instance (`dagster/dagster.yaml` keeps
that setting unset on purpose).

A job's tags are read ONCE, when its module is imported, because
`run_limit_tags(...)` is the decorator's argument -- so a test that sets
the variable reloads the module, the way `test_storage_snapshot.py` does
it. The original namespace is put back after every reload, so a reload
leaves no trace for the tests that run after this one.

No network and no Dagster instance here: the setting, the tags and the job
definitions only.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_job_run_limits.py -q`
"""

from __future__ import annotations

import importlib
import logging
import os
from typing import Any, NamedTuple

import pytest

# Two of the modules below read the environment when they are imported, and
# both would reach the network with a token set: `agent_runs` builds its
# schedules at import (no `AGENT_RUN_TOKEN` -> no HTTP) and
# `authored_factory` fetches the runnable pipelines at import (no
# `PIPELINE_RUN_TOKEN` -> no HTTP). The ingest job's own limit is cleared
# for the same reason `test_storage_snapshot.py` clears it: its tag is read
# at import.
os.environ.pop("AGENT_RUN_TOKEN", None)
os.environ.pop("PIPELINE_RUN_TOKEN", None)
os.environ.pop("INGEST_RUN_MAX_RUNTIME_SECONDS", None)

from dispar_orchestrate import (
    agent_runs,
    alerts_run,
    assets,
    authored_factory,
    capacity_snapshot,
    cdc_apply,
    discover,
    file_ingest,
    gold_export,
    ingest_factory,
    maintenance,
    replication_metrics,
)
from dispar_orchestrate.definitions import defs

# The tag run monitoring reads (`SRC-39`).
TAG = "dagster/max_runtime"

# The logger the helper warns on: one place parses the setting, so one
# logger names the variable that could not be read.
LIMIT_LOGGER = ingest_factory.logger.name


class _JobSpec(NamedTuple):
    """One job of this code location, with the setting that gives it a
    limit."""

    module: Any
    attr: str
    env_name: str
    default_seconds: int


# Every job of this code location, with the variable that sets its limit and
# the default it falls back to. The defaults are the plan's decision 2
# table; every one of them is at least twice the longest run measured on
# staging on 2026-11-16. `ingest_job` (21600 s) and `storage_snapshot_job`
# (1800 s) keep the settings `SRC-39` and OPS-110 gave them, so they are
# not listed here -- the guard test below still covers them.
_JOB_SPECS = (
    _JobSpec(alerts_run, "alerts_run_job", "DAGSTER_RUN_LIMIT_ALERTS_RUN_JOB_SECONDS", 900),
    _JobSpec(
        replication_metrics,
        "replication_slot_check_job",
        "DAGSTER_RUN_LIMIT_REPLICATION_SLOT_CHECK_JOB_SECONDS",
        900,
    ),
    _JobSpec(capacity_snapshot, "capacity_snapshot_job", "DAGSTER_RUN_LIMIT_CAPACITY_SNAPSHOT_JOB_SECONDS", 1_800),
    _JobSpec(maintenance, "bronze_maintenance_job", "DAGSTER_RUN_LIMIT_BRONZE_MAINTENANCE_JOB_SECONDS", 10_800),
    _JobSpec(gold_export, "gold_export_job", "DAGSTER_RUN_LIMIT_GOLD_EXPORT_JOB_SECONDS", 7_200),
    _JobSpec(agent_runs, "agent_run_job", "DAGSTER_RUN_LIMIT_AGENT_RUN_JOB_SECONDS", 1_800),
    _JobSpec(discover, "discover_job", "DAGSTER_RUN_LIMIT_DISCOVER_JOB_SECONDS", 1_800),
    _JobSpec(assets, "bronze_ingest_job", "DAGSTER_RUN_LIMIT_BRONZE_INGEST_JOB_SECONDS", 21_600),
    _JobSpec(file_ingest, "file_ingest_job", "DAGSTER_RUN_LIMIT_FILE_INGEST_JOB_SECONDS", 21_600),
    _JobSpec(cdc_apply, "cdc_apply_job", "DAGSTER_RUN_LIMIT_CDC_APPLY_JOB_SECONDS", 21_600),
)

# The one variable the authored jobs share: they are built per pipeline
# from the same factory, so they share one limit.
AUTHORED_ENV_NAME = "DAGSTER_RUN_LIMIT_AUTHORED_JOB_SECONDS"
AUTHORED_DEFAULT_SECONDS = 21_600

# A pipeline the authored factory can build a job from. The shape is the
# one `authored_factory._op_for_pipeline` reads: an id, a status, and the
# definition's zones and tables.
_SAMPLE_PIPELINE = {
    "id": "pl-run-limit-1",
    "status": "ready",
    "definition": {
        "sourceZone": "silver",
        "sourceTable": "orders",
        "targetZone": "serving",
        "targetTable": "orders_clean",
        "transforms": ["select(id,name)"],
        "fbicEnabled": False,
    },
}

# The jobs that are long-lived by design and therefore carry no limit.
# EMPTY today: the scout behind the plan's decision 2 table found no such
# job. To exempt one, add its Dagster job name here with a comment saying
# why a limit would end it in normal work (the plan's decision 4).
EXEMPT_JOBS: tuple[str, ...] = ()


@pytest.fixture
def reload_job(monkeypatch):
    """Re-imports a module, so the job it defines carries the environment
    this test sets: a job's tags are read once, when its module is
    imported.

    The original namespace is put back afterwards. A reload rebinds every
    class the module defines to a NEW object, and another test module may
    hold one of them by identity -- `test_file_ingest.py` holds
    `file_ingest.LoadFailure` -- so a reload that stayed would break the
    tests that run after this one.
    """
    saved: list[tuple[Any, dict[str, Any]]] = []

    def _reload(module: Any, **env: str) -> Any:
        for name, value in env.items():
            monkeypatch.setenv(name, value)
        saved.append((module, dict(module.__dict__)))
        return importlib.reload(module)

    yield _reload
    for module, namespace in reversed(saved):
        module.__dict__.clear()
        module.__dict__.update(namespace)


@pytest.mark.parametrize("spec", _JOB_SPECS, ids=[spec.env_name for spec in _JOB_SPECS])
def test_every_job_carries_its_own_run_limit_by_default(spec, reload_job, monkeypatch) -> None:
    monkeypatch.delenv(spec.env_name, raising=False)
    module = reload_job(spec.module)
    job = getattr(module, spec.attr)
    assert TAG in job.tags, spec.env_name
    assert job.tags[TAG] == str(spec.default_seconds), spec.env_name


@pytest.mark.parametrize("spec", _JOB_SPECS, ids=[spec.env_name for spec in _JOB_SPECS])
def test_every_job_reads_its_run_limit_from_its_own_variable(spec, reload_job) -> None:
    module = reload_job(spec.module, **{spec.env_name: "4242"})
    job = getattr(module, spec.attr)
    assert job.tags[TAG] == "4242", spec.env_name


@pytest.mark.parametrize("spec", _JOB_SPECS, ids=[spec.env_name for spec in _JOB_SPECS])
def test_a_job_whose_run_limit_cannot_be_read_falls_back_to_its_default(
    spec, reload_job, caplog
) -> None:
    with caplog.at_level(logging.WARNING, logger=LIMIT_LOGGER):
        module = reload_job(spec.module, **{spec.env_name: "not-a-number"})
    job = getattr(module, spec.attr)
    assert job.tags[TAG] == str(spec.default_seconds), spec.env_name
    assert any(spec.env_name in record.getMessage() for record in caplog.records), spec.env_name


def test_an_authored_job_carries_the_run_limit_of_the_authored_jobs(monkeypatch) -> None:
    monkeypatch.delenv(AUTHORED_ENV_NAME, raising=False)
    job = authored_factory.build_authored_job(_SAMPLE_PIPELINE)
    assert TAG in job.tags
    assert job.tags[TAG] == str(AUTHORED_DEFAULT_SECONDS)


def test_an_authored_job_reads_its_run_limit_from_the_authored_variable(monkeypatch) -> None:
    monkeypatch.setenv(AUTHORED_ENV_NAME, "4242")
    job = authored_factory.build_authored_job(_SAMPLE_PIPELINE)
    assert job.tags[TAG] == "4242"


def test_every_job_of_this_code_location_carries_a_run_limit() -> None:
    """The guard: a job that ships without a limit would hold a run-queue
    slot forever, so every job of the code location must carry one. The
    walk is over the ACTUAL definitions, not a hand-written list, so a job
    added later fails this test instead of shipping without a limit."""
    jobs = list(defs.resolve_all_job_defs())
    assert len(jobs) >= 12, [job.name for job in jobs]
    for job in jobs:
        if job.name in EXEMPT_JOBS:
            continue
        assert TAG in job.tags, job.name
        assert TAG in job.run_tags, job.name
        assert int(job.tags[TAG]) > 0, job.name
    # The authored jobs are built per pipeline at code-load time, so the
    # code location holds none of them without a runnable pipeline: one is
    # built here from a sample pipeline.
    authored = authored_factory.build_authored_job(_SAMPLE_PIPELINE)
    assert TAG in authored.tags, authored.name
