"""The warning that the run queue is one below its limit (OPS-37, the plan's
decision 4).

Ten runs may be in progress at once on this instance
(`dagster/dagster.yaml`, `concurrency.runs.max_concurrent_runs`). The next
run waits in the queue, and so does the one after it. Nothing said so: on
2026-11-18 staging's Dagster stopped launching runs while two
`bronze_maintenance_job` runs overlapped and the inner code server died, and
the operator learned about it from a restart at 02:58 UTC. The queue that
was full then is the thing this module watches.

# What it does

One sensor, `run_queue_near_limit_sensor`, ticks every five minutes and
counts the runs in progress. When that count sits one below the limit for
fifteen minutes, the tick is recorded as SKIPPED with a reason that names the
count and the limit. Dagster keeps that reason in the tick history and logs
it ("Sensor ... skipped: ..."), so the operator reads the warning there.

The console's System status page was considered first (the plan's decision
4, option (a)) and rejected: it reads Dagster's LIVENESS through
`GET /api/ops/services`, not the run state, so showing this there would mean
a new API route, which the plan puts out of scope. Option (b) it is.

# How the fifteen minutes are remembered

The sensor keeps no state of its own: the CURSOR is the state. It holds the
time the queue first came near its limit and whether the warning already
fired in this episode, so a queue that stays near its limit for a day warns
ONCE, and a queue that drops below the mark and comes back warns again.

A skipped tick PERSISTS the cursor (the daemon writes it unless the tick
failed with an exception), which is what makes the rule work across ticks.
So the cursor is set with `context.update_cursor(...)` and never on the
`SensorResult`: Dagster refuses that pair with a `DagsterInvariantViolationError`.

# Why the rule is a pure function

`decide_warning` holds the whole rule and takes the clock as an argument,
so the fifteen minutes are tested without waiting for them and the sensor
itself stays a thin reader of the instance.

No network and no run is started here: the sensor creates no runs (it is a
job-less sensor) and reads only the instance's own run storage.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_run_queue_sensor.py -q`
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass
from typing import Any

from dagster import (
    DagsterRunStatus,
    DefaultSensorStatus,
    RunsFilter,
    SensorEvaluationContext,
    SensorResult,
    sensor,
)

# The run statuses the daemon counts as in progress when it decides how many
# runs may still be launched: Dagster's own `IN_PROGRESS_RUN_STATUSES`
# (`dagster/_core/storage/dagster_run.py`), the set
# `QueuedRunCoordinatorDaemon._get_in_progress_run_records` filters on. A
# QUEUED run holds no run-queue slot, so it is not counted here either.
_IN_PROGRESS_STATUSES = [
    DagsterRunStatus.STARTING,
    DagsterRunStatus.STARTED,
    DagsterRunStatus.CANCELING,
]

# How long the queue may sit one below its limit before the operator is
# warned: fifteen minutes. The plan's decision 4.
NEAR_LIMIT_SECONDS = 900

# How far below the limit counts as "near": one run short of the limit.
NEAR_LIMIT_MARGIN = 1

# The sensor's tick interval: five minutes, so a fifteen-minute condition is
# noticed within one tick of the moment it ends.
SENSOR_SECONDS = 300

# The fixed line of a tick on an instance without a run queue. The sensor
# does not invent a limit there: there is none to be near.
NO_RUN_QUEUE_REASON = (
    "this Dagster instance has no run-queue limit configured, "
    "so the near-limit warning cannot be checked"
)


@dataclass(frozen=True)
class Decision:
    """What one tick decides: the cursor to keep and the warning to show.

    `message` is None when there is nothing to warn about; `cursor` is None
    when there is no state to keep (the queue is below the mark, or the
    instance has no run queue).
    """

    cursor: str | None
    message: str | None


@dataclass(frozen=True)
class _Episode:
    """One episode of "the queue is near its limit": when it started, and
    whether the operator has been warned in it already."""

    since: float
    warned: bool


def warning_message(count: int, limit: int) -> str:
    """The warning the operator reads: the count and the limit, in a fixed
    sentence shape.

    The shape says "at or near its limit" and not "one below its limit",
    because the rule fires from one below the limit and upwards: at the limit
    itself the second wording would be a lie.
    """
    return (
        f"{count} runs are in progress and the run queue allows {limit}: "
        "the queue is at or near its limit, so new runs wait in the queue."
    )


def episode_cursor(since: float, warned: bool = False) -> str:
    """The cursor of an episode that started at `since`.

    The cursor is the sensor's whole state, so it is written in one place and
    read in one place: `_read_episode`. A cursor that cannot be read is
    treated as no cursor at all (a fresh episode), never as a crash.
    """
    return json.dumps({"since": since, "warned": warned}, sort_keys=True)


def _read_episode(cursor: str | None) -> _Episode | None:
    """The episode the cursor remembers, or None when it remembers none (or
    holds something this sensor cannot read)."""
    if not cursor:
        return None
    try:
        raw = json.loads(cursor)
    except (TypeError, ValueError):
        return None
    if not isinstance(raw, dict):
        return None
    since = raw.get("since")
    if not isinstance(since, (int, float)) or isinstance(since, bool):
        return None
    return _Episode(since=float(since), warned=bool(raw.get("warned")))


def decide_warning(*, count: int, limit: int, cursor: str | None, now: float) -> Decision:
    """The whole rule: what a tick with this count, this limit, this cursor
    and this clock decides.

    The queue is "near its limit" when `count >= limit - 1`. The cursor
    remembers when it first came that close; the warning fires once the
    queue has stayed there for `NEAR_LIMIT_SECONDS`, and not again while it
    stays there. Below the mark the state is forgotten, so a later episode
    warns on its own.

    Never raises: a tick that fails is not persisted by Dagster, so a raise
    would lose the episode instead of ending it.
    """
    if limit < 1:
        # No positive limit: nothing to be near. Dagster's -1 disables the
        # limit and 0 stops every run from launching.
        return Decision(cursor=None, message=None)
    mark = limit - NEAR_LIMIT_MARGIN
    if count < mark:
        # The queue has room again: the episode is over and its state is
        # forgotten, so a new one can start (and warn) later.
        return Decision(cursor=None, message=None)
    episode = _read_episode(cursor)
    if episode is None:
        # The first tick at the mark: the episode starts now.
        return Decision(cursor=episode_cursor(since=now), message=None)
    if episode.warned:
        # Already warned in this episode: the warning does not repeat while
        # the queue stays near its limit.
        return Decision(cursor=episode_cursor(since=episode.since, warned=True), message=None)
    if now - episode.since < NEAR_LIMIT_SECONDS:
        # Near the limit, but not for long enough yet.
        return Decision(cursor=episode_cursor(since=episode.since), message=None)
    return Decision(
        cursor=episode_cursor(since=episode.since, warned=True),
        message=warning_message(count, limit),
    )


def run_queue_limit(instance: Any) -> int | None:
    """The number of runs this instance's run queue allows at once, or None
    when the instance has no run queue (its run coordinator is not the queued
    one, so there is no queue to be near)."""
    run_queue = instance.get_concurrency_config().run_queue_config
    if run_queue is None:
        return None
    return run_queue.max_concurrent_runs


# default_status=RUNNING: a sensor that ships stopped silently never fires,
# and this warning is the operator's only sign of a full queue (the same
# reason as `cdc_apply.py`'s sensor).
@sensor(
    name="run_queue_near_limit_sensor",
    minimum_interval_seconds=SENSOR_SECONDS,
    default_status=DefaultSensorStatus.RUNNING,
    description=(
        "Warns when the number of runs in progress stays one below the run queue's limit for "
        "fifteen minutes. It starts no run: the warning is the skipped tick's reason."
    ),
)
def run_queue_near_limit_sensor(context: SensorEvaluationContext) -> SensorResult:
    """One tick of the near-limit warning.

    The count and the limit come from the instance the daemon runs this
    sensor on; the rule is `decide_warning`'s. The cursor is set with
    `context.update_cursor(...)` and never on the `SensorResult`: Dagster
    refuses that pair, and the cursor is what a skipped tick persists.
    """
    instance = context.instance
    limit = run_queue_limit(instance)
    if limit is None:
        return SensorResult(skip_reason=NO_RUN_QUEUE_REASON)
    if limit < 1:
        return SensorResult(
            skip_reason=(
                f"the run queue of this Dagster instance allows {limit} runs, "
                "not a positive number, so the near-limit warning cannot be checked"
            )
        )
    count = instance.get_runs_count(RunsFilter(statuses=_IN_PROGRESS_STATUSES))
    decision = decide_warning(count=count, limit=limit, cursor=context.cursor, now=time.time())
    context.update_cursor(decision.cursor)
    if decision.message:
        return SensorResult(skip_reason=decision.message)
    return SensorResult(skip_reason=f"{count} runs are in progress and the run queue allows {limit}")
