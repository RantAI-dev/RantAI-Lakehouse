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
  compose file passed before (`code-server start -h ... -p ... -m ...`), plus
  the heartbeat TTL, so nothing about the code location changes;
- every 30 s sends ONE `Ping` THROUGH the proxy. The proxy FORWARDS `Ping`
  to the inner server (`dagster/_grpc/proxy_server.py`), so a Ping that does
  not come back means the inner server is gone -- which is exactly what the
  hardcoded health service cannot report;
- reads the code server's output and forwards it, unchanged, so the container
  log keeps showing what the code server says. A byte the encoding cannot
  show is rendered escaped, and the reader never stops while the code server
  lives: a pipe nobody reads fills up and blocks every run;
- when the inner server says it is shutting down because it missed its
  heartbeat, it asks the proxy to RELOAD the code location: a new inner
  server takes the new runs, and the old one finishes the runs it still
  has. The guard does not exit for that, so the in-flight runs survive;
- ends the runs the PREVIOUS code location left behind, once, when the code
  location answers for the first time: a run that was STARTED when the
  guard started ran in a process tree that died with the previous container
  (`orphan_runs.py`);
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
import errno
import logging
import os
import queue
import re
import signal
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from types import FrameType
from typing import Any

from dispar_orchestrate import orphan_runs

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

# How long the inner server waits for the proxy's heartbeat before it shuts
# itself down, in seconds. Dagster's own default is 30 s
# (`dagster/_cli/code_server.py`), and a busy container misses that easily:
# that is how the inner server died on 2026-11-18, and every run dequeued in
# the shutdown window failed at once. Five minutes is long enough for a
# loaded container, and short enough that an orphaned inner server (one whose
# proxy is gone) ends by itself.
DEFAULT_HEARTBEAT_TTL_SECONDS = 300

# This repository's own setting for that TTL, and Dagster's own variable
# for the same thing (`dagster/_cli/code_server.py`). Ours wins; an operator
# who already sets Dagster's keeps it.
HEARTBEAT_TTL_ENV = "DAGSTER_CODE_SERVER_HEARTBEAT_TTL_SECONDS"
DAGSTER_HEARTBEAT_TTL_ENV = "DAGSTER_GRPC_PROXY_HEARTBEAT_TTL_SECONDS"

# The line the INNER server writes when it misses its heartbeat
# (`dagster/_grpc/server.py`). The PROXY writes the IDENTICAL line when it
# is started with `--heartbeat` (`dagster/_grpc/proxy_server.py`), so the
# child's command must never carry `--heartbeat` -- it does not, and
# `test_the_child_command_does_not_carry_the_heartbeat_flag` keeps it that
# way: with the flag, every reload would be triggered by the proxy's own
# heartbeat instead of the inner server's.
# PR #636 review 1: the run workers share these streams. Match the entire
# record written by the CLI's dagster.code_server logger, using Dagster's
# default timestamp, name and level format (_utils/log.py). Strip the SGR
# sequences its colored formatter adds before matching.
LOG_COLOR_PATTERN = re.compile(r"\x1b\[[0-9;]*m")
SHUTDOWN_LINE_PATTERN = re.compile(
    r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} [+-][0-9]{4}"
    r" - dagster\.code_server - WARNING - "
    r"No heartbeat received in [0-9]+ seconds, shutting down"
)

# How long the guard waits for the code server to RELOAD, in seconds. The
# proxy starts a new inner server and waits for it, and the new server
# imports the whole code location, which takes tens of seconds: the guard's
# own probe timeout (10 s) is far too short for that.
RELOAD_TIMEOUT_SECONDS = 120

# How long a reload protects the code location from a second shutdown. A
# second missed-heartbeat line within this window means the reload did not
# help, and the guard gives up: exiting non-zero hands the container to
# `restart: unless-stopped`, which is the fallback #605 already proved.
SECOND_SHUTDOWN_WINDOW_SECONDS = 600

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

