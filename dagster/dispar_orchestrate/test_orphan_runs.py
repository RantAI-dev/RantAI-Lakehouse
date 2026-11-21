"""The runs a previous code location left behind (OPS-37).

`DefaultRunLauncher` runs every run of this code location inside the code
server's own process tree (`dagster/_core/launcher/default_run_launcher.py`),
so a container restart kills the runs' processes and leaves their runs
STARTED in the instance. `orphan_runs.py` ends those runs ONCE, when the
guard starts, through the Dagster instance API.

These tests use a FAKE instance: no database, no network, no Dagster
instance. The fake exposes the two calls the module makes --
`get_run_records(filters=...)` and `report_run_failed(run, message=...)` --
and the runs it hands back are stand-ins for `DagsterRun` and `RunRecord`
(`dagster/_core/storage/dagster_run.py`).

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_orphan_runs.py -q`
"""

from __future__ import annotations

import logging
from types import SimpleNamespace

import pytest
from dagster import DagsterRunStatus

from dispar_orchestrate import orphan_runs

# The code location this repository runs (`dagster/workspace.yaml`,
# `location_name:`), and the module the compose file starts the code
# location with (`docker-compose.yml`, `--module`).
LOCATION_NAME = "dispar_orchestrate.definitions"

# The guard's start time, as epoch seconds, and the time of the check. The
# timestamp is the one the plan names, so the expected message below is
# written out and not computed by the code under test.
GUARD_STARTED_AT = 1_795_181_503.0
NOW = GUARD_STARTED_AT + 30.0

# The message an orphaned run is failed with, with the guard's start time
# formatted as ISO-8601 UTC.
EXPECTED_MESSAGE = (
    "The code location restarted at 2026-11-20T13:31:43Z; "
    "this run's process ended with it (OPS-37)."
)


def _origin(location_name: str) -> SimpleNamespace:
    """The remote origin of a run of `location_name`.

    A run names its code location at
    `remote_job_origin.repository_origin.code_location_origin.location_name`
    (`dagster/_core/storage/dagster_run.py`).
    """
    return SimpleNamespace(
        repository_origin=SimpleNamespace(
            code_location_origin=SimpleNamespace(location_name=location_name)
        )
    )


class FakeRun:
    """Stands in for a `DagsterRun`."""

    def __init__(
        self,
        *,
        run_id: str,
        status: DagsterRunStatus,
        location_name: str | None,
    ) -> None:
        self.run_id = run_id
        self.status = status
        self.remote_job_origin = (
            None if location_name is None else _origin(location_name)
        )


class FakeRecord:
    """Stands in for a `RunRecord`: the run, and when it started."""

    def __init__(self, run: FakeRun, *, start_time: float | None) -> None:
        self.dagster_run = run
        self.start_time = start_time


class FakeInstance:
    """Stands in for a `DagsterInstance`: no database, no network."""

    def __init__(self, records: list[FakeRecord]) -> None:
        self.records = records
        self.failed: list[tuple[str, str | None]] = []
        self.list_calls = 0
        self.list_error: Exception | None = None
        self.fail_error_for: str | None = None

    def get_run_records(self, filters=None, **kwargs) -> list[FakeRecord]:
        self.list_calls += 1
        if self.list_error is not None:
            raise self.list_error
        return list(self.records)

    def report_run_failed(self, run, message=None) -> None:
        if self.fail_error_for == run.run_id:
            raise RuntimeError("the run cannot be failed")
        self.failed.append((run.run_id, message))


def _run(
    run_id: str,
    *,
    status: DagsterRunStatus = DagsterRunStatus.STARTED,
    location_name: str | None = LOCATION_NAME,
) -> FakeRun:
    return FakeRun(run_id=run_id, status=status, location_name=location_name)


def _fail(
    instance: FakeInstance,
    *,
    guard_started_at: float = GUARD_STARTED_AT,
    now: float = NOW,
    location_name: str = LOCATION_NAME,
) -> int:
    return orphan_runs.fail_orphaned_runs(
        instance,
        location_name=location_name,
        guard_started_at=guard_started_at,
        now=now,
    )


def test_a_started_run_of_this_location_that_started_before_the_guard_is_failed() -> None:
    """A run that was STARTED when the guard started belongs to a process
    tree that died with the previous container: it is failed, with the
    message that names the reason."""
    instance = FakeInstance(
        [FakeRecord(_run("run-1"), start_time=GUARD_STARTED_AT - 60.0)]
    )
    assert _fail(instance) == 1
    assert instance.failed == [("run-1", EXPECTED_MESSAGE)]


def test_a_started_run_that_started_after_the_guard_is_not_touched() -> None:
    """A run that started after the guard belongs to the code server that
    is coming up: it is left alone."""
    instance = FakeInstance(
        [FakeRecord(_run("run-2"), start_time=GUARD_STARTED_AT + 5.0)]
    )
    assert _fail(instance) == 0
    assert instance.failed == []


