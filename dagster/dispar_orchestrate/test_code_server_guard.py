"""The code server guard: what makes a dead inner server restart (OPS-37).

On staging, from 2026-11-18 22:15 UTC to 2026-11-19 02:58 UTC, Dagster
stopped launching runs: the inner code server of the `dagster-code-location`
container died and the `code-server start` proxy never respawned it, so the
container stayed "Up" while the daemon failed 1550 times.

`code_server_guard.py` is the container's main process now. This file tests
the parts that can be tested OFFLINE, with no network and no container:

- the RULE that decides to give up (a pure function, with its whole truth
  table);
- the probe, against a port nothing listens on;
- the `--healthcheck` mode, which exits 1 when nothing answers;
- the command line the compose file passes, and the child command the guard
  builds from it -- the same code server command the compose file had
  before, plus the heartbeat TTL;
- the SIGTERM path: the signal reaches the code server and the guard exits
  0, so `docker compose stop` stops the container cleanly;
- the runs the previous code location left behind: they are failed ONCE,
  after the code location answered for the first time;
- the missed-heartbeat line of the inner server: the guard forwards the
  code server's output, reloads the code location when the line appears,
  and gives up when it appears twice in a row.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_code_server_guard.py -q`
"""

from __future__ import annotations

import logging
import os
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from dispar_orchestrate import code_server_guard

# The directory the `dispar_orchestrate` package lives in, so the
# subprocess of the healthcheck test finds the module wherever pytest was
# started from.
DAGSTER_DIR = Path(__file__).resolve().parents[1]

# The command the compose file gave the code location BEFORE the guard
# (`docker-compose.yml`, `dagster-code-location`). The guard's child must be
# that command with those arguments: the guard changes WHO watches the code
# server, never WHAT the code server is.
PREVIOUS_COMPOSE_COMMAND = [
    "dagster",
    "code-server",
    "start",
    "-h",
    "0.0.0.0",
    "-p",
    "4000",
    "-m",
    "dispar_orchestrate.definitions",
]


def _closed_port() -> int:
    """A port nothing listens on: bound, then released."""
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _guard_args(port: int) -> object:
    """The arguments the guard runs with, for a port nothing answers."""
    return code_server_guard.build_parser().parse_args(
        [
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--module",
            "dispar_orchestrate.definitions",
        ]
    )


class _StopGuard(Exception):
    """Ends the guard's loop from a test.

    The guard's loop has no other way to be stopped from the outside, and
    a test must not wait 30 s for a probe interval.
    """


def _sleeping_child() -> subprocess.Popen:
    """A code server that stays alive: the guard watches it."""
    return subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])