# How the child's pipes are opened, and the ONE place that says so: the
# guard's child and the test's child both use it, so a test reads a child's
# output exactly the way the guard does.
#
# `errors="backslashreplace"` is the fix for PR #636 review 1. With the
# default `errors="strict"` the FIRST byte the encoding cannot show raises
# `UnicodeDecodeError` in the reader thread -- and `UnicodeDecodeError` IS
# a `ValueError`, so the reader caught it and stopped for good. Nobody read
# the pipe after that, it filled up (64 KiB), and every process that writes
# to it blocked: the proxy, the inner server and every run the location
# starts (a run's process inherits these streams). That is the hang OPS-37
# exists to remove. `backslashreplace` turns every byte sequence into a
# line and shows the undecodable byte escaped (`\xff`), so the reader keeps
# going.
CHILD_PIPE_OPTIONS: dict[str, Any] = {
    "text": True,
    "errors": "backslashreplace",
    "bufsize": 1,
}

# How many times the reader tries the text layer again after a failure
# before it copies the raw bytes instead, and how long it waits between two
# tries. The wait keeps a stream that fails on every line from spinning the
# reader thread (PR #636 review 1).
READ_FAILURES_BEFORE_RAW_COPY = 3
READ_RETRY_BACKOFF_SECONDS = 0.05
READ_RETRY_BACKOFF_MAX_SECONDS = 1.0

# How many raw bytes the reader copies at a time when the text layer cannot
# read the stream at all.
RAW_COPY_CHUNK_BYTES = 8192


def _env(name: str, default: str) -> str:
    """An environment variable, stripped; an empty one counts as unset."""
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class HeartbeatTtlSettings:
    """How long the inner server waits for the proxy's heartbeat.

    `from_env` reads this repository's own setting first and Dagster's own
    variable second, so an operator who already sets Dagster's keeps it.
    A malformed value is not used: the default is, and the warning names the
    setting, never its value (`ingest_factory.py::RunLimitSettings.from_env`
    reads a setting the same way).
    """

    ttl_seconds: int = DEFAULT_HEARTBEAT_TTL_SECONDS

    @classmethod
    def from_env(cls) -> "HeartbeatTtlSettings":
        for env_name in (HEARTBEAT_TTL_ENV, DAGSTER_HEARTBEAT_TTL_ENV):
            raw = _env(env_name, "")
            if not raw:
                continue
            # ASCII digits only, and at least one second: `int()` alone
            # would take "30.5" and "1_000", which the code server's own
            # option refuses.
            seconds = int(raw) if raw.isascii() and raw.isdigit() else 0
            if seconds < 1:
                # The value is the operator's own setting, but a malformed
                # one is not worth echoing into a log line the whole team
                # reads.
                logger.warning(
                    "%s cannot be used (expected a whole number of seconds, 1 or more); "
                    "using the default %d",
                    env_name,
                    DEFAULT_HEARTBEAT_TTL_SECONDS,
                )
                return cls(ttl_seconds=DEFAULT_HEARTBEAT_TTL_SECONDS)
            return cls(ttl_seconds=seconds)
        return cls(ttl_seconds=DEFAULT_HEARTBEAT_TTL_SECONDS)


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


