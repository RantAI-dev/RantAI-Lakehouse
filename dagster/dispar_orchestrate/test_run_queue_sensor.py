"""The near-limit warning on the run queue (OPS-37, the plan's decision 4).

Ten runs may be in progress at once (`dagster/dagster.yaml`,
`concurrency.runs.max_concurrent_runs`). The next run waits in the queue,
and so does the one after it. Nothing said so: on 2026-11-18 staging's
Dagster stopped launching runs while two `bronze_maintenance_job` runs
overlapped and the inner code server died, and the operator learned about it
from a restart at 02:58 UTC.

This file holds the warning: a sensor that watches how many runs are in
progress and, when that number sits one below the limit for fifteen minutes,
records a skipped tick whose reason names the count and the limit. A skipped
tick is what Dagster keeps in its tick history and logs ("Sensor ... skipped:
..."), so it is the operator's warning without a new API route: the console's
System status page reads Dagster's LIVENESS only (`GET /api/ops/services`),
not the run state, which is why the plan's decision 4 took option (b).

Three halves are tested here:

- the RULE, as a pure function whose clock is an argument, so the fifteen
  minutes are tested without waiting for them. The cursor remembers when the
  queue first came near its limit; the warning fires ONCE per episode, not on
  every tick of a long one;
- the SENSOR, against a REAL instance built from the repo's own
  `dagster/dagster.yaml` (the file without its `storage:` key, so the
  instance is a local SQLite one inside a tmp directory): the limit it reads
  is the repository's own number and the runs it counts are real runs in that
  instance's run storage;
- the DAEMON'S WRITE PATH, because the cursor is the sensor's whole state and
  only survives from one tick to the next if Dagster PERSISTS it. The daemon's
  own `SensorLaunchContext` is driven against a real instance and a real
  sensor state, so what these tests prove is what the daemon does and not
  what this file believes it does.

No network and no Postgres here: the instance is local and offline.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_run_queue_sensor.py -q`
"""

from __future__ import annotations

import json
import logging
import os
import time
from collections.abc import Iterator
from pathlib import Path

import pytest
import yaml
from dagster import (
    DagsterInstance,
    DagsterInvariantViolationError,
    DagsterRun,
    DagsterRunStatus,
    DefaultSensorStatus,
    SensorDefinition,
    SensorEvaluationContext,
    SensorResult,
)
from dagster._core.definitions.sensor_definition import SensorExecutionData, SensorType
from dagster._core.instance.ref import InstanceRef
from dagster._core.remote_origin import (
    RegisteredCodeLocationOrigin,
    RemoteInstigatorOrigin,
    RemoteRepositoryOrigin,
)
from dagster._core.scheduler.instigation import (
    InstigatorState,
    InstigatorStatus,
    InstigatorType,
    SensorInstigatorData,
    TickData,
    TickStatus,
)
from dagster._daemon.sensor import SensorLaunchContext

# `agent_runs` builds its schedules at import and `authored_factory` fetches
# the runnable pipelines at import: without a token neither reaches the
# network (`test_job_serial_tags.py` clears the same two).
os.environ.pop("AGENT_RUN_TOKEN", None)
os.environ.pop("PIPELINE_RUN_TOKEN", None)

from dispar_orchestrate.definitions import defs
from dispar_orchestrate.run_queue_sensor import (
    NEAR_LIMIT_SECONDS,
    SENSOR_SECONDS,
    decide_warning,
    episode_cursor,
    run_queue_near_limit_sensor,
    warning_message,
)

# The instance config this code location ships, and the file that holds it.
DAGSTER_DIR = Path(__file__).resolve().parents[1]
INSTANCE_CONFIG = DAGSTER_DIR / "dagster.yaml"

# The run-queue limit the repository ships, READ from its own file rather
# than written here: the tests then prove the rule against the number this
# deployment runs with, and a change to the file shows up in them.
LIMIT = int(yaml.safe_load(INSTANCE_CONFIG.read_text())["concurrency"]["runs"]["max_concurrent_runs"])

# The mark the rule watches: one run short of the limit.
MARK = LIMIT - 1

# A fixed clock for the pure rule: the time of the first tick of an episode.
T0 = 1_700_000_000.0

