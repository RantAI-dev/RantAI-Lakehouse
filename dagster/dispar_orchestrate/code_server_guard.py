"""A supervisor for the code server, so a DEAD inner server restarts (OPS-37).

# Why this module exists

On staging, from 2026-11-18 22:15 UTC to 2026-11-19 02:58 UTC, Dagster
stopped launching runs. The inner code server of the `dagster-code-location`
container died ("No heartbeat received in 30 seconds, shutting down") and the
`code-server start` PROXY never respawned it: the proxy starts the inner
server ONCE (`dagster/_grpc/server.py`, `GrpcServerProcess
.start_server_process`) and the server registry never restarts a dead one. The
proxy's own gRPC HEALTH service is hardcoded SERVING
(`dagster/_grpc/server.py`), so `dagster api grpc-health-check` cannot see the
dead inner server either. The container stayed "Up" while the daemon failed
1550 times, and no schedule or sensor ran until a manual restart 4.5 hours
later.

Dagster 1.13.20 offers NO option that makes the proxy exit when the inner
server dies, and Docker does not restart an UNHEALTHY container by itself:
`restart: unless-stopped` restarts a container whose PROCESS exited, not one
whose process is alive and whose server is dead. So the container's main
process has to notice and EXIT. This module is that process.

# What it does

- starts the code server as a CHILD process, with the very arguments the
  compose file passed before (`code-server start -h ... -p ... -m ...`), so
  nothing about the code location changes;
- every 30 s sends ONE `Ping` THROUGH the proxy. The proxy FORWARDS `Ping`
  to the inner server (`dagster/_grpc/proxy_server.py`), so a Ping that does
  not come back means the inner server is gone -- which is exactly what the
  hardcoded health service cannot report;
- exits NON-ZERO when the inner server is gone, so `restart: unless-stopped`
  restarts the container. Exiting is the only restart path Docker has;
- forwards SIGTERM and SIGINT to the child and exits 0, so `docker compose
  stop` stops the container cleanly. As PID 1 in a container the kernel does
  not apply the DEFAULT action of SIGTERM, so the handlers have to be
  installed here.

`--healthcheck` runs ONE probe and exits 0 or 1: the compose healthcheck
uses it, so the container's health state shows a dead inner server too.

# Invocation

`python3 -m dispar_orchestrate.code_server_guard --host 0.0.0.0 --port 4000
--module dispar_orchestrate.definitions` (the `dagster-code-location`
service's `command:` in `docker-compose.yml`).
"""

from __future__ import annotations

import argparse
import logging
import os
import signal
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from types import FrameType

logger = logging.getLogger("code_server_guard")

# How often the guard pings the code server, and how long ONE ping may take.
# The ping interval is the guard's detection latency: a dead inner server is
# noticed within one interval plus one ping timeout.
PROBE_INTERVAL_SECONDS = 30
PROBE_TIMEOUT_SECONDS = 10

# How many pings in a row mean "the inner server is dead", once the code
# server has answered at least once, and how many mean "it never came up"
# (the code location imports its definitions at start, which takes a while).
FAILURES_AFTER_FIRST_SUCCESS = 5
FAILURES_BEFORE_FIRST_SUCCESS = 10

# How long the guard waits for the code server to stop before killing it.
SHUTDOWN_TIMEOUT_SECONDS = 30

# The address the healthcheck and the guard's own ping use. The code server
# runs in the SAME container, so the loopback address reaches it; `--host` is
# the address the server BINDS to, not the one to connect to.
LOOPBACK_HOST = "127.0.0.1"

# The gRPC timeout the CONTAINER was given, if any. The guard overrides it
# in its own process (`dagster/_grpc/utils.py` reads the variable in
# `default_grpc_timeout()` and `dagster/_grpc/client.py` binds the result,
# once, at import time) and hands the child the container's value, so an
# operator who set it for the code server keeps it. `None` means "unset":
# the child then runs with Dagster's own default.
CONTAINER_GRPC_TIMEOUT_SECONDS = os.environ.get("DAGSTER_GRPC_TIMEOUT_SECONDS")