def _child_printing(script: str, *lines: str) -> subprocess.Popen:
    """A code server that writes `lines` to its output, then waits.

    Its stdout and stderr are pipes, so the guard reads them and forwards
    what it reads.
    """
    return subprocess.Popen(
        [sys.executable, "-c", script, *lines],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


# A child that writes one line, then waits.
_CHILD_ONE_LINE = """
import sys, time
print(sys.argv[1], flush=True)
time.sleep(30)
"""

# A child that writes both lines at once.
_CHILD_TWO_LINES = """
import sys, time
print(sys.argv[1], flush=True)
print(sys.argv[2], flush=True)
time.sleep(30)
"""

# A child that writes the second line only after the guard asks for it: the
# test writes to the child's stdin, so the second line arrives at a moment
# the test chose.
_CHILD_SECOND_LINE_ON_ASK = """
import sys, time
print(sys.argv[1], flush=True)
sys.stdin.readline()
print(sys.argv[2], flush=True)
time.sleep(30)
"""

# A child that writes one line to stdout and one to stderr.
_CHILD_TWO_STREAMS = """
import sys, time
print(sys.argv[1], flush=True)
print(sys.argv[2], file=sys.stderr, flush=True)
time.sleep(30)
"""


class _Forwarded:
    """What the guard forwarded, collected from the captured output.

    The guard's reader threads write to `sys.stdout` and `sys.stderr`, so
    pytest's capture sees the code server's output.
    """

    def __init__(self, capsys: pytest.CaptureFixture) -> None:
        self.capsys = capsys
        self.out = ""
        self.err = ""

    def collect(self) -> None:
        captured = self.capsys.readouterr()
        self.out += captured.out
        self.err += captured.err


def _probe_waiting_for(
    forwarded: _Forwarded,
    line: str,
    *,
    reloads: list,
    stop_when_reloads: int,
    iterations_after: int = 3,
):
    """A probe that waits until the guard forwarded `line`.

    The guard forwards a line BEFORE it acts on it, so a test that has seen
    the line knows the reader thread has read it: the guard's next round
    drains it. The probe ends the guard's loop when the test's condition is
    met, or after a few rounds past the line.
    """
    state = {"seen": False, "after": 0}

    def probe(*, host: str, port: int) -> bool:
        forwarded.collect()
        if not state["seen"]:
            if line not in forwarded.out and line not in forwarded.err:
                time.sleep(0.01)
                return True
            state["seen"] = True
        state["after"] += 1
        if len(reloads) >= stop_when_reloads or state["after"] > iterations_after:
            raise _StopGuard()
        return True

    return probe


class _RecordingInstance:
    """A Dagster instance that records what the guard asked it to do."""

    def __init__(self, events: list[str]) -> None:
        self.events = events
        self.failed: list[tuple[str, str | None]] = []

    def get_run_records(self, filters=None, **kwargs) -> list:
        self.events.append("instance")
        return []

    def report_run_failed(self, run, message=None) -> None:
        self.failed.append((run.run_id, message))


def _probe_recording(events: list[str], answers: list[bool]):
    """A probe that answers as told, records every call, and ends the
    guard's loop when the answers run out."""
    calls = 0

    def probe(*, host: str, port: int) -> bool:
        nonlocal calls
        calls += 1
        if calls > len(answers):
            raise _StopGuard()
        answered = answers[calls - 1]
        events.append("probe:yes" if answered else "probe:no")
        return answered

    return probe


@pytest.mark.parametrize(
    "state,child_exited,expected",
    [
        pytest.param(
            code_server_guard.ProbeState(succeeded_once=True, consecutive_failures=0),
            True,
            True,
            id="the-code-server-process-exited",
        ),
        pytest.param(
            code_server_guard.ProbeState(succeeded_once=False, consecutive_failures=0),
            True,
            True,
            id="the-code-server-process-exited-before-any-ping",
        ),
        pytest.param(
            code_server_guard.ProbeState(succeeded_once=True, consecutive_failures=4),
            False,
            False,
            id="four-pings-in-a-row-failed-after-a-success",
        ),
        pytest.param(
            code_server_guard.ProbeState(succeeded_once=True, consecutive_failures=5),
            False,
            True,
            id="five-pings-in-a-row-failed-after-a-success",
        ),
        pytest.param(
            code_server_guard.ProbeState(succeeded_once=False, consecutive_failures=9),
            False,
            False,
            id="nine-pings-failed-without-a-success",
        ),
        pytest.param(
            code_server_guard.ProbeState(succeeded_once=False, consecutive_failures=10),
            False,
            True,
            id="ten-pings-failed-without-a-success",
        ),
    ],
)
def test_the_guard_gives_up_exactly_when_the_rule_says_so(
    state: code_server_guard.ProbeState, child_exited: bool, expected: bool
) -> None:
    assert (
        code_server_guard.should_give_up(state, child_exited=child_exited) is expected
    )


def test_a_successful_probe_resets_the_failure_streak() -> None:
    """One answer after four failures starts the count again: the streak is
    of the pings IN A ROW, not of all the pings."""
    state = code_server_guard.ProbeState(succeeded_once=False, consecutive_failures=4)
    after_success = state.after_probe(True)
    assert after_success.succeeded_once is True
    assert after_success.consecutive_failures == 0
    assert (
        code_server_guard.should_give_up(
            after_success, child_exited=False
        )
        is False
    )


def test_a_failed_probe_after_a_success_keeps_the_streak() -> None:
    state = code_server_guard.ProbeState(succeeded_once=True, consecutive_failures=2)
    after_failure = state.after_probe(False)
    assert after_failure.succeeded_once is True
    assert after_failure.consecutive_failures == 3


def test_the_probe_reports_a_port_nothing_listens_on_as_no_answer() -> None:
    assert code_server_guard.probe(host="127.0.0.1", port=_closed_port()) is False


def test_the_healthcheck_mode_exits_one_when_nothing_answers_the_port() -> None:
    """The compose healthcheck reads the exit code: 1 means the container is
    unhealthy, and the guard's own process exit is what restarts it."""
    port = _closed_port()
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "dispar_orchestrate.code_server_guard",
            "--healthcheck",
            "--port",
            str(port),
        ],
        cwd=str(DAGSTER_DIR),
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert result.returncode == 1, result.stdout + result.stderr


