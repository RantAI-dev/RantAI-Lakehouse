"""The jobs that must never overlap another run of THEMSELVES (OPS-37).

Two `bronze_maintenance_job` runs overlapped on staging on 2026-11-18: one
scheduled at 22:08 UTC and one started by hand a minute later, each about
55 minutes long. The instance config held no rule about overlapping runs,
so nothing held the second one back.

The rule has two halves, and this file tests both:

- the TAG: every job on the serial list carries `lakehouse/serial` with
  its OWN name as the value. The tag comes from the ONE helper
  `ingest_factory.job_tags`, next to `run_limit_tags`, so it is never
  written by hand at a call site and a job added later carries it the same
  way.
- the LIMIT: `dagster/dagster.yaml` holds the run-queue rule the daemon
  reads -- one run per VALUE of `lakehouse/serial`, so a second
  `bronze_maintenance_job` waits in the queue while `gold_export_job` may
  run beside it. The global limit stays the ten runs the instance already
  had, now written down.

No network, no database and no real Dagster instance here: the job
definitions and the instance config file only. The one instance this file
builds is a local SQLite one, written into a tmp directory, so the run
queue the daemon reads can be read offline.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_job_serial_tags.py -q`
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import pytest
import yaml
from dagster import DagsterInstance
from dagster._core.instance.ref import InstanceRef
from dagster._utils.tags import TagConcurrencyLimitsCounter

# `agent_runs` builds its schedules at import and `authored_factory` fetches
# the runnable pipelines at import: without a token neither reaches the
# network (`test_job_run_limits.py` clears the same two).
os.environ.pop("AGENT_RUN_TOKEN", None)
os.environ.pop("PIPELINE_RUN_TOKEN", None)

from dispar_orchestrate import (
    alerts_run,
    authored_factory,
    capacity_snapshot,
    discover,
    gold_export,
    gold_latest_copy,
    ingest_factory,
    maintenance,
    storage_snapshot,
)
from dispar_orchestrate.definitions import defs

# The tag the run queue reads, owned by the module that puts it on a job.
SERIAL_TAG = ingest_factory.SERIAL_TAG

# `DATA-66` D6: the per-table sync tag. Its rule is one run per VALUE
# (`<zone>.<mart>`), so two tables sync in parallel while two syncs of one
# table queue.
SYNC_TAG = "lakehouse/iceberg-sync-target"

# Every rule the instance config carries, in the order the file lists them.
EXPECTED_LIMITS = [
    {
        "key": SERIAL_TAG,
        "value": {"applyLimitPerUniqueValue": True},
        "limit": 1,
    },
    {
        "key": SYNC_TAG,
        "value": {"applyLimitPerUniqueValue": True},
        "limit": 1,
    },
]

# The instance config this code location ships, and the file that holds it.
DAGSTER_DIR = Path(__file__).resolve().parents[1]
INSTANCE_CONFIG = DAGSTER_DIR / "dagster.yaml"

# The global run limit the instance already enforced: Dagster's default run
# coordinator (`QueuedRunCoordinator`) allows ten runs at once, and
# `dagster.yaml` now says so instead of leaving it to a library default.
GLOBAL_RUN_LIMIT = 10

# The jobs the plan's decision 1 puts on the serial list, with the module
# that defines each of them.
_SERIAL_JOBS = (
    (maintenance, "bronze_maintenance_job"),
    (capacity_snapshot, "capacity_snapshot_job"),
    (discover, "discover_job"),
    (gold_export, "gold_export_job"),
    (gold_latest_copy, "gold_latest_copy_job"),
    (storage_snapshot, "storage_snapshot_job"),
)

# Jobs that are NOT on the list: they carry no serial tag.
_NOT_SERIAL_JOBS = (
    (ingest_factory, "ingest_job"),
    (alerts_run, "alerts_run_job"),
)

# A pipeline the authored factory can build a job from. The shape is the
# one `authored_factory._op_for_pipeline` reads: an id, a status, and the
# definition's zones and tables.
_SAMPLE_PIPELINE = {
    "id": "pl-serial-tag-1",
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


@dataclass
class _Run:
    """A run, as Dagster's own tag counter sees it: its tags."""

    tags: dict[str, str] = field(default_factory=dict)


@pytest.fixture
def postgres_env(monkeypatch) -> None:
    """The five variables `dagster.yaml` reads for its Postgres storage.

    Throwaway values: the instance config is PARSED here, never connected
    to. The names must be set, because the file reads them through `env:`
    references -- an unset name is a parse error, not a missing database.
    """
    for name, value in (
        ("DAGSTER_PG_HOST", "test-host"),
        ("DAGSTER_PG_PORT", "5432"),
        ("DAGSTER_PG_USER", "test-user"),
        ("DAGSTER_PG_PASSWORD", "testvalue"),
        ("DAGSTER_PG_DB", "test-dagster"),
    ):
        monkeypatch.setenv(name, value)


@pytest.fixture
def serial_counter() -> TagConcurrencyLimitsCounter:
    """Dagster's OWN counter, built from the rule the repo's own config
    holds -- not from a copy of it written here.

    One `bronze_maintenance_job` is in progress, so the counter answers the
    question the daemon asks: may this run start now?
    """
    limits = yaml.safe_load(INSTANCE_CONFIG.read_text())["concurrency"]["runs"]["tag_concurrency_limits"]
    in_progress = [_Run({SERIAL_TAG: "bronze_maintenance_job"})]
    return TagConcurrencyLimitsCounter(limits, in_progress)