@dataclass(frozen=True)
class ProbeState:
    """What the guard knows about the code server after its probes."""

    succeeded_once: bool = False
    consecutive_failures: int = 0

    def after_probe(self, answered: bool) -> "ProbeState":
        """The state after one more probe: a success resets the streak."""
        if answered:
            return ProbeState(succeeded_once=True, consecutive_failures=0)
        return ProbeState(
            succeeded_once=self.succeeded_once,
            consecutive_failures=self.consecutive_failures + 1,
        )


def should_give_up(state: ProbeState, *, child_exited: bool) -> bool:
    """The rule that decides to give up, in one place.

    - the code server PROCESS died: the container cannot serve the code
      location at all, so the guard gives up at once;
    - the code server answered a ping before and stopped answering: five
      pings in a row (about two and a half minutes at the probe interval)
      mean the inner server is dead;
    - the code server never answered: ten pings in a row (about five
      minutes) mean it never came up, so the guard stops probing and
      exits non-zero.

    Giving up means EXITING NON-ZERO, and that exit is the restart path:
    `restart: unless-stopped` restarts the container, the guard runs again,
    and the container keeps trying to bring the code server up. The retry
    loop is INTENDED. A code server that never comes up is a deployment
    problem -- a broken image, a broken definitions import -- that the next
    deploy fixes, while the alternative, a container that stays "Up" with a
    dead code server, is exactly the staging outage this guard exists to
    prevent (the container stayed "Up" from 2026-11-18 while the daemon
    failed 1550 times). The operator sees the difference: the container's
    state is "restarting" (measured on this machine: `docker ps` shows
    "Restarting (1)"), and the code server's own error is in the
    container's log.

    Docker's restart backoff -- it grows the delay between restarts
    (measured: the gaps between restarts grew) -- keeps a never-starting
    code location from spinning at full speed, and
    that backoff is the ONLY bound: there is NO restart budget across
    container restarts, because a budget would need state that survives the
    restart (a file in a mounted volume), which this plan does not ask for
    and which would add a second failure mode.
    """
    if child_exited:
        return True
    if state.succeeded_once:
        return state.consecutive_failures >= FAILURES_AFTER_FIRST_SUCCESS
    return state.consecutive_failures >= FAILURES_BEFORE_FIRST_SUCCESS


def give_up_reason(state: ProbeState) -> str:
    """One line, no secrets: why the guard is giving up."""
    if state.succeeded_once:
        return (
            f"the code server did not answer {state.consecutive_failures} pings in a row: "
            "its inner server is dead"
        )
    return (
        f"the code server answered none of {state.consecutive_failures} pings: it never came up"
    )


def probe_host(bind_host: str) -> str:
    """The address the guard pings.

    `--host` is where the code server BINDS. The guard runs in the SAME
    container, so it reaches the server on the loopback address; a bind
    address of `0.0.0.0` (every interface) is the loopback address for this
    purpose, and a specific address is used as it is.
    """
    if bind_host in ("", "0.0.0.0", "::"):
        return LOOPBACK_HOST
    return bind_host


def probe(*, host: str, port: int) -> bool:
    """ONE `Ping` through the proxy. True when the code server answered.

    The proxy FORWARDS `Ping` to the inner server
    (`dagster/_grpc/proxy_server.py`), so a Ping that does not come back
    means the inner server is gone -- which the proxy's own health service,
    hardcoded SERVING, cannot report.
    """
    # Imported here, not at module level: the import of
    # `dagster/_grpc/client.py` reads `DAGSTER_GRPC_TIMEOUT_SECONDS` ONCE,
    # at import time -- `dagster/_grpc/utils.py` (`default_grpc_timeout()`)
    # reads the variable and the client module binds the result to
    # `DEFAULT_GRPC_TIMEOUT` -- so the value has to be in the environment
    # before this import happens.
    from dagster._grpc.client import DagsterGrpcClient

    client = DagsterGrpcClient(host=host, port=port)
    try:
        client.ping(echo="")
    except Exception:
        # `DagsterUserCodeUnreachableError` is the expected one (the inner
        # server is gone). Any other gRPC error means the same thing here:
        # the code server did not answer.
        return False
    return True