def test_the_command_line_parser_takes_the_flags_the_compose_file_passes() -> None:
    args = code_server_guard.build_parser().parse_args(
        [
            "--host",
            "0.0.0.0",
            "--port",
            "4000",
            "--module",
            "dispar_orchestrate.definitions",
        ]
    )
    assert args.host == "0.0.0.0"
    assert args.port == 4000
    assert args.module == "dispar_orchestrate.definitions"
    assert args.healthcheck is False


def test_the_command_line_parser_takes_the_healthcheck_flag() -> None:
    args = code_server_guard.build_parser().parse_args(
        ["--healthcheck", "--port", "4000"]
    )
    assert args.healthcheck is True
    assert args.port == 4000


def test_the_child_command_is_the_code_server_command_the_compose_file_had_before_plus_the_heartbeat_ttl() -> None:
    """The child is still the code server command the compose file had
    before -- the guard changes WHO watches the code server, never WHAT the
    code server is -- and it carries the heartbeat TTL, which the compose
    file does not pass."""
    command = code_server_guard.child_command(
        host="0.0.0.0",
        port=4000,
        module="dispar_orchestrate.definitions",
        heartbeat_ttl_seconds=300,
    )
    assert command[0] == sys.executable
    assert command[1:] == [
        "-m",
        "dagster",
        *PREVIOUS_COMPOSE_COMMAND[1:],
        "--heartbeat-ttl",
        "300",
    ]


def test_the_child_command_does_not_carry_the_heartbeat_flag() -> None:
    """`--heartbeat` would make the PROXY write the very line the guard
    matches on (`dagster/_grpc/proxy_server.py`), so every reload would be
    triggered by the proxy's own heartbeat instead of the inner server's."""
    command = code_server_guard.child_command(
        host="0.0.0.0",
        port=4000,
        module="dispar_orchestrate.definitions",
        heartbeat_ttl_seconds=300,
    )
    assert "--heartbeat" not in command
    assert "--heartbeat-timeout" not in command