def child_command(
    *, host: str, port: int, module: str, heartbeat_ttl_seconds: int
) -> list[str]:
    """The code server, with the arguments the compose file passed before.

    `sys.executable -m dagster` instead of the `dagster` entry point: the
    guard and the code server then run on the SAME interpreter, whichever
    one the container was started with.

    `--heartbeat-ttl` is how long the INNER server waits for the proxy's
    heartbeat before it shuts itself down
    (`dagster/_core/remote_representation/grpc_server_registry.py`). The
    command must NOT carry `--heartbeat`: that flag makes the PROXY write
    the same missed-heartbeat line the guard matches on, and every reload
    would then be triggered by the proxy's own heartbeat.
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
        "--heartbeat-ttl",
        str(heartbeat_ttl_seconds),
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


def spawn_child(
    *, host: str, port: int, module: str, heartbeat_ttl_seconds: int
) -> subprocess.Popen:
    """Starts the code server as a CHILD of the guard.

    The child gets its own session, so the guard's process group signals
    never reach it behind the guard's back: the guard forwards the signals
    itself, and knows when the child is gone.

    Its output goes to PIPES, and the guard reads them: a pipe nobody reads
    fills up and blocks the code server. The reader threads forward every
    line to the guard's own output, so the container log keeps showing what
    the code server says. The pipes are opened with
    `CHILD_PIPE_OPTIONS`, whose decoder never raises: one undecodable byte
    must not stop the reader (PR #636 review 1).
    """
    return subprocess.Popen(
        child_command(
            host=host,
            port=port,
            module=module,
            heartbeat_ttl_seconds=heartbeat_ttl_seconds,
        ),
        env=child_environment(os.environ),
        start_new_session=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        **CHILD_PIPE_OPTIONS,
    )


class ShutdownWatcher:
    """Notices the line the inner server writes when it misses its heartbeat.

    The line comes from the INNER server (`dagster/_grpc/server.py`), whose
    output the PROXY inherits, so it reaches the guard's child.

    The reader thread only QUEUES the line: the reload runs on the guard's
    main loop, because a gRPC call that blocks on that thread would stall
    the child's output pipe.
    """

    def __init__(self) -> None:
        self.lines: queue.Queue[tuple[float, str]] = queue.Queue()
        self.wake = threading.Event()

    def notice(self, line: str) -> None:
        """Queues a line that matches the missed-heartbeat line."""
        if SHUTDOWN_LINE_PATTERN.fullmatch(LOG_COLOR_PATTERN.sub("", line.rstrip("\r\n"))):
            self.lines.put((time.monotonic(), line))
            self.wake.set()

    def drain(self) -> list[tuple[float, str]]:
        """The missed-heartbeat lines seen since the last drain, oldest
        first, each with the moment it was read."""
        found: list[tuple[float, str]] = []
        while True:
            try:
                found.append(self.lines.get_nowait())
            except queue.Empty:
                return found


class _Forwarder:
    """Writes the code server's lines on to the guard's own output.

    A write that fails does NOT end the reader: the pipe the code server
    writes to must keep being drained, or the code server blocks on a full
    pipe (PR #636 review 1). The lines are noticed by the watcher either
    way, so a missed-heartbeat line is seen even when the guard's own
    output is gone.
    """

    def __init__(self, target, watcher: ShutdownWatcher) -> None:
        self._target = target
        self._watcher = watcher
        self._writing = True

    def forward(self, line: str) -> None:
        """Notices the line, then writes it on, unchanged.

        The line is noticed BEFORE it is forwarded: a test that has seen
        the line knows the watcher has it, and the guard's next round acts
        on it.
        """
        self._watcher.notice(line)
        if not self._writing:
            return
        try:
            self._target.write(line)
            self._target.flush()
        except (OSError, ValueError):
            self._writing = False
            logger.warning(
                "the guard's own output could not be written to: the code server's "
                "output keeps being read, but it is not forwarded any more"
            )


def _the_pipe_is_closed(error: BaseException) -> bool:
    """True when the pipe itself is closed: there is nothing left to read.

    `UnicodeDecodeError` is a `ValueError`, so the type alone does not tell
    "the pipe closed" from "the bytes could not be decoded": the message
    does.
    """
    if isinstance(error, UnicodeDecodeError):
        return False
    if isinstance(error, ValueError):
        return "closed file" in str(error)
    if isinstance(error, OSError):
        return error.errno in (errno.EBADF, errno.EPIPE)
    return False


def _copy_raw_bytes(stream, *, forwarder: _Forwarder) -> bool:
    """Copies the raw bytes of a stream the text layer cannot read.

    The bytes are decoded with `backslashreplace`, so an undecodable byte
    is shown escaped instead of raising again, and the lines are noticed
    by the watcher as they come.

    False when the stream has no raw side (a test double), so the caller
    goes on with the text layer. True when the copy is DONE: the raw side
    reached its end, or the pipe closed.

    A failure that is NOT the pipe closing does NOT end the copy (PR #636
    review 1): the reader has to keep draining the pipe while the code
    server lives, or the pipe fills up (64 KiB) and blocks every process
    that writes to it -- the proxy, the inner server and every run. The
    read is tried again after a short wait, so a raw side that fails on
    every read does not spin the reader thread.
    """
    raw = getattr(stream, "buffer", None)
    if raw is None:
        return False
    # `read1` gives back what is there without waiting for a full buffer:
    # the reader must forward a line the moment it arrives. It is looked up
    # WITHOUT naming `read` as the default, because a raw side that offers
    # only `read1` must not raise while the fallback is set up.
    read = getattr(raw, "read1", None) or raw.read
    pending = b""
    failures = 0
    reported = False
    while True:
        try:
            chunk = read(RAW_COPY_CHUNK_BYTES)
        except Exception as error:
            if _the_pipe_is_closed(error):
                # The pipe closed while the raw bytes were copied: there is
                # nothing left to read, so the reader is done with it.
                return True
            failures += 1
            if not reported:
                reported = True
                logger.warning(
                    "the code server's raw output could not be read (%s): the reader "
                    "keeps draining it, so the code server does not block on a full "
                    "pipe",
                    type(error).__name__,
                )
            # The raw side failed again right away: wait a little, so it
            # does not spin this thread.
            time.sleep(
                min(
                    READ_RETRY_BACKOFF_SECONDS * failures,
                    READ_RETRY_BACKOFF_MAX_SECONDS,
                )
            )
            continue
        if not chunk:
            # The raw side reached its end: the child closed its side.
            break
        failures = 0
        pending += chunk
        *lines, pending = pending.split(b"\n")
        for raw_line in lines:
            forwarder.forward(raw_line.decode("utf-8", errors="backslashreplace") + "\n")
    if pending:
        forwarder.forward(pending.decode("utf-8", errors="backslashreplace"))
    return True


def _forward_lines(stream, *, is_stderr: bool, watcher: ShutdownWatcher) -> None:
    """One line at a time: notice it, then write it on, unchanged.

    The reader must NOT stop while the child lives (PR #636 review 1): a
    pipe nobody reads fills up (64 KiB) and then blocks every process that
    writes to it -- the proxy, the inner server and every run the location
    starts. One undecodable byte used to end this thread for good: the
    reader caught `ValueError` (which `UnicodeDecodeError` is) and
    returned, and the hang that followed is the one OPS-37 exists to
    remove.

    So the pipes are opened with `errors="backslashreplace"`
    (`CHILD_PIPE_OPTIONS`), which turns every byte sequence into a line,
    and a failure that is not the pipe closing is logged ONCE and the
    reader keeps draining. When the text layer cannot read the stream at
    all, the raw bytes are copied through instead. The thread ends only
    when the stream ends.
    """
    forwarder = _Forwarder(sys.stderr if is_stderr else sys.stdout, watcher)
    reported = False
    raw_copy_reported = False
    failures = 0
    while True:
        try:
            for line in stream:
                failures = 0
                forwarder.forward(line)
            # The stream ended: the child closed its side.
            return
        except Exception as error:
            if _the_pipe_is_closed(error):
                return
            if not reported:
                reported = True
                logger.warning(
                    "the code server's output could not be read (%s): the reader keeps "
                    "draining it, so the code server does not block on a full pipe",
                    type(error).__name__,
                )
            failures += 1
            # Decoding is impossible for this stream: the raw bytes are
            # copied through instead, so the pipe keeps being drained.
            if (
                isinstance(error, UnicodeDecodeError)
                or failures >= READ_FAILURES_BEFORE_RAW_COPY
            ):
                try:
                    copied = _copy_raw_bytes(stream, forwarder=forwarder)
                except Exception as raw_error:
                    # PR #636 review 1: the failure is recorded; a silent
                    # retry every second would leave nothing in the log.
                    if not raw_copy_reported:
                        raw_copy_reported = True
                        logger.warning(
                            "the code server's raw output could not be copied (%s): "
                            "the reader goes on with the text layer",
                            type(raw_error).__name__,
                        )
                    copied = False
                if copied:
                    # The raw side reached its end, or the pipe closed:
                    # there is nothing left to read.
                    return
            # The text layer failed again right away: wait a little, so a
            # stream that fails on every line does not spin this thread.
            time.sleep(
                min(
                    READ_RETRY_BACKOFF_SECONDS * failures,
                    READ_RETRY_BACKOFF_MAX_SECONDS,
                )
            )


def start_output_readers(
    child: subprocess.Popen, watcher: ShutdownWatcher
) -> list[threading.Thread]:
    """Reads the code server's output and forwards it, line by line.

    The threads are daemons: the guard exits without waiting for them.
    They are returned so that a test can wait for the output to be read.
    """
    threads: list[threading.Thread] = []
    for stream, is_stderr in ((child.stdout, False), (child.stderr, True)):
        if stream is None:
            # A child built without pipes (a test builds one by hand):
            # there is nothing to read.
            continue
        thread = threading.Thread(
            target=_forward_lines,
            args=(stream,),
            kwargs={"is_stderr": is_stderr, "watcher": watcher},
            name="code-server-output",
            daemon=True,
        )
        thread.start()
        threads.append(thread)
    return threads


@dataclass(frozen=True)
class ReloadState:
    """What the guard knows about the reloads it asked for."""

    reloads: int = 0
    last_reload_at: float | None = None

    def after_reload(self, at: float) -> "ReloadState":
        """The state after one more reload, started at `at`."""
        return ReloadState(reloads=self.reloads + 1, last_reload_at=at)


def should_give_up_after_shutdown(state: ReloadState, *, observed_at: float) -> bool:
    """True when a second missed-heartbeat line arrived within the window.

    The first reload starts a new inner server. A second shutdown line right
    after it means the reload did not help, and the guard stops trying:
    exiting non-zero hands the container to `restart: unless-stopped`, which
    is the fallback #605 already proved.
    """
    if state.last_reload_at is None:
        return False
    return observed_at - state.last_reload_at <= SECOND_SHUTDOWN_WINDOW_SECONDS


def reload_code(*, host: str, port: int) -> None:
    """Asks the proxy to RELOAD the code location.

    `ReloadCode` on the proxy (`dagster/_grpc/proxy_server.py`) starts a new
    inner server and tells the old one to shut down; the old one keeps
    running its in-flight runs to the end, and the new runs go to the new
    server. That is the whole point: the shutdown window is left, and the
    runs that were in flight are not killed.
    """
    from dagster._grpc.client import DagsterGrpcClient

    client = DagsterGrpcClient(host=host, port=port)
    client.reload_code(timeout=RELOAD_TIMEOUT_SECONDS)


def dagster_instance():
    """The Dagster instance of this container.

    `DAGSTER_HOME` points at the instance's config (`dagster/Dockerfile`),
    and that config points the storage at the same Postgres the daemon uses
    (`dagster/dagster.yaml`), so this is the SAME instance the daemon sees.
    """
    from dagster import DagsterInstance

    return DagsterInstance.get()


def fail_orphaned_runs_at_start(
    args: argparse.Namespace,
    *,
    instance_factory: Callable[[], Any],
    guard_started_at: float,
) -> None:
    """Ends the runs the previous code location left behind, ONCE.

    Every run that was STARTED when the guard started ran in a process tree
    that died with the previous container, so it is failed here instead of
    waiting for the daemon's run monitoring to reach `dagster/max_runtime`
    (three hours for `bronze_maintenance_job`).

    The instance is built when this runs, not before: the guard's own start
    must not depend on the database. An instance that cannot be reached, or
    opened, is logged and the guard goes on.
    """
    try:
        the_instance = instance_factory()
    except Exception as error:
        logger.warning(
            "the Dagster instance could not be opened (%s): the runs of the previous code "
            "location are not ended now (the run monitoring ends them when their run limit "
            "runs out)",
            type(error).__name__,
        )
        return
    # The code location's name is `dispar_orchestrate.definitions`
    # (`dagster/workspace.yaml`, `location_name:`), and the guard is
    # started with the SAME string as the code location's module
    # (`docker-compose.yml`, `--module`). A run names its location with the
    # name the workspace gives it, so the module name is what a run of THIS
    # location carries. If an operator renames one of the two, the runs of
    # this location are no longer recognized: the step finds nothing and
    # the zombies stay.
    failed = orphan_runs.fail_orphaned_runs(
        the_instance,
        location_name=args.module,
        guard_started_at=guard_started_at,
        now=time.time(),
    )
    logger.info(
        "failed %d run(s) that the previous code location left behind", failed
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

    def wait_interval(self, seconds: float, *, wake: threading.Event | None = None) -> bool:
        """Waits `seconds`, in small steps, so a signal or a dead child is
        noticed at once. False when the wait was cut short.

        `wake` ends the wait early: the reader thread sets it when the code
        server's output carries a missed-heartbeat line, and the reload that
        follows must not wait for the whole probe interval.
        """
        deadline = time.monotonic() + seconds
        while not self.stop.is_set():
            if self.child.poll() is not None:
                return False
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return True
            step = min(remaining, 1.0)
            if wake is not None:
                if wake.wait(step):
                    wake.clear()
                    return True
            else:
                self.stop.wait(step)
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
    instance_factory: Callable[[], Any],
    spawn: Callable[..., subprocess.Popen] = spawn_child,
    probe_fn: Callable[..., bool] = probe,
    reload_fn: Callable[..., None] = reload_code,
    interval_seconds: float = PROBE_INTERVAL_SECONDS,
) -> int:
    """Runs the code server under the guard. The exit code is the container's.

    0: a signal asked the guard to stop (a stopped container is not a
    failure). 1: the code server is gone, so `restart: unless-stopped`
    restarts the container, or the inner server shut down twice in a row
    (the reload did not help).

    PR #636 review 2: the caller must supply the instance factory so a test
    with a fake probe cannot silently open the deployment's storage. The
    factory runs only after the first successful probe.
    """
    guard_started_at = time.time()
    ttl = HeartbeatTtlSettings.from_env()
    logger.info(
        "the code server's inner heartbeat TTL is %d s", ttl.ttl_seconds
    )
    child = spawn(
        host=args.host,
        port=args.port,
        module=args.module,
        heartbeat_ttl_seconds=ttl.ttl_seconds,
    )
    supervisor = CodeServerSupervisor(child)
    watcher = ShutdownWatcher()
    start_output_readers(child, watcher)
    previous_handlers = {
        signum: signal.getsignal(signum) for signum in (signal.SIGTERM, signal.SIGINT)
    }
    target = probe_host(args.host)
    state = ProbeState()
    reload_state = ReloadState()
    orphan_checked = False
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
            for observed_at, line in watcher.drain():
                if should_give_up_after_shutdown(reload_state, observed_at=observed_at):
                    logger.error(
                        "the inner code server shut down again within %d s of the last reload: "
                        "giving up so the container restarts",
                        SECOND_SHUTDOWN_WINDOW_SECONDS,
                    )
                    supervisor.stop_child()
                    return 1
                reload_state = reload_state.after_reload(time.monotonic())
                logger.warning(
                    "the inner code server missed its heartbeat and is shutting down: "
                    "reloading the code location"
                )
                try:
                    reload_fn(host=target, port=args.port)
                except Exception as error:
                    logger.warning(
                        "the reload of the code location failed (%s): the guard keeps running",
                        type(error).__name__,
                    )
            answered = probe_fn(host=target, port=args.port)
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
                if not orphan_checked:
                    # The code location is up: now, and only now, the runs
                    # the PREVIOUS code location left behind are ended.
                    orphan_checked = True
                    fail_orphaned_runs_at_start(
                        args, instance_factory=instance_factory, guard_started_at=guard_started_at
                    )
            if should_give_up(state, child_exited=child.poll() is not None):
                logger.error("%s", give_up_reason(state))
                supervisor.stop_child()
                return 1
            if not supervisor.wait_interval(interval_seconds, wake=watcher.wake):
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
    return run_guard(args, instance_factory=dagster_instance)


if __name__ == "__main__":
    sys.exit(main())