def child_command(*, host: str, port: int, module: str) -> list[str]:
    """The code server, with the arguments the compose file passed before.

    `sys.executable -m dagster` instead of the `dagster` entry point: the
    guard and the code server then run on the SAME interpreter, whichever
    one the container was started with.
    """
    return [
        sys.executable,
        "-m",
        "dagster",
        "code-server",
        "start",
        "-h",
        host,
        "-p",
        str(port),
        "-m",
        module,
    ]


def child_environment(environ: Mapping[str, str]) -> dict[str, str]:
    """The code server's environment.

    The guard sets `DAGSTER_GRPC_TIMEOUT_SECONDS` for its OWN probe (the
    gRPC client reads it once, at import time: `dagster/_grpc/utils.py`
    reads the variable in `default_grpc_timeout()`). The code server must
    not inherit that value: its own calls keep Dagster's default. The value
    the CONTAINER was given, if any, is passed on.
    """
    env = {
        key: value
        for key, value in environ.items()
        if key != "DAGSTER_GRPC_TIMEOUT_SECONDS"
    }
    if CONTAINER_GRPC_TIMEOUT_SECONDS is not None:
        env["DAGSTER_GRPC_TIMEOUT_SECONDS"] = CONTAINER_GRPC_TIMEOUT_SECONDS
    return env


def spawn_child(*, host: str, port: int, module: str) -> subprocess.Popen:
    """Starts the code server as a CHILD of the guard.

    The child gets its own session, so the guard's process group signals
    never reach it behind the guard's back: the guard forwards the signals
    itself, and knows when the child is gone.
    """
    return subprocess.Popen(
        child_command(host=host, port=port, module=module),
        env=child_environment(os.environ),
        start_new_session=True,
    )


def forward_signal(child: subprocess.Popen, signum: int) -> None:
    """Sends a signal to the code server process.

    The process may already be gone: a signal that arrives after the child
    died is not an error.
    """
    try:
        child.send_signal(signum)
    except ProcessLookupError:
        pass