# A run coordinator that is NOT the queued one, so the instance has no run
# queue at all. The same class Dagster's own `instance_for_test` uses.
NO_RUN_QUEUE_COORDINATOR = {
    "module": "dagster._core.run_coordinator.sync_in_memory_run_coordinator",
    "class": "SyncInMemoryRunCoordinator",
}


def _build_instance(tmp_path: Path, monkeypatch, **extra: object) -> DagsterInstance:
    """The repo's own instance config, read offline.

    The file names its Postgres storage, and an instance built from it would
    CONNECT to that database. The rule does not live in the `storage:` key,
    so the file is copied without it and the copy is read as a local SQLite
    instance, offline. `DAGSTER_HOME` points at a tmp directory so nothing
    is written into the repository.
    """
    config = yaml.safe_load(INSTANCE_CONFIG.read_text())
    config.pop("storage")
    config.update(extra)
    config_dir = tmp_path / "instance"
    config_dir.mkdir()
    (config_dir / "dagster.yaml").write_text(yaml.safe_dump(config))
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("DAGSTER_HOME", str(home))
    return DagsterInstance.from_ref(InstanceRef.from_dir(str(config_dir)))


@pytest.fixture
def instance(tmp_path: Path, monkeypatch) -> Iterator[DagsterInstance]:
    """The repo's own instance, built as `_build_instance` builds it."""
    built = _build_instance(tmp_path, monkeypatch)
    try:
        yield built
    finally:
        built.dispose()


def _context(instance: DagsterInstance, cursor: str | None) -> SensorEvaluationContext:
    """The context the daemon hands a sensor: the instance, and the cursor the
    last tick left behind."""
    return SensorEvaluationContext(instance_ref=None, instance=instance, cursor=cursor)


# ---------------------------------------------------------------------------
# The rule (pure)
# ---------------------------------------------------------------------------


def test_the_warning_does_not_fire_on_the_first_tick_one_run_below_the_limit() -> None:
    """The first tick at the mark only STARTS the episode: fifteen minutes
    have to pass before the operator is told anything."""
    decision = decide_warning(count=MARK, limit=LIMIT, cursor=None, now=T0)
    assert decision.message is None
    assert decision.cursor is not None


def test_the_warning_fires_once_the_queue_has_been_near_its_limit_for_fifteen_minutes() -> None:
    first = decide_warning(count=MARK, limit=LIMIT, cursor=None, now=T0)
    later = decide_warning(count=MARK, limit=LIMIT, cursor=first.cursor, now=T0 + NEAR_LIMIT_SECONDS)
    assert later.message is not None
    assert str(MARK) in later.message
    assert str(LIMIT) in later.message


def test_the_warning_does_not_fire_two_runs_below_the_limit() -> None:
    """A queue with room to spare is not near its limit, however long it
    stays that way."""
    decision = decide_warning(count=MARK - 1, limit=LIMIT, cursor=None, now=T0 + 10 * NEAR_LIMIT_SECONDS)
    assert decision.message is None
    assert decision.cursor is None


def test_the_warning_does_not_fire_when_the_queue_was_near_its_limit_for_only_ten_minutes() -> None:
    first = decide_warning(count=MARK, limit=LIMIT, cursor=None, now=T0)
    later = decide_warning(count=MARK, limit=LIMIT, cursor=first.cursor, now=T0 + 600)
    assert later.message is None


def test_the_warning_is_not_repeated_while_the_queue_stays_near_its_limit() -> None:
    """One episode, one warning: a queue that stays near its limit for a day
    must not fill the tick history with the same line."""
    first = decide_warning(count=MARK, limit=LIMIT, cursor=None, now=T0)
    fired = decide_warning(count=MARK, limit=LIMIT, cursor=first.cursor, now=T0 + NEAR_LIMIT_SECONDS)
    assert fired.message is not None
    again = decide_warning(
        count=MARK, limit=LIMIT, cursor=fired.cursor, now=T0 + NEAR_LIMIT_SECONDS + SENSOR_SECONDS
    )
    assert again.message is None


