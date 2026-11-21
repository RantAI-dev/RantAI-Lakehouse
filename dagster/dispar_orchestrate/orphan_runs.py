"""The runs a previous code location left behind (OPS-37).

# Why this module exists

`DefaultRunLauncher` runs every run of this code location inside the code
server's own process tree (`dagster/_core/launcher/default_run_launcher.py`),
so a container restart kills the runs' processes and leaves their runs
STARTED in the instance. `DefaultRunLauncher` has no worker health check
(`dagster/_core/launcher/base.py`), so the daemon's run monitoring only ends
such a run when `dagster/max_runtime` runs out -- three hours for
`bronze_maintenance_job` (`dagster/dagster.yaml`, OPS-35). On staging, the
04:25Z restart of 2026-11-20 left two such runs STARTED, and because a
STARTED serial run holds the `lakehouse/serial` slot
(`dagster/dagster.yaml`, `concurrency.runs.tag_concurrency_limits`), every
later run of that job waited for them.

A new guard process means a new container, so every run that was still
STARTED when the guard started belongs to a process tree that no longer
exists. This module finds those runs and ends them, ONCE, at guard start.

# What it does

- reads the runs through the Dagster instance API
  (`DagsterInstance.get_run_records`), never through SQL: the instance is
  the only supported way to end a run;
- keeps the runs that are STARTED, belong to THIS code location, and started
  BEFORE the guard started;
- ends each of them with `DagsterInstance.report_run_failed` and a fixed
  message that names the reason.

# What it deliberately does NOT do

- It does not fail a run of ANOTHER code location: `RunsFilter` has no
  code-location field (`dagster/_core/storage/dagster_run.py`), so the
  location is checked here, on the run's own remote origin.
- It does not fail a run that started AFTER the guard: that run belongs to
  the code server that is coming up.
- It does not fail a run whose start time is unknown: such a run cannot be
  shown to predate the guard.
- It does not fail a run that is not STARTED (QUEUED, SUCCESS, FAILURE,
  ...), and it does not cancel anything.
- It does not raise: an instance that cannot be reached, or a run that
  cannot be failed, is logged and the caller goes on. The daemon's run
  monitoring still ends the run when `dagster/max_runtime` runs out.
- It does not run on a timer: the caller runs it once, when the code
  location has answered for the first time.
"""

from __future__ import annotations

import logging
from datetime import datetime, timezone

from dagster import DagsterRunStatus, RunsFilter

logger = logging.getLogger(__name__)

# The message every orphaned run is failed with. It names the reason and the
# finding, so the reader of the run's history knows WHY the run failed
# without reading the container's log.
ORPHAN_FAILURE_MESSAGE = (
    "The code location restarted at {restarted_at}; this run's process ended with it (OPS-37)."
)


def restarted_at_iso(guard_started_at: float) -> str:
    """The guard's start time as ISO-8601 UTC: `2026-11-20T13:31:43Z`."""
    moment = datetime.fromtimestamp(guard_started_at, tz=timezone.utc).replace(
        microsecond=0
    )
    return moment.isoformat().replace("+00:00", "Z")


def orphan_failure_message(guard_started_at: float) -> str:
    """The message one orphaned run is failed with."""
    return ORPHAN_FAILURE_MESSAGE.format(
        restarted_at=restarted_at_iso(guard_started_at)
    )


def run_location_name(run) -> str | None:
    """The code location a run belongs to, or None when it belongs to none.

    A run names its location at
    `remote_job_origin.repository_origin.code_location_origin.location_name`
    (`dagster/_core/storage/dagster_run.py`). A run without a remote origin
    belongs to no location at all, and is never this location's orphan.
    """
    origin = run.remote_job_origin
    if origin is None:
        return None
    return origin.repository_origin.code_location_origin.location_name


def is_orphan(
    record,
    *,
    location_name: str,
    guard_started_at: float,
    now: float,
) -> bool:
    """True when a run record belongs to a process tree that is gone.

    A run is an orphan when it is STARTED, belongs to THIS code location,
    and started BEFORE the guard started: the guard is a new process in a
    new container, and every run of the previous container ran in a process
    tree that died with it.

    `start_time` is the moment the run STARTED, in epoch seconds. It is
    the EXECUTION start, not the moment the run record was written:
    `dagster/_core/storage/dagster_run.py:719-720` says of the field
    "start_time and end_time fields will be populated once the run has
    started and ended, respectively, but will be None beforehand", and it
    sits next to `create_timestamp` (`:717`), the moment the record was
    written. It is None for a run that never started, and such a run
    cannot be shown to predate the guard. That is what makes the rule
    sound: a run that started before the guard started was launched by the
    PREVIOUS container's code server, so its process died with it.

    `now` is the time of the check. A run whose start time is in the future
    relative to it cannot be shown to predate the guard either: the guard's
    container and the host that wrote the start time keep their own clocks.
    """
    run = record.dagster_run
    if run.status != DagsterRunStatus.STARTED:
        return False
    if run_location_name(run) != location_name:
        return False
    start_time = record.start_time
    if start_time is None:
        return False
    if start_time > now:
        return False
    return start_time < guard_started_at


def orphaned_runs(
    instance,
    *,
    location_name: str,
    guard_started_at: float,
    now: float,
) -> list:
    """The run records that are orphans.

    The runs come from the instance, which is asked for the STARTED ones;
    the location and the start time are checked here, in Python, because
    `RunsFilter` cannot filter on either.
    """
    records = instance.get_run_records(
        filters=RunsFilter(statuses=[DagsterRunStatus.STARTED])
    )
    return [
        record
        for record in records
        if is_orphan(
            record,
            location_name=location_name,
            guard_started_at=guard_started_at,
            now=now,
        )
    ]


def fail_orphaned_runs(
    instance,
    *,
    location_name: str,
    guard_started_at: float,
    now: float,
) -> int:
    """Ends every orphaned run; returns how many were ended.

    Never raises. An instance that cannot be read, or a run that cannot be
    ended, is logged and the step goes on: the guard that called this keeps
    watching the code server, and the daemon's run monitoring still ends the
    run when `dagster/max_runtime` runs out.
    """
    try:
        records = orphaned_runs(
            instance,
            location_name=location_name,
            guard_started_at=guard_started_at,
            now=now,
        )
    except Exception as error:
        logger.warning(
            "the runs of the instance could not be read (%s): the runs of the previous "
            "code location are not ended now",
            type(error).__name__,
        )
        return 0
    failed = 0
    for record in records:
        run = record.dagster_run
        try:
            instance.report_run_failed(
                run, message=orphan_failure_message(guard_started_at)
            )
        except Exception as error:
            logger.warning(
                "the orphaned run %s could not be failed (%s)",
                run.run_id,
                type(error).__name__,
            )
            continue
        failed += 1
    return failed