def test_a_started_run_of_another_location_that_started_before_the_guard_is_not_touched() -> None:
    """Another code location's run is not this container's business: it is
    left alone (Auditor check a)."""
    instance = FakeInstance(
        [
            FakeRecord(
                _run("run-3", location_name="other_location"),
                start_time=GUARD_STARTED_AT - 60.0,
            )
        ]
    )
    assert _fail(instance) == 0
    assert instance.failed == []


def test_a_queued_run_is_not_touched() -> None:
    """A queued run was never launched into a process tree: the code server
    that is coming up will launch it."""
    instance = FakeInstance(
        [
            FakeRecord(
                _run("run-4", status=DagsterRunStatus.QUEUED),
                start_time=GUARD_STARTED_AT - 60.0,
            )
        ]
    )
    assert _fail(instance) == 0
    assert instance.failed == []


def test_a_successful_run_is_not_touched() -> None:
    """A run that finished is never failed again."""
    instance = FakeInstance(
        [
            FakeRecord(
                _run("run-5", status=DagsterRunStatus.SUCCESS),
                start_time=GUARD_STARTED_AT - 60.0,
            )
        ]
    )
    assert _fail(instance) == 0
    assert instance.failed == []


def test_a_run_without_a_code_location_is_not_touched() -> None:
    """A run without a remote origin belongs to no code location, so it
    cannot be shown to be this location's orphan."""
    instance = FakeInstance(
        [
            FakeRecord(
                _run("run-6", location_name=None),
                start_time=GUARD_STARTED_AT - 60.0,
            )
        ]
    )
    assert _fail(instance) == 0
    assert instance.failed == []


def test_a_run_that_never_started_is_not_touched() -> None:
    """A run without a start time cannot be shown to predate the guard."""
    instance = FakeInstance([FakeRecord(_run("run-7"), start_time=None)])
    assert _fail(instance) == 0
    assert instance.failed == []


def test_a_run_with_an_unknown_start_time_does_not_stop_the_other_orphans_from_being_failed() -> None:
    """A run whose start time is unknown cannot be shown to predate the
    guard, so it is left alone, and it does not stop the runs that can be
    shown to predate the guard from being ended."""
    instance = FakeInstance(
        [
            FakeRecord(_run("run-10"), start_time=None),
            FakeRecord(_run("run-11"), start_time=GUARD_STARTED_AT - 60.0),
        ]
    )
    assert _fail(instance) == 1
    assert instance.failed == [("run-11", EXPECTED_MESSAGE)]


def test_a_run_that_started_in_the_future_does_not_stop_the_other_orphans_from_being_failed() -> None:
    """A run whose start time is in the future relative to the check (the
    container's clock and the host that wrote the start time do not agree)
    is left alone, and it does not stop the runs that can be shown to
    predate the guard from being ended."""
    instance = FakeInstance(
        [
            FakeRecord(_run("run-12"), start_time=NOW + 3600.0),
            FakeRecord(_run("run-13"), start_time=GUARD_STARTED_AT - 60.0),
        ]
    )
    assert _fail(instance) == 1
    assert instance.failed == [("run-13", EXPECTED_MESSAGE)]


def test_an_instance_that_cannot_be_read_is_logged_and_the_step_goes_on(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The instance is unreachable: the step logs a warning and returns, so
    the guard keeps watching the code server."""
    instance = FakeInstance([])
    instance.list_error = RuntimeError("the instance is not reachable")
    with caplog.at_level(logging.WARNING):
        failed = _fail(instance)
    assert failed == 0
    assert any(
        "could not be read" in record.message for record in caplog.records
    ), [record.message for record in caplog.records]


def test_a_run_that_cannot_be_failed_is_logged_and_the_other_runs_are_still_failed(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """One run that cannot be ended does not stop the others."""
    instance = FakeInstance(
        [
            FakeRecord(_run("run-8"), start_time=GUARD_STARTED_AT - 60.0),
            FakeRecord(_run("run-9"), start_time=GUARD_STARTED_AT - 60.0),
        ]
    )
    instance.fail_error_for = "run-8"
    with caplog.at_level(logging.WARNING):
        failed = _fail(instance)
    assert failed == 1
    assert instance.failed == [("run-9", EXPECTED_MESSAGE)]
    assert any(
        "could not be failed" in record.message for record in caplog.records
    ), [record.message for record in caplog.records]


def test_the_failure_message_names_the_restart_time_in_utc() -> None:
    """The message carries the guard's start time as ISO-8601 UTC."""
    assert orphan_runs.restarted_at_iso(GUARD_STARTED_AT) == "2026-11-20T13:31:43Z"
    assert orphan_runs.orphan_failure_message(GUARD_STARTED_AT) == EXPECTED_MESSAGE


def test_the_runs_are_read_through_the_instance_api() -> None:
    """The runs come from `get_run_records`, never from SQL: the module
    asks the instance for the STARTED runs."""
    instance = FakeInstance([])
    _fail(instance)
    assert instance.list_calls == 1