def test_the_state_resets_when_the_queue_drops_below_the_mark_so_a_new_episode_can_warn_again() -> None:
    """The episode ends when the queue has room again, and a later episode
    warns on its own."""
    first = decide_warning(count=MARK, limit=LIMIT, cursor=None, now=T0)
    fired = decide_warning(count=MARK, limit=LIMIT, cursor=first.cursor, now=T0 + NEAR_LIMIT_SECONDS)
    assert fired.message is not None
    below = decide_warning(
        count=MARK - 1, limit=LIMIT, cursor=fired.cursor, now=T0 + NEAR_LIMIT_SECONDS + 60
    )
    assert below.message is None
    assert below.cursor is None
    restarted = decide_warning(
        count=MARK, limit=LIMIT, cursor=below.cursor, now=T0 + NEAR_LIMIT_SECONDS + 120
    )
    assert restarted.message is None
    warned_again = decide_warning(
        count=MARK,
        limit=LIMIT,
        cursor=restarted.cursor,
        now=T0 + NEAR_LIMIT_SECONDS + 120 + NEAR_LIMIT_SECONDS,
    )
    assert warned_again.message is not None


def test_the_warning_message_names_the_count_and_the_limit_in_a_fixed_shape() -> None:
    assert warning_message(MARK, LIMIT) == (
        f"{MARK} runs are in progress and the run queue allows {LIMIT}: "
        "the queue is at or near its limit, so new runs wait in the queue."
    )


# ---------------------------------------------------------------------------
# The sensor (against a real instance)
# ---------------------------------------------------------------------------


def test_the_real_sensor_warns_when_the_queue_has_been_near_its_limit_for_twenty_minutes(
    instance: DagsterInstance,
) -> None:
    """The whole path, on the repo's own instance: the limit comes from
    `dagster/dagster.yaml`, the runs are real runs in the run storage, and
    the cursor says the queue has been near its limit for twenty minutes."""
    limit = instance.get_concurrency_config().run_queue_config.max_concurrent_runs
    for index in range(limit - 1):
        instance.add_run(
            DagsterRun(
                job_name="bronze_maintenance_job",
                status=DagsterRunStatus.STARTED,
                run_id=f"near-limit-run-{index}",
            )
        )
    context = _context(instance, episode_cursor(time.time() - 1200))
    result = run_queue_near_limit_sensor.evaluate_tick(context)
    assert result.skip_message is not None
    assert str(limit - 1) in result.skip_message
    assert str(limit) in result.skip_message


def test_the_real_sensor_persists_the_cursor_that_records_the_episode_as_warned(
    instance: DagsterInstance,
) -> None:
    """The warning fires ONCE per episode only because the sensor PERSISTS
    the cursor that marks the episode as warned.

    The cursor is the sensor's whole state, and the daemon persists what the
    tick leaves behind: without the write the previous cursor is kept, the
    episode is never marked as warned, and the same warning repeats on every
    tick of a long episode.
    """
    limit = instance.get_concurrency_config().run_queue_config.max_concurrent_runs
    for index in range(limit - 1):
        instance.add_run(
            DagsterRun(
                job_name="bronze_maintenance_job",
                status=DagsterRunStatus.STARTED,
                run_id=f"warned-episode-run-{index}",
            )
        )
    since = time.time() - 1200
    context = _context(instance, episode_cursor(since))
    result = run_queue_near_limit_sensor.evaluate_tick(context)
    assert result.skip_message is not None
    assert result.cursor is not None
    persisted = json.loads(result.cursor)
    assert persisted["warned"] is True
    assert persisted["since"] == pytest.approx(since)


def test_the_real_sensor_forgets_the_episode_when_the_queue_has_room_again(
    instance: DagsterInstance,
) -> None:
    """Below the mark the episode is over, and the sensor PERSISTS that
    forgetting: the cursor it leaves behind is empty, so a later episode warns
    on its own instead of continuing this one.
    """
    instance.add_run(
        DagsterRun(
            job_name="bronze_maintenance_job",
            status=DagsterRunStatus.STARTED,
            run_id="room-again-run",
        )
    )
    context = _context(instance, episode_cursor(time.time() - 1200))
    result = run_queue_near_limit_sensor.evaluate_tick(context)
    assert result.skip_message is not None
    assert result.cursor is None