@pytest.mark.parametrize("module,attr", _SERIAL_JOBS, ids=[attr for _, attr in _SERIAL_JOBS])
def test_a_serial_job_carries_the_serial_tag_with_its_own_name(module: Any, attr: str) -> None:
    job = getattr(module, attr)
    assert job.tags.get(SERIAL_TAG) == job.name, job.name


@pytest.mark.parametrize("module,attr", _NOT_SERIAL_JOBS, ids=[attr for _, attr in _NOT_SERIAL_JOBS])
def test_a_job_that_is_not_serial_carries_no_serial_tag(module: Any, attr: str) -> None:
    job = getattr(module, attr)
    assert SERIAL_TAG not in job.tags, job.name


def test_an_authored_job_is_not_serial() -> None:
    """The plan's decision 1 lists the jobs that write to the same tables as
    another run of themselves; an authored pipeline is not one of them, so
    its job carries no serial tag."""
    job = authored_factory.build_authored_job(_SAMPLE_PIPELINE)
    assert SERIAL_TAG not in job.tags, job.name


def test_the_instance_config_file_really_parses(postgres_env) -> None:
    """The repo's own `dagster.yaml` must parse: a rule the daemon refuses
    to read is no rule at all. The parsed settings carry the rule, so the
    daemon sees the numbers this file writes."""
    ref = InstanceRef.from_dir(str(DAGSTER_DIR))
    runs = ref.settings["concurrency"]["runs"]
    assert runs["max_concurrent_runs"] == GLOBAL_RUN_LIMIT
    assert runs["tag_concurrency_limits"] == EXPECTED_LIMITS


def test_the_run_queue_the_daemon_reads_holds_the_serial_rule(
    postgres_env, tmp_path: Path, monkeypatch
) -> None:
    """The run queue as the daemon reads it: one run per VALUE of
    `lakehouse/serial`, and the global limit the instance already had.

    The repo's own file names its Postgres storage, and an instance built
    from it would CONNECT to that database. The rule does not live in the
    `storage:` key, so the file is copied without it and the copy is read
    as a local SQLite instance, offline.
    """
    config = yaml.safe_load(INSTANCE_CONFIG.read_text())
    config.pop("storage")
    config_dir = tmp_path / "instance"
    config_dir.mkdir()
    (config_dir / "dagster.yaml").write_text(yaml.safe_dump(config))
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("DAGSTER_HOME", str(home))

    instance = DagsterInstance.from_ref(InstanceRef.from_dir(str(config_dir)))
    try:
        run_queue = instance.get_concurrency_config().run_queue_config
        assert run_queue is not None
        assert run_queue.max_concurrent_runs == GLOBAL_RUN_LIMIT
        assert list(run_queue.tag_concurrency_limits) == [
            {
                "key": limit["key"],
                "limit": limit["limit"],
                "value": limit["value"],
            }
            for limit in EXPECTED_LIMITS
        ]
    finally:
        instance.dispose()


def test_the_raw_instance_config_carries_the_serial_rule() -> None:
    """The file itself, read as YAML: the rule is in the file the daemon
    reads, not only in the object parsed out of it."""
    runs = yaml.safe_load(INSTANCE_CONFIG.read_text())["concurrency"]["runs"]
    assert runs["max_concurrent_runs"] == GLOBAL_RUN_LIMIT
    assert runs["tag_concurrency_limits"] == EXPECTED_LIMITS


def test_a_second_run_of_the_same_serial_job_waits_in_the_queue(serial_counter) -> None:
    assert serial_counter.is_blocked(_Run({SERIAL_TAG: "bronze_maintenance_job"})) is True


def test_a_run_of_another_serial_job_is_not_blocked(serial_counter) -> None:
    assert serial_counter.is_blocked(_Run({SERIAL_TAG: "gold_export_job"})) is False


def test_a_second_sync_of_the_same_table_waits_and_another_table_does_not() -> None:
    """`DATA-66` D6: the per-table rule is what replaces a serial entry
    for `iceberg_sync_job`. A second run of the SAME table waits; a run of
    a different table starts beside it."""
    limits = yaml.safe_load(INSTANCE_CONFIG.read_text())["concurrency"]["runs"]["tag_concurrency_limits"]
    counter = TagConcurrencyLimitsCounter(
        limits, [_Run({SYNC_TAG: "gold.orders"})]
    )
    assert counter.is_blocked(_Run({SYNC_TAG: "gold.orders"})) is True
    assert counter.is_blocked(_Run({SYNC_TAG: "gold.sales"})) is False
    assert counter.is_blocked(_Run({SYNC_TAG: "silver.orders"})) is False
    # A serial job is unaffected by the sync rule.
    assert counter.is_blocked(_Run({SERIAL_TAG: "gold_export_job"})) is False


def test_a_run_without_the_serial_tag_is_not_blocked(serial_counter) -> None:
    assert serial_counter.is_blocked(_Run({"dagster/max_runtime": "10800"})) is False


def test_every_job_of_this_code_location_carries_the_serial_tag_only_when_it_is_serial() -> None:
    """The guard: the walk is over the ACTUAL definitions, not a
    hand-written list, so a job added later fails this test instead of
    shipping without the rule -- or with a tag it must not carry."""
    jobs = list(defs.resolve_all_job_defs())
    assert len(jobs) >= 12, [job.name for job in jobs]
    for job in jobs:
        if job.name in ingest_factory.SERIAL_JOB_NAMES:
            assert job.tags.get(SERIAL_TAG) == job.name, job.name
        else:
            assert SERIAL_TAG not in job.tags, job.name