class CodeServerSupervisor:
    """The guard's own state: the code server child, and the stop flag."""

    def __init__(self, child: subprocess.Popen) -> None:
        self.child = child
        self.stop = threading.Event()

    def install_signal_handlers(self) -> None:
        """SIGTERM and SIGINT reach the guard, not the code server.

        As PID 1 in a container the kernel does not apply the DEFAULT
        action of SIGTERM, so `docker compose stop` would hang until
        Docker's kill timeout: the handler has to exist. It forwards the
        signal to the code server and asks the loop to stop.
        """
        signal.signal(signal.SIGTERM, self._on_signal)
        signal.signal(signal.SIGINT, self._on_signal)

    def _on_signal(self, signum: int, _frame: FrameType | None) -> None:
        logger.info("received signal %d: stopping the code server", signum)
        self.stop.set()
        forward_signal(self.child, signum)

    def wait_interval(self, seconds: float) -> bool:
        """Waits `seconds`, in small steps, so a signal or a dead child is
        noticed at once. False when the wait was cut short."""
        deadline = time.monotonic() + seconds
        while not self.stop.is_set():
            if self.child.poll() is not None:
                return False
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return True
            self.stop.wait(min(remaining, 1.0))
        return False

    def wait_for_exit(self) -> None:
        """Waits for the code server to stop, and makes sure it did."""
        try:
            self.child.wait(timeout=SHUTDOWN_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            logger.warning(
                "the code server did not stop within %d s: killing it",
                SHUTDOWN_TIMEOUT_SECONDS,
            )
            self.child.kill()
            self.child.wait(timeout=SHUTDOWN_TIMEOUT_SECONDS)

    def stop_child(self) -> None:
        """Asks the code server to stop, then waits for it."""
        forward_signal(self.child, signal.SIGTERM)
        self.wait_for_exit()


def run_guard(
    args: argparse.Namespace,
    *,
    spawn: Callable[..., subprocess.Popen] = spawn_child,
) -> int:
    """Runs the code server under the guard. The exit code is the container's.

    0: a signal asked the guard to stop (a stopped container is not a
    failure). 1: the code server is gone, so `restart: unless-stopped`
    restarts the container.
    """
    child = spawn(host=args.host, port=args.port, module=args.module)
    supervisor = CodeServerSupervisor(child)
    previous_handlers = {
        signum: signal.getsignal(signum) for signum in (signal.SIGTERM, signal.SIGINT)
    }
    target = probe_host(args.host)
    state = ProbeState()
    try:
        supervisor.install_signal_handlers()
        while True:
            if supervisor.stop.is_set():
                supervisor.wait_for_exit()
                return 0
            if child.poll() is not None:
                logger.error(
                    "the code server process exited (code %s): the container cannot serve "
                    "the code location",
                    child.returncode,
                )
                return 1
            answered = probe(host=target, port=args.port)
            state = state.after_probe(answered)
            if not answered:
                logger.warning(
                    "the code server did not answer a ping on %s:%d (attempt %d)",
                    target,
                    args.port,
                    state.consecutive_failures,
                )
            else:
                logger.info("the code server answered a ping on %s:%d", target, args.port)
            if should_give_up(state, child_exited=child.poll() is not None):
                logger.error("%s", give_up_reason(state))
                supervisor.stop_child()
                return 1
            if not supervisor.wait_interval(PROBE_INTERVAL_SECONDS):
                if supervisor.stop.is_set():
                    supervisor.wait_for_exit()
                    return 0
                logger.error(
                    "the code server process exited (code %s): the container cannot serve "
                    "the code location",
                    child.returncode,
                )
                return 1
    finally:
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)


def run_healthcheck(*, port: int) -> int:
    """ONE probe, and the exit code the compose healthcheck reads: 0 when the
    code server answers, 1 when it does not."""
    if probe(host=LOOPBACK_HOST, port=port):
        return 0
    logger.warning("the code server did not answer a ping on %s:%d", LOOPBACK_HOST, port)
    return 1


def build_parser() -> argparse.ArgumentParser:
    """The command line the compose file passes."""
    parser = argparse.ArgumentParser(
        prog="python3 -m dispar_orchestrate.code_server_guard",
        description=(
            "Supervise the Dagster code server and exit non-zero when its inner "
            "server is dead, so the container restarts."
        ),
    )
    parser.add_argument(
        "--host",
        default="0.0.0.0",
        help="the address the code server binds (default: 0.0.0.0)",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=4000,
        help="the port the code server serves (default: 4000)",
    )
    parser.add_argument(
        "--module",
        default="dispar_orchestrate.definitions",
        help="the code location module (default: dispar_orchestrate.definitions)",
    )
    parser.add_argument(
        "--healthcheck",
        action="store_true",
        help="run ONE probe against the loopback address and exit 0 or 1",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """The container's main process. The exit code is the container's."""
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s"
    )
    args = build_parser().parse_args(argv)
    # The guard's own pings get a SHORT timeout: `dagster/_grpc/utils.py`
    # reads `DAGSTER_GRPC_TIMEOUT_SECONDS` (`default_grpc_timeout()`) and
    # `dagster/_grpc/client.py` binds the result, once, at import time, to
    # `DEFAULT_GRPC_TIMEOUT`, the default timeout of `DagsterGrpcClient
    # ._query`, which `ping` uses. A hung inner server must not hold the
    # guard for a minute per probe. The code server child does NOT inherit
    # this value (`child_environment`).
    os.environ["DAGSTER_GRPC_TIMEOUT_SECONDS"] = str(PROBE_TIMEOUT_SECONDS)
    if args.healthcheck:
        return run_healthcheck(port=args.port)
    return run_guard(args)


if __name__ == "__main__":
    sys.exit(main())