def test_the_real_sensor_counts_the_runs_the_daemon_counts(instance: DagsterInstance) -> None:
    """The daemon counts STARTING, STARTED and CANCELING runs as in progress
    (`dagster/_core/storage/dagster_run.py`'s `IN_PROGRESS_RUN_STATUSES`,
    the set `QueuedRunCoordinatorDaemon` filters on). The sensor must count
    the same runs, or it warns about a queue that is not full."""
    for index, status in enumerate(
        (DagsterRunStatus.STARTING, DagsterRunStatus.STARTED, DagsterRunStatus.CANCELING)
    ):
        instance.add_run(
            DagsterRun(job_name="bronze_maintenance_job", status=status, run_id=f"in-progress-{index}")
        )
    context = _context(instance, episode_cursor(time.time() - 1200))
    result = run_queue_near_limit_sensor.evaluate_tick(context)
    assert result.skip_message is not None
    assert result.skip_message.startswith("3 runs are in progress")


def test_the_real_sensor_does_not_warn_while_the_queue_has_room(instance: DagsterInstance) -> None:
    """One run in progress against a limit of ten: nothing to warn about, and
    the tick says so."""
    instance.add_run(
        DagsterRun(job_name="bronze_maintenance_job", status=DagsterRunStatus.STARTED, run_id="one-run")
    )
    context = _context(instance, episode_cursor(time.time() - 1200))
    result = run_queue_near_limit_sensor.evaluate_tick(context)
    assert result.skip_message is not None
    assert str(LIMIT) in result.skip_message


def test_the_real_sensor_does_not_invent_a_limit_when_the_instance_has_no_run_queue(
    tmp_path: Path, monkeypatch
) -> None:
    """An instance whose run coordinator is not the queued one has no run
    queue at all: the sensor says so and warns about nothing."""
    built = _build_instance(tmp_path, monkeypatch, run_coordinator=NO_RUN_QUEUE_COORDINATOR)
    try:
        assert built.get_concurrency_config().run_queue_config is None
        result = run_queue_near_limit_sensor.evaluate_tick(_context(built, None))
        assert result.skip_message == (
            "this Dagster instance has no run-queue limit configured, "
            "so the near-limit warning cannot be checked"
        )
        assert result.cursor is None
    finally:
        built.dispose()


def test_the_near_limit_sensor_is_registered_in_this_code_location() -> None:
    """The warning only reaches the operator when the sensor is registered
    with the code location, and it ships RUNNING: a sensor that no one turns
    on cannot fire."""
    repository = defs.get_repository_def()
    assert repository.has_sensor_def("run_queue_near_limit_sensor")
    sensor_def = repository.get_sensor_def("run_queue_near_limit_sensor")
    assert sensor_def.minimum_interval_seconds == SENSOR_SECONDS
    assert sensor_def.default_status == DefaultSensorStatus.RUNNING


# ---------------------------------------------------------------------------
# The daemon's own write path (the cursor is the sensor's whole state)
# ---------------------------------------------------------------------------

# The sensor's origin in the daemon's own terms: a REAL
# `RemoteInstigatorOrigin`, so the two ids the daemon looks the sensor's
# state up by (the origin id and the selector id) are the ones the
# instance's storage really keys on.
_SENSOR_ORIGIN = RemoteInstigatorOrigin(
    repository_origin=RemoteRepositoryOrigin(
        code_location_origin=RegisteredCodeLocationOrigin(location_name="test_location"),
        repository_name="test_repository",
    ),
    instigator_name="run_queue_near_limit_sensor",
)


class _StubRemoteSensor:
    """The daemon's view of one sensor, without a code location.

    The daemon's sensor loop gets its `RemoteSensor` from the code location
    over gRPC. This stub stands in for it so the daemon's OWN tick
    bookkeeping can be driven offline: the write path under test
    (`_write()` at `dagster/_daemon/sensor.py:232`) reads exactly these four
    things from the remote sensor, and nothing else of it.
    """

    def __init__(self, origin_id: str, selector_id: str) -> None:
        self._origin_id = origin_id
        self.selector_id = selector_id
        self.min_interval_seconds = SENSOR_SECONDS
        self.sensor_type = SensorType.STANDARD

    def get_remote_origin_id(self) -> str:
        return self._origin_id


