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
  before;
- the SIGTERM path: the signal reaches the code server and the guard exits
  0, so `docker compose stop` stops the container cleanly.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_code_server_guard.py -q`
"""

from __future__ import annotations

import os
import signal
import socket
import subprocess
import sys
import threading
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


def test_the_child_command_is_the_code_server_command_the_compose_file_had_before() -> None:
    command = code_server_guard.child_command(
        host="0.0.0.0", port=4000, module="dispar_orchestrate.definitions"
    )
    assert command[0] == sys.executable
    assert command[1:] == ["-m", "dagster", *PREVIOUS_COMPOSE_COMMAND[1:]]


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