def test_the_child_environment_does_not_carry_the_guard_probe_timeout(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The guard sets `DAGSTER_GRPC_TIMEOUT_SECONDS` for its OWN ping; the
    code server keeps Dagster's default for the calls the daemon makes
    through the proxy."""
    monkeypatch.setattr(code_server_guard, "CONTAINER_GRPC_TIMEOUT_SECONDS", None)
    env = code_server_guard.child_environment(
        {"DAGSTER_PG_HOST": "postgres", "DAGSTER_GRPC_TIMEOUT_SECONDS": "10"}
    )
    assert "DAGSTER_GRPC_TIMEOUT_SECONDS" not in env
    assert env["DAGSTER_PG_HOST"] == "postgres"


def test_the_child_environment_keeps_the_timeout_the_container_was_given(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """An operator who sets the timeout for the code server keeps it: the
    guard's own probe timeout is not passed on, the container's is."""
    monkeypatch.setattr(code_server_guard, "CONTAINER_GRPC_TIMEOUT_SECONDS", "30")
    env = code_server_guard.child_environment({"DAGSTER_GRPC_TIMEOUT_SECONDS": "10"})
    assert env["DAGSTER_GRPC_TIMEOUT_SECONDS"] == "30"


def test_a_sigterm_reaches_the_code_server_and_the_guard_exits_zero() -> None:
    """`docker compose stop` sends SIGTERM to the container's PID 1, which
    is the guard. As PID 1 the kernel does not apply the default action, so
    the guard installs its own handler, forwards the signal to the code
    server, and exits 0: a stopped container is not a failure."""
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
    timer = threading.Timer(0.5, os.kill, args=(os.getpid(), signal.SIGTERM))
    timer.start()
    try:
        exit_code = code_server_guard.run_guard(
            _guard_args(_closed_port()), spawn=lambda **_kwargs: child
        )
    finally:
        timer.cancel()
    assert exit_code == 0
    # The child must have died OF THE SIGNAL, not of the guard's kill: a
    # negative return code is the signal that killed it.
    assert child.poll() is not None, "the code server did not get the signal"
    assert child.returncode == -signal.SIGTERM, child.returncode


def test_the_guard_exits_non_zero_when_the_code_server_process_dies() -> None:
    """A dead code server PROCESS means the container cannot serve the code
    location: the guard exits non-zero and `restart: unless-stopped`
    restarts the container."""
    child = subprocess.Popen([sys.executable, "-c", "raise SystemExit(3)"])
    exit_code = code_server_guard.run_guard(
        _guard_args(_closed_port()), spawn=lambda **_kwargs: child
    )
    assert exit_code == 1


# --- the heartbeat TTL (decision 2) ------------------------------------------


def test_the_heartbeat_ttl_default_is_300_seconds(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Five minutes: long enough that a busy container does not miss the
    inner server's heartbeat, short enough that an orphaned inner server
    ends by itself."""
    monkeypatch.delenv(code_server_guard.HEARTBEAT_TTL_ENV, raising=False)
    monkeypatch.delenv(code_server_guard.DAGSTER_HEARTBEAT_TTL_ENV, raising=False)
    settings = code_server_guard.HeartbeatTtlSettings.from_env()
    assert settings.ttl_seconds == 300


def test_the_heartbeat_ttl_of_this_repository_is_used(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """This repository's own setting wins over Dagster's."""
    monkeypatch.setenv(code_server_guard.HEARTBEAT_TTL_ENV, "120")
    monkeypatch.setenv(code_server_guard.DAGSTER_HEARTBEAT_TTL_ENV, "60")
    settings = code_server_guard.HeartbeatTtlSettings.from_env()
    assert settings.ttl_seconds == 120


def test_the_dagster_heartbeat_ttl_is_used_when_ours_is_unset(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """An operator who already sets Dagster's own variable keeps it."""
    monkeypatch.delenv(code_server_guard.HEARTBEAT_TTL_ENV, raising=False)
    monkeypatch.setenv(code_server_guard.DAGSTER_HEARTBEAT_TTL_ENV, "60")
    settings = code_server_guard.HeartbeatTtlSettings.from_env()
    assert settings.ttl_seconds == 60


@pytest.mark.parametrize("value", ["soon", "30s", "0", "-1", "30.5"])
def test_a_malformed_heartbeat_ttl_falls_back_to_the_default(
    monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture, value: str
) -> None:
    """A value that is not a whole number of seconds, or is below one, is
    not used: the default is, and the warning names the setting."""
    monkeypatch.setenv(code_server_guard.HEARTBEAT_TTL_ENV, value)
    with caplog.at_level(logging.WARNING):
        settings = code_server_guard.HeartbeatTtlSettings.from_env()
    assert settings.ttl_seconds == 300
    assert any(
        code_server_guard.HEARTBEAT_TTL_ENV in record.message
        for record in caplog.records
    ), [record.message for record in caplog.records]


def test_a_malformed_heartbeat_ttl_is_not_echoed_into_the_log(
    monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    """The warning names the setting, never its value: the value is the
    operator's own setting, and the log line is read by everybody."""
    monkeypatch.setenv(code_server_guard.HEARTBEAT_TTL_ENV, "soon")
    with caplog.at_level(logging.WARNING):
        code_server_guard.HeartbeatTtlSettings.from_env()
    assert not any("soon" in record.message for record in caplog.records)


# --- the runs of the previous code location (decision 1) --------------------


def test_the_guard_fails_the_runs_of_the_previous_code_location_once_after_the_first_ping() -> None:
    """The step runs ONCE, and only after the code location answered for
    the first time: that is the moment the guard knows the code location is
    up."""
    events: list[str] = []
    instance = _RecordingInstance(events)
    child = _sleeping_child()
    try:
        with pytest.raises(_StopGuard):
            code_server_guard.run_guard(
                _guard_args(_closed_port()),
                spawn=lambda **_kwargs: child,
                instance=instance,
                probe_fn=_probe_recording(events, answers=[False, True]),
                interval_seconds=0.01,
            )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert events.count("instance") == 1
    assert events.index("instance") > events.index("probe:yes")


def test_the_guard_fails_the_runs_of_the_previous_code_location_only_once_when_several_pings_answer() -> None:
    """The step runs ONCE per guard start, not once per ping: the guard
    pings the code server for as long as it lives, and the runs of the
    previous code location are ended one time only."""
    events: list[str] = []
    instance = _RecordingInstance(events)
    child = _sleeping_child()
    try:
        with pytest.raises(_StopGuard):
            code_server_guard.run_guard(
                _guard_args(_closed_port()),
                spawn=lambda **_kwargs: child,
                instance=instance,
                probe_fn=_probe_recording(events, answers=[True, True]),
                interval_seconds=0.01,
            )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert events.count("probe:yes") == 2
    assert events.count("instance") == 1


def test_the_guard_goes_on_when_the_instance_cannot_be_read(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """An instance that cannot be reached is logged and the guard keeps
    watching the code server: the run monitoring ends the runs of the
    previous code location at their run limit."""
    class BrokenInstance:
        def get_run_records(self, filters=None, **kwargs) -> list:
            raise RuntimeError("the instance is not reachable")

        def report_run_failed(self, run, message=None) -> None:
            raise AssertionError("no run may be failed")

    child = _sleeping_child()
    try:
        with caplog.at_level(logging.WARNING):
            with pytest.raises(_StopGuard):
                code_server_guard.run_guard(
                    _guard_args(_closed_port()),
                    spawn=lambda **_kwargs: child,
                    instance=BrokenInstance(),
                    probe_fn=_probe_recording([], answers=[True, True]),
                    interval_seconds=0.01,
                )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert any(
        "could not be read" in record.message for record in caplog.records
    ), [record.message for record in caplog.records]


def test_the_guard_goes_on_when_the_dagster_instance_cannot_be_opened(
    monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    """The instance is built when the step runs, not before: an instance
    that cannot be opened is logged and the guard goes on."""

    def broken() -> object:
        raise RuntimeError("DAGSTER_HOME is not set")

    monkeypatch.setattr(code_server_guard, "dagster_instance", broken)
    child = _sleeping_child()
    try:
        with caplog.at_level(logging.WARNING):
            with pytest.raises(_StopGuard):
                code_server_guard.run_guard(
                    _guard_args(_closed_port()),
                    spawn=lambda **_kwargs: child,
                    probe_fn=_probe_recording([], answers=[True, True]),
                    interval_seconds=0.01,
                )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert any(
        "could not be opened" in record.message for record in caplog.records
    ), [record.message for record in caplog.records]


# --- the shutdown window (decision 3) ---------------------------------------

# The line the INNER server writes when it misses its heartbeat
# (`dagster/_grpc/server.py`).
HEARTBEAT_LINE = "No heartbeat received in 30 seconds, shutting down"


def test_the_missed_heartbeat_line_triggers_one_reload(
    capsys: pytest.CaptureFixture,
) -> None:
    """The inner server is shutting itself down: the guard reloads the code
    location ONCE, and the in-flight runs of the old server keep running."""
    forwarded = _Forwarded(capsys)
    reloads: list[dict] = []
    child = _child_printing(_CHILD_ONE_LINE, HEARTBEAT_LINE)
    try:
        with pytest.raises(_StopGuard):
            code_server_guard.run_guard(
                _guard_args(_closed_port()),
                spawn=lambda **_kwargs: child,
                probe_fn=_probe_waiting_for(
                    forwarded, HEARTBEAT_LINE, reloads=reloads, stop_when_reloads=1
                ),
                reload_fn=lambda **kwargs: reloads.append(kwargs),
                interval_seconds=0.01,
            )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert len(reloads) == 1


@pytest.mark.parametrize(
    "line",
    [
        pytest.param(
            "Tried to start a run on a server after telling it to shut down",
            id="a-run-on-a-shutting-down-server",
        ),
        pytest.param(
            "Failed to shut down server", id="a-shutdown-that-failed"
        ),
    ],
)
def test_another_line_does_not_trigger_a_reload(
    capsys: pytest.CaptureFixture, line: str
) -> None:
    """Only the missed-heartbeat line reloads the code location: the
    `ShutdownServer` path the proxy itself uses must not start another
    reload."""
    forwarded = _Forwarded(capsys)
    reloads: list[dict] = []
    child = _child_printing(_CHILD_ONE_LINE, line)
    try:
        with pytest.raises(_StopGuard):
            code_server_guard.run_guard(
                _guard_args(_closed_port()),
                spawn=lambda **_kwargs: child,
                probe_fn=_probe_waiting_for(
                    forwarded, line, reloads=reloads, stop_when_reloads=1
                ),
                reload_fn=lambda **kwargs: reloads.append(kwargs),
                interval_seconds=0.01,
            )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert reloads == []


def test_the_second_shutdown_window_is_ten_minutes() -> None:
    """The guard gives up when the inner server shuts down again within ten
    minutes of the reload: long enough for a reload to settle, short enough
    that a code location which misbehaves again does not keep the container
    alive."""
    assert code_server_guard.SECOND_SHUTDOWN_WINDOW_SECONDS == 600
    assert code_server_guard.SECOND_SHUTDOWN_WINDOW_SECONDS == 10 * 60


def test_a_second_heartbeat_line_within_ten_minutes_makes_the_guard_exit_non_zero(
    capsys: pytest.CaptureFixture,
) -> None:
    """The reload did not help: the guard gives up and exits non-zero, so
    the container restarts -- the fallback #605 already proved."""
    forwarded = _Forwarded(capsys)
    reloads: list[dict] = []
    child = _child_printing(_CHILD_TWO_LINES, HEARTBEAT_LINE, HEARTBEAT_LINE)
    try:
        exit_code = code_server_guard.run_guard(
            _guard_args(_closed_port()),
            spawn=lambda **_kwargs: child,
            probe_fn=_probe_waiting_for(
                forwarded, HEARTBEAT_LINE, reloads=reloads, stop_when_reloads=99
            ),
            reload_fn=lambda **kwargs: reloads.append(kwargs),
            interval_seconds=0.01,
        )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert exit_code == 1
    assert len(reloads) == 1


def test_a_second_heartbeat_line_outside_the_ten_minute_window_reloads_again(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture
) -> None:
    """A second shutdown line long after the first one reloads again: the
    guard does not give up on a code location that misbehaves once."""
    monkeypatch.setattr(code_server_guard, "SECOND_SHUTDOWN_WINDOW_SECONDS", 0.05)
    forwarded = _Forwarded(capsys)
    reloads: list[dict] = []
    child = _child_printing(_CHILD_SECOND_LINE_ON_ASK, HEARTBEAT_LINE, HEARTBEAT_LINE)

    def reload_fn(**kwargs) -> None:
        reloads.append(kwargs)
        # The second line arrives well after the reload started, so it is
        # outside the window.
        time.sleep(0.2)
        child.stdin.write("\n")
        child.stdin.flush()

    try:
        with pytest.raises(_StopGuard):
            code_server_guard.run_guard(
                _guard_args(_closed_port()),
                spawn=lambda **_kwargs: child,
                probe_fn=_probe_waiting_for(
                    forwarded, HEARTBEAT_LINE, reloads=reloads, stop_when_reloads=2
                ),
                reload_fn=reload_fn,
                interval_seconds=0.01,
            )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert len(reloads) == 2


def test_the_code_servers_output_is_forwarded_to_the_guards_own_output(
    capsys: pytest.CaptureFixture,
) -> None:
    """The container log keeps showing what the code server says: the guard
    reads the child's output and writes it on, unchanged."""
    forwarded = _Forwarded(capsys)
    reloads: list[dict] = []
    child = _child_printing(
        _CHILD_TWO_STREAMS, "a line from the code server", "a warning line"
    )
    try:
        with pytest.raises(_StopGuard):
            code_server_guard.run_guard(
                _guard_args(_closed_port()),
                spawn=lambda **_kwargs: child,
                probe_fn=_probe_waiting_for(
                    forwarded,
                    "a line from the code server",
                    reloads=reloads,
                    stop_when_reloads=99,
                ),
                reload_fn=lambda **kwargs: reloads.append(kwargs),
                interval_seconds=0.01,
            )
    finally:
        child.kill()
        child.wait(timeout=10)
    assert "a line from the code server\n" in forwarded.out
    assert "a warning line\n" in forwarded.err


def test_the_matcher_finds_the_rendered_heartbeat_line() -> None:
    """The line the inner server writes, as it appears in the log."""
    assert code_server_guard.SHUTDOWN_LINE_PATTERN.search(HEARTBEAT_LINE)


def test_the_installed_dagster_source_still_has_the_line_the_matcher_expects() -> None:
    """A Dagster version bump that changes the line fails this test instead
    of silently disabling the reload."""
    import dagster

    source = Path(dagster.__file__).parent / "_grpc" / "server.py"
    assert source.is_file(), source
    text = source.read_text()
    assert (
        "No heartbeat received in {heartbeat_timeout} seconds, shutting down" in text
    ), "the installed Dagster writes another line for the missed heartbeat"