class _Daemon:
    """The daemon's tick bookkeeping, driven against a real instance.

    The daemon ticks a sensor in three steps: it asks the code location for
    the sensor's execution data, records the answer on its tick, and writes
    the tick to the instance when the tick ends (`dagster/_daemon/sensor.py`).
    This class drives the LAST TWO with the daemon's own class
    (`SensorLaunchContext`) against a real instance and a real sensor state,
    so what the tests below prove is what the daemon does and not what this
    file believes it does.

    The hop over gRPC to the code location is the one step not driven: it
    needs a code-location server, and the execution data it carries is the
    one `evaluate_tick` returns.
    """

    def __init__(self, instance: DagsterInstance) -> None:
        self._instance = instance
        self._remote_sensor = _StubRemoteSensor(
            _SENSOR_ORIGIN.get_id(), _SENSOR_ORIGIN.get_selector().get_id()
        )
        instance.add_instigator_state(
            InstigatorState(
                origin=_SENSOR_ORIGIN,
                instigator_type=InstigatorType.SENSOR,
                status=InstigatorStatus.RUNNING,
                instigator_data=SensorInstigatorData(
                    last_tick_timestamp=None,
                    last_run_key=None,
                    min_interval=SENSOR_SECONDS,
                    cursor=None,
                    last_tick_start_timestamp=None,
                    last_sensor_start_timestamp=None,
                    sensor_type=SensorType.STANDARD,
                    last_tick_success_timestamp=None,
                ),
            )
        )

    def sensor_tick(self, cursor: str | None) -> SensorExecutionData:
        """One tick of the REAL sensor, recorded and persisted by the daemon.

        The cursor handed to the sensor is the one the daemon persisted on
        the last tick (`dagster/_daemon/sensor.py:850`), and the answer is
        recorded on a skipped tick the way the daemon records it (`:887`).
        """
        execution = run_queue_near_limit_sensor.evaluate_tick(_context(self._instance, cursor))
        self._record(execution.skip_message, execution.cursor)
        return execution

    def _record(self, skip_reason: str | None, cursor: str | None) -> None:
        """Put one skipped tick on a fresh tick and let the daemon write it.

        The tick is the one the daemon creates for an evaluation
        (`dagster/_daemon/sensor.py:572-582`): a fresh tick with no cursor,
        which is why a tick that carries no cursor clears the persisted one
        instead of leaving the old one in place.
        """
        tick = self._instance.create_tick(
            TickData(
                instigator_origin_id=self._remote_sensor.get_remote_origin_id(),
                instigator_name="run_queue_near_limit_sensor",
                instigator_type=InstigatorType.SENSOR,
                status=TickStatus.STARTED,
                timestamp=time.time(),
                selector_id=self._remote_sensor.selector_id,
                consecutive_failure_count=0,
            )
        )
        with SensorLaunchContext(
            self._remote_sensor, tick, self._instance, logging.getLogger("dagster.test"), {}
        ) as daemon_context:
            daemon_context.update_state(TickStatus.SKIPPED, skip_reason=skip_reason, cursor=cursor)

    def persisted_cursor(self) -> str | None:
        """The cursor the instance holds for this sensor right now."""
        state = self._instance.get_instigator_state(
            self._remote_sensor.get_remote_origin_id(), self._remote_sensor.selector_id
        )
        assert state is not None and state.instigator_data is not None
        return state.instigator_data.cursor


def _add_in_progress_runs(instance: DagsterInstance, count: int, prefix: str) -> list[DagsterRun]:
    """Add `count` runs the sensor counts as in progress, and return them (so
    a test can let one of them end)."""
    runs = [
        DagsterRun(
            job_name="bronze_maintenance_job",
            status=DagsterRunStatus.STARTED,
            run_id=f"{prefix}-run-{index}",
        )
        for index in range(count)
    ]
    for run in runs:
        instance.add_run(run)
    return runs


def test_the_daemon_persists_the_cursor_the_sensor_set_on_a_skipped_tick(
    instance: DagsterInstance,
) -> None:
    """A skipped tick PERSISTS the cursor: that write is what carries the
    episode from one tick to the next.

    The daemon's own write path is under test: `SensorLaunchContext` holds
    the tick and writes it to the instance when the tick ends
    (`dagster/_daemon/sensor.py:232-282`), and the instance's stored sensor
    state is what the next tick reads.
    """
    daemon = _Daemon(instance)
    limit = instance.get_concurrency_config().run_queue_config.max_concurrent_runs
    _add_in_progress_runs(instance, limit - 1, "persisted-cursor")
    execution = daemon.sensor_tick(None)
    assert execution.cursor is not None
    assert daemon.persisted_cursor() == execution.cursor


def test_the_daemon_clears_the_persisted_cursor_when_the_queue_has_room_again(
    instance: DagsterInstance,
) -> None:
    """The episode's state is FORGOTTEN when the queue has room again: the
    sensor sets the cursor to nothing (`context.update_cursor(None)`) and the
    daemon writes that nothing over the cursor the previous tick persisted.

    The cursor under test was PERSISTED by a tick, not put there by hand: the
    first tick runs with the queue at the mark and leaves the episode behind,
    and the second one runs after a run has ended, so the queue has room.
    Without the clear, a queue that had been full would keep its episode for
    good, and the next episode could never warn on its own.
    """
    daemon = _Daemon(instance)
    limit = instance.get_concurrency_config().run_queue_config.max_concurrent_runs
    runs = _add_in_progress_runs(instance, limit - 1, "room-again")
    daemon.sensor_tick(None)
    assert daemon.persisted_cursor() is not None
    instance.report_run_failed(runs[0], message="the run ended, so the queue has room again")
    execution = daemon.sensor_tick(daemon.persisted_cursor())
    assert execution.cursor is None
    assert daemon.persisted_cursor() is None


def test_the_sensor_warns_once_across_two_ticks_that_read_the_persisted_cursor(
    instance: DagsterInstance,
) -> None:
    """One episode, one warning, across the ticks the daemon persists.

    The second tick does not get its cursor from a local variable: it reads
    the one the daemon persisted on the first tick, the way the daemon hands
    the stored cursor back to the sensor
    (`dagster/_daemon/sensor.py:850`).

    The clock is not faked. The episode's start (`since`) is put twenty
    minutes in the past, which is what a tick that ran twenty minutes ago
    would have persisted; the rule's own clock stays the real one. The
    fifteen minutes are crossed by moving the episode's start, not by moving
    the clock and not by sleeping.
    """
    daemon = _Daemon(instance)
    limit = instance.get_concurrency_config().run_queue_config.max_concurrent_runs
    _add_in_progress_runs(instance, limit - 1, "warn-once")
    first = daemon.sensor_tick(episode_cursor(time.time() - 1200))
    assert first.skip_message is not None
    assert str(limit - 1) in first.skip_message
    assert str(limit) in first.skip_message
    persisted = daemon.persisted_cursor()
    assert persisted is not None
    assert json.loads(persisted)["warned"] is True
    second = daemon.sensor_tick(persisted)
    assert second.skip_message is not None
    assert "at or near its limit" not in second.skip_message
    assert second.cursor == persisted


def test_the_cursor_cannot_be_set_on_the_result_when_the_context_already_carries_one(
    instance: DagsterInstance,
) -> None:
    """The sensor sets its cursor on the CONTEXT and never on the result:
    Dagster refuses that pair
    (`dagster/_core/definitions/sensor_definition.py:966`).

    The refusal is what makes the cursor the sensor's whole state: the value
    the context holds is the value the tick carries
    (`sensor_definition.py:944`), so a result carrying a second cursor would
    be a second, contradictory source of truth.
    """

    def cursor_on_both(context: SensorEvaluationContext) -> SensorResult:
        context.update_cursor("from-the-context")
        return SensorResult(skip_reason="because", cursor="from-the-result")

    sensor_def = SensorDefinition(name="cursor_on_both", evaluation_fn=cursor_on_both)
    with pytest.raises(DagsterInvariantViolationError):
        sensor_def.evaluate_tick(_context(instance, None))

