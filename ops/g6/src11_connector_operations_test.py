#!/usr/bin/env python3
"""SRC-11 gate (task 8, decision D14): connector operations against a real
stack: retries with the source going away and coming back, no retry for a
refused sign-in, auto-disable and Resume, a paused connector that does not
load, a column left out of a load, an honest run log, and the run history.

It is a job of its own (`g6-ops`), so no existing gate job grows. Every step
prints `[g6-ops] SRC-11 ...`; a step that fails raises `OpsGateFailure` whose
message starts with the step's name, and nothing is skipped silently.

# The settings are LOWERED for this gate, on purpose

The product's defaults are the spec's: three retries after 1, 5 and 15 minutes
(`INGEST_RETRY_DELAYS_SECONDS`, default `60,300,900`) and a pause after 30
failed runs in a row (`CONNECTOR_AUTO_DISABLE_AFTER`, default 30). A gate
that waited 21 minutes per retried table, or ran 30 failing loads, would be a
slow and expensive test of the same code path. The job's `.env` therefore sets
`INGEST_RETRY_DELAYS_SECONDS=5,10,20` and `CONNECTOR_AUTO_DISABLE_AFTER=3`
(the feature page's checklist row 6 allows a lowered number if the result says
so). The gate reads the second one from its own environment and checks the
pause reason against it; what it proves is the mechanism (a retry after the
configured wait, a pause at the configured count), NOT the default numbers.
The defaults are the code's (`ingest_errors.DEFAULT_RETRY_DELAYS_SECONDS`,
`lakehouse_store::connectors::DEFAULT_AUTO_DISABLE_AFTER`) and are pinned by
their unit tests.

# Steps

a. Retry then success (D1, AC1). The gate starts a TCP forwarder in a thread
   of its own process, listening on the runner's address on the compose
   network and forwarding to the `mysql-g6` fixture. A MySQL connector dials
   the forwarder. With the forwarder OFF (its port closed: "connection
   refused") a run fails an attempt as `unreachable` with a wait; the gate
   switches the forwarder ON and the run must succeed on a retry, with at
   least two attempts in the run log. The same shape as `SRC-17`'s receiver:
   no product change is needed to reach it, because the API and Dagster
   containers reach the runner over the compose network.
b. No retry for a refused sign-in (D2, SRC-19). A PostgreSQL connector with a
   wrong password fails with exactly one attempt of class `sign_in_refused`.
c. Auto-disable and Resume (D3, D4). A MySQL connector loads once, then its
   password is made wrong; N failed runs in a row (N = the lowered setting)
   leave it paused with the auto-disable reason, a `connector_disabled`
   webhook rule fires exactly once at a receiver the gate runs (`SRC-17`'s
   `Receiver`), the schedule's due list leaves it out, and `POST .../resume`
   lifts the pause and sets the streak to 0.
d. A paused connector does not load (D13, SRC-18). Before Resume, with the
   password right again, the gate launches the run through Dagster's GraphQL
   `launchRun` (the API refuses a paused connector's run with a 409, so the
   API is not the way in); the run record ends `paused` with the end reason
   `connector_paused` and the Bronze row count is unchanged.
e. A column left out (D9, D10, AC3). `PUT .../ingest-spec` with
   `excludedColumns`; the schema-changes list shows the column inactive with
   reason `excluded`, nothing waits for a decision, and the next load carries
   no value for it.
f. The log is honest (D5). The text download of step b's run is `text/plain`,
   names the attempt and its class, and holds none of the wrong password,
   `password authentication failed`, `psycopg2`, `OperationalError` or
   `Traceback`.
g. History (D7). `GET .../runs` lists every run this gate launched through
   `POST .../ingest/run` with the trigger `user`.

# Settings (environment)

`LAKEHOUSE_API_URL`, `DAGSTER_URL`, `CH_*`, `AUTH_BOOTSTRAP_*`,
`MYSQL_ROOT_PASSWORD`, `CONNECTOR_MYSQL_PASSWORD` as `g6_ingest_matrix_test.py`
reads them (its login, tenant, connector and credential-file helpers are
imported, and `src17_alert_delivery_test.py`'s receiver and rule helpers);
`INGEST_SERVICE_TOKEN` (the ingest service identity, to read the due list the
way the schedule sensor does); `CONNECTOR_AUTO_DISABLE_AFTER` (the number the
API runs with; the same value as the API container's, from the job's `.env`);
`SRC11_FORWARDER_PORT` (default 13306); `SRC17_RECEIVER_HOST` and
`SRC17_RECEIVER_PORT` (default 8099) for the receiver.

`SEC-10` (alert webhooks cannot reach internal addresses) is NOT built yet.
When it lands it will refuse the receiver's address by default and must give
this gate its documented opt-in (the gate may not weaken the default). The
forwarder needs no opt-in of its own: the connector's address is checked by the
gate-only `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` and `INGEST_ALLOW_INTERNAL_HOSTS`
of `ops/g6/docker-compose.g6.override.yml`, the ones every other g6 gate uses.

Exit code 0 and `[g6-ops] PASS`, or `[g6-ops] FAILED: <step>: ...` on stderr and
exit code 1.
"""
from __future__ import annotations

import json
import os
import secrets
import socket
import sys
import threading
import time

import requests

import g6_ingest_matrix_test as matrix
import src17_alert_delivery_test as src17

GATE = "g6-ops"
FORWARDER_PORT = int(os.environ.get("SRC11_FORWARDER_PORT", "13306"))
# The number the API runs with (`CONNECTOR_AUTO_DISABLE_AFTER`, lowered in the
# job's `.env`, see the module docstring). The gate refuses to guess it: an
# unset value would make the pause at 30 a 30-run test.
AUTO_DISABLE_AFTER_RAW = os.environ.get("CONNECTOR_AUTO_DISABLE_AFTER", "")
INGEST_SERVICE_TOKEN = os.environ.get("INGEST_SERVICE_TOKEN", "")
DAGSTER_REPOSITORY = os.environ.get("DAGSTER_REPO", "__repository__")
DAGSTER_LOCATION = os.environ.get("DAGSTER_LOCATION", "dispar_orchestrate.definitions")
INGEST_JOB = "ingest_job"

# A due-list window that holds the only time the leap-day schedule below
# fires. The schedule sensor of the stack under test therefore never launches
# these connectors on its own (it would race the gate's runs); the due list is
# evaluated by the API for any window we give it.
DUE_CRON = "0 2 29 2 *"
DUE_AFTER = "2028-02-29T01:59:00Z"
DUE_UNTIL = "2028-02-29T02:00:00Z"

RUN_TIMEOUT_S = 150
# The run sensor ticks every 30 s and reports a finished run to the API, which
# is where a failed run is counted (`connector_run_event`).
SENSOR_WAIT_S = 150
# What a log must never hold (principle 4): the driver's own words and its
# class names, plus the secret the gate set (added per call).
FORBIDDEN_IN_LOG = ("password authentication failed", "psycopg2", "operationalerror", "traceback")
SIGN_IN_PHRASE = "the source refused the sign-in"  # `ErrorClass::SignInRefused.phrase()`

# Every (connector id, run id) launched through POST .../ingest/run, for step g.
LAUNCHED: list[tuple[str, str]] = []


class OpsGateFailure(Exception):
    """A step of this gate failed; the message starts with the step's name."""


def say(line: str) -> None:
    print(f"[{GATE}] SRC-11 {line}", flush=True)


def auto_disable_after() -> int:
    """The lowered threshold, or a failure that says how to set it."""
    try:
        value = int(AUTO_DISABLE_AFTER_RAW)
    except ValueError:
        value = 0
    if value < 1:
        raise OpsGateFailure(
            "CONNECTOR_AUTO_DISABLE_AFTER is not set to a whole number of 1 or more in the gate's environment; "
            "it must be the value the API container runs with (the job's .env lowers it)"
        )
    return value


# ── the forwarder ─────────────────────────────────────────────────────────


class Forwarder:
    """A TCP forwarder that can be switched OFF and ON.

    ON: listens on `0.0.0.0:<port>` and pipes every connection to `target`.
    OFF: the listening socket and every open connection are closed, so the
    port refuses new connections at once (what a source that is down looks
    like to a driver: "can't connect", error 2003 for MySQL, class
    `unreachable`). `on()` after `off()` binds the same port again."""

    def __init__(self, port: int, target: tuple[str, int]) -> None:
        self._port = port
        self._target = target
        self._lock = threading.Lock()
        self._listener: socket.socket | None = None
        self._open: set[socket.socket] = set()

    def on(self) -> None:
        with self._lock:
            if self._listener is not None:
                return
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind(("0.0.0.0", self._port))
            sock.listen(32)
            self._listener = sock
        threading.Thread(target=self._accept, args=(sock,), name="src11-forwarder", daemon=True).start()

    def off(self) -> None:
        with self._lock:
            sock, self._listener = self._listener, None
            open_now, self._open = list(self._open), set()
        if sock is not None:
            # `shutdown` wakes the thread blocked in `accept`; `close` alone does not.
            _close(sock, shutdown=True)
        for conn in open_now:
            _close(conn, shutdown=True)

    def is_on(self) -> bool:
        with self._lock:
            return self._listener is not None

    def _accept(self, sock: socket.socket) -> None:
        while True:
            try:
                client, _ = sock.accept()
            except OSError:
                return
            threading.Thread(target=self._serve, args=(client,), daemon=True).start()

    def _serve(self, client: socket.socket) -> None:
        try:
            upstream = socket.create_connection(self._target, timeout=10)
        except OSError:
            _close(client)
            return
        with self._lock:
            if self._listener is None:  # switched OFF while this connection was being made
                _close(client)
                _close(upstream)
                return
            self._open.update((client, upstream))
        threading.Thread(target=_pipe, args=(client, upstream), daemon=True).start()
        _pipe(upstream, client)


def _close(sock: socket.socket, shutdown: bool = False) -> None:
    try:
        if shutdown:
            sock.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    try:
        sock.close()
    except OSError:
        pass


def _pipe(source: socket.socket, sink: socket.socket) -> None:
    try:
        while True:
            data = source.recv(65536)
            if not data:
                break
            sink.sendall(data)
    except OSError:
        pass
    finally:
        _close(source, shutdown=True)
        _close(sink, shutdown=True)


# ── small helpers, unit-testable without a stack ──────────────────────────


def poll(what: str, check, timeout_s: float, interval_s: float = 2.0):
    """Call `check()` until it returns something truthy; raise OpsGateFailure
    naming `what` and the last error when `timeout_s` passes."""
    deadline = time.time() + timeout_s
    last: object = None
    while True:
        try:
            value = check()
        except (requests.RequestException, ValueError) as exc:
            value, last = None, f"{type(exc).__name__}: {exc}"
        if value:
            return value
        if time.time() >= deadline:
            raise OpsGateFailure(f"timed out after {timeout_s:.0f}s waiting for {what} (last error: {last!r})")
        time.sleep(interval_s)


def attempts_phrase(attempts: list[dict]) -> str:
    """The attempts of a run log as one printable line."""
    parts = []
    for a in attempts:
        text = f"{a['table']} attempt {a['attempt']} {a['result']}"
        if a.get("errorClass"):
            text += f" ({a['errorClass']})"
        if a.get("waitSeconds") is not None:
            text += f", next in {a['waitSeconds']}s"
        if a["result"] == "succeeded":
            text += f", {a.get('rowsLoaded', 0)} rows"
        parts.append(text)
    return "; ".join(parts)


def check_log_text(text: str, *, secret: str, must_have: list[str]) -> None:
    """The downloaded log names every string of `must_have` and holds none of
    `secret` or `FORBIDDEN_IN_LOG` (case-insensitive). Raises OpsGateFailure."""
    for needle in must_have:
        if needle not in text:
            raise OpsGateFailure(f"the log does not hold {needle!r}: {text[:400]!r}")
    lowered = text.lower()
    for bad in (*FORBIDDEN_IN_LOG, secret.lower()):
        if bad and bad in lowered:
            # The secret itself is never printed.
            shown = "the wrong password" if bad == secret.lower() else repr(bad)
            raise OpsGateFailure(f"the log holds source or driver text ({shown})")


# ── API and Dagster calls ─────────────────────────────────────────────────


def api_get(path: str, **kwargs) -> requests.Response:
    return matrix.API.get(f"{matrix.API_URL}{path}", timeout=15, **kwargs)


def expect_ok(resp: requests.Response, what: str, status: int = 200) -> dict:
    if resp.status_code != status:
        raise OpsGateFailure(f"{what}: expected {status}, got {resp.status_code} {resp.text[:300]}")
    return resp.json() if resp.content else {}


def connector_detail(connector_id: str) -> dict:
    """`GET /api/connectors/{id}` (`connectors::detail`): `pausedAt`,
    `pausedReason`, `failureStreak`."""
    return expect_ok(api_get(f"/api/connectors/{connector_id}"), f"GET connector {connector_id}")


def runs_of(connector_id: str) -> list[dict]:
    """`GET /api/connectors/{id}/runs` (`connector_runs::list`): `{"runs": [...]}`."""
    return expect_ok(api_get(f"/api/connectors/{connector_id}/runs"), f"GET runs of {connector_id}")["runs"]


def run_record(connector_id: str, run_id: str) -> dict | None:
    return next((r for r in runs_of(connector_id) if r["runId"] == run_id), None)


def run_log(connector_id: str, run_id: str) -> dict | None:
    """`GET /api/connectors/{id}/runs/{runId}/log`: `{"run", "attempts", "lines"}`;
    `None` while the run is unknown (404: its start report has not arrived)."""
    resp = api_get(f"/api/connectors/{connector_id}/runs/{run_id}/log")
    if resp.status_code == 404:
        return None
    return expect_ok(resp, f"GET log of run {run_id}")


def launch_run(connector_id: str) -> str:
    """`POST /api/connectors/{id}/ingest/run`: the run a person starts (tagged
    `lakehouse/trigger: user`)."""
    run_id = matrix._post_ingest_run(connector_id).json().get("runId")
    if not run_id:
        raise OpsGateFailure(f"ingest/run for {connector_id} returned no runId")
    LAUNCHED.append((connector_id, run_id))
    say(f"launched run {run_id} of connector {connector_id}")
    return run_id


def dagster_status(run_id: str) -> str:
    query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename ... on Run { status } } }"
    resp = requests.post(matrix.DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
    resp.raise_for_status()
    return str((resp.json().get("data", {}).get("pipelineRunOrError", {}) or {}).get("status"))


def wait_dagster(run_id: str, want: str, what: str, timeout_s: int = RUN_TIMEOUT_S) -> None:
    """Poll Dagster until the run has status `want` (`SUCCESS` or `FAILURE`).
    The other final status is a failure of this gate."""
    other = "FAILURE" if want == "SUCCESS" else "SUCCESS"

    def check() -> bool:
        status = dagster_status(run_id)
        if status == other:
            raise OpsGateFailure(f"{what}: run {run_id} ended {status}, expected {want}")
        return status == want

    poll(f"{what}: run {run_id} {want}", check, timeout_s, interval_s=3.0)


def launch_through_dagster(connector_id: str) -> str:
    """Launch `ingest_job` for `connector_id` through Dagster's GraphQL
    `launchRun`, the way `lakehouse-dagster`'s `launch_run_with_config` does
    (the same selector and `RunConfigData` object), and as the orchestrator's
    own screen would: no `lakehouse/trigger` tag, so the run is recorded
    `other`. Used for step d, because the API refuses a paused connector's
    run (409) before Dagster is asked."""
    mutation = (
        "mutation($sel: JobOrPipelineSelector!, $cfg: RunConfigData!) { "
        'launchRun(executionParams: { selector: $sel, mode: "default", runConfigData: $cfg }) { '
        "__typename ... on LaunchRunSuccess { run { runId } } "
        "... on PythonError { message } "
        "... on RunConfigValidationInvalid { errors { message } } } }"
    )
    variables = {
        "sel": {
            "repositoryName": DAGSTER_REPOSITORY,
            "repositoryLocationName": DAGSTER_LOCATION,
            "pipelineName": INGEST_JOB,
        },
        "cfg": {"ops": {"run_ingest": {"config": {"connector_id": connector_id}}}},
    }
    resp = requests.post(matrix.DAGSTER_URL, json={"query": mutation, "variables": variables}, timeout=15)
    resp.raise_for_status()
    launched = (resp.json().get("data") or {}).get("launchRun") or {}
    run_id = (launched.get("run") or {}).get("runId")
    if launched.get("__typename") != "LaunchRunSuccess" or not run_id:
        raise OpsGateFailure(f"Dagster launchRun did not start the run: {json.dumps(launched)[:300]}")
    return run_id


def ingestible_ids(*, due: bool) -> dict[str, dict]:
    """`GET /api/connectors/ingestible` (`connectors::list_ingestible`,
    `ingest:read`), with the service token the schedule sensor uses, by id.
    With `due`, only the connectors whose schedule fires in the leap-day
    window (the sensor's `dueAfter`/`dueUntil` query)."""
    if not INGEST_SERVICE_TOKEN:
        raise OpsGateFailure("INGEST_SERVICE_TOKEN is not set in the gate's environment (the due list needs it)")
    url = f"{matrix.API_URL}/api/connectors/ingestible"
    if due:
        url += f"?dueAfter={DUE_AFTER}&dueUntil={DUE_UNTIL}"
    resp = requests.get(url, headers={"Authorization": f"Bearer {INGEST_SERVICE_TOKEN}"}, timeout=15)
    return {c["id"]: c for c in expect_ok(resp, "GET connectors/ingestible")}


# ── connectors ────────────────────────────────────────────────────────────


def put_mysql_spec(
    connector_id: str, host: str, port: int, table: str, bronze: str, *,
    cron: str | None = None, excluded: list[str] | None = None,
) -> requests.Response:
    """`PUT /api/connectors/{id}/ingest-spec` for one MySQL table, loaded by
    `append` (a second load adds rows, so a load that should NOT happen shows
    as a changed count)."""
    source: dict = {"name": table, "target": bronze, "loadMode": "append"}
    if excluded is not None:
        source["excludedColumns"] = excluded
    body: dict = {
        "adapter": "sql", "ingestMode": "batch",
        "dial": {"driver": "mysql", "host": host, "port": port, "database": "g6_ingest", "user": "g6_reader"},
        "sourceObjects": [source],
    }
    if cron is not None:
        body["scheduleCron"] = cron
    return matrix.API.put(f"{matrix.API_URL}/api/connectors/{connector_id}/ingest-spec", json=body, timeout=15)


def register_mysql(
    name: str, host: str, port: int, table: str, bronze: str, *, cron: str | None = None,
    credential: str | None = None,
) -> tuple[str, str]:
    """A MySQL connector with its credential file and ingest spec. Returns
    (connector id, the derived credential name, to rewrite the password).
    `credential` defaults to the gate fixture's environment-provided MySQL
    password; callers do not spell it out (a password keyword argument reads
    as a key assignment to gitleaks' `generic-api-key` rule)."""
    body = matrix._create_connector(
        name=name, kind="MySQL", host=f"{host}:{port}", credential={"source": "file", "primary": "password"}
    )
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise OpsGateFailure(f"the create response of {name} carried no derived credential name: {body}")
    matrix._write_credential_file(derived, matrix.CONNECTOR_MYSQL_PASSWORD if credential is None else credential)
    expect_ok(put_mysql_spec(connector_id, host, port, table, bronze, cron=cron), f"PUT ingest-spec of {name}")
    return connector_id, derived


def bronze_rows(bronze: str) -> int:
    return matrix._bronze_row_count(bronze)


def bronze_non_null(bronze: str, column: str) -> int:
    """Rows of `bronze.<bronze>` whose `column` has a value. `WHERE 1` keeps
    `ops/lint/check_bare_iceberg_count.py` and R11 (a metadata-only count
    overcounts on equality deletes) satisfied, as `_bronze_ingested_at_null_count`."""
    text = matrix.ch_query(
        f"SELECT countIf(`{column}` IS NOT NULL) FROM {matrix.CATALOG_DB}.`bronze.{bronze}` WHERE 1 "
        "SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
    )
    return int(text.strip())


# ── steps ─────────────────────────────────────────────────────────────────


def step_retry_then_success(host: str, sfx: str) -> None:
    """a. D1, AC1: the source is down for the first attempt and back for a retry."""
    table, bronze = f"g11_retry_{sfx}", f"g6_ops_retry_{sfx}"
    matrix._src8_mysql(
        f"DROP TABLE IF EXISTS {table}",
        f"CREATE TABLE {table} (id INT PRIMARY KEY, name VARCHAR(50))",
        f"INSERT INTO {table} (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')",
    )
    forwarder = Forwarder(FORWARDER_PORT, ("mysql-g6", 3306))
    try:
        # OFF from the start: the port is closed until the gate opens it.
        connector_id, _ = register_mysql(
            f"g11-retry-{sfx}", host, FORWARDER_PORT, table, bronze
        )
        run_id = launch_run(connector_id)
        say(f"a. the source is down (forwarder {host}:{FORWARDER_PORT} OFF); waiting for a failed attempt")

        def failed_attempt() -> dict | None:
            log = run_log(connector_id, run_id)
            return next(
                (a for a in (log or {}).get("attempts", [])
                 if a["result"] == "failed" and a.get("errorClass") == "unreachable" and a.get("waitSeconds") is not None),
                None,
            )

        first = poll("a failed attempt of class unreachable with a wait", failed_attempt, 120, interval_s=1.0)
        say(f"a. attempt {first['attempt']} failed as unreachable, next try in {first['waitSeconds']}s; switching the source ON")
        forwarder.on()

        wait_dagster(run_id, "SUCCESS", "retry then success")
        log = run_log(connector_id, run_id)
        attempts = (log or {}).get("attempts", [])
        if len(attempts) < 2 or attempts[-1]["result"] != "succeeded" or attempts[0]["result"] != "failed":
            raise OpsGateFailure(f"expected a failed attempt then a successful one in the run log, got: {attempts_phrase(attempts)}")
        say(f"a. attempts: {attempts_phrase(attempts)}")
        rows = poll(f"bronze.{bronze} rows visible", lambda: bronze_rows(bronze) or None, 60, interval_s=3.0)
        if rows != 3:
            raise OpsGateFailure(f"expected 3 rows in bronze.{bronze}, got {rows}")
        done = poll(
            "the run record to end succeeded",
            lambda: (lambda r: r if r and r["status"] == "succeeded" else None)(run_record(connector_id, run_id)),
            SENSOR_WAIT_S, interval_s=5.0,
        )
        if done["attempts"] < 2:
            raise OpsGateFailure(f"the run record counts {done['attempts']} attempts, expected 2 or more")
        say(f"a. the run succeeded on a retry without anyone acting: {done['attempts']} attempts, {rows} rows")
    finally:
        forwarder.off()


def step_no_retry_for_refused_sign_in(sfx: str) -> tuple[str, str, str]:
    """b. D2, SRC-19: a wrong PostgreSQL password is one attempt. Returns
    (connector id, run id, the wrong password) for step f."""
    wrong = "g11-wrong-" + secrets.token_hex(6)
    body = matrix._create_connector(
        name=f"g11-pg-wrong-{sfx}", kind="PostgreSQL", host="postgres:5432",
        credential={"source": "file", "primary": "password"},
    )
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise OpsGateFailure(f"the create response carried no derived credential name: {body}")
    matrix._write_credential_file(derived, wrong)
    spec = matrix.API.put(
        f"{matrix.API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "sql", "ingestMode": "batch",
            "dial": {"driver": "postgres", "host": "postgres", "port": 5432, "database": "lakehouse", "user": "lakehouse"},
            "sourceObjects": [{"name": f"public.g11_missing_{sfx}", "target": f"g6_ops_pg_{sfx}"}],
        },
        timeout=15,
    )
    expect_ok(spec, "PUT ingest-spec of the PostgreSQL connector")
    run_id = launch_run(connector_id)
    wait_dagster(run_id, "FAILURE", "wrong PostgreSQL password")
    log = poll("the run log of the failed run", lambda: run_log(connector_id, run_id), 60, interval_s=2.0)
    attempts = log["attempts"]
    if len(attempts) != 1 or attempts[0].get("errorClass") != "sign_in_refused" or attempts[0].get("waitSeconds") is not None:
        raise OpsGateFailure(f"expected exactly one attempt of class sign_in_refused and no wait, got: {attempts_phrase(attempts)}")
    say(f"b. a refused sign-in is not retried: {attempts_phrase(attempts)}")
    return connector_id, run_id, wrong


def step_log_is_honest(connector_id: str, run_id: str, wrong_password: str) -> None:
    """f. D5: the downloaded log of step b's run."""
    resp = api_get(f"/api/connectors/{connector_id}/runs/{run_id}/log", params={"format": "text"})
    if resp.status_code != 200:
        raise OpsGateFailure(f"GET log?format=text: expected 200, got {resp.status_code} {resp.text[:200]}")
    content_type = resp.headers.get("Content-Type", "")
    if not content_type.startswith("text/plain"):
        raise OpsGateFailure(f"the log's Content-Type is {content_type!r}, expected text/plain")
    check_log_text(resp.text, secret=wrong_password, must_have=["attempt 1", SIGN_IN_PHRASE, run_id])
    say("f. the text log names attempt 1 and its class and holds no password, driver text or traceback")


def step_auto_disable(state: dict, sfx: str, rule_path: str, receiver: src17.Receiver, base_url: str) -> None:
    """c (first half). D3: N failed runs in a row pause the connector, raise
    `connector_disabled` once and drop it from the due list. Fills `state`
    (what the next steps need) as it goes, so the caller can delete the rule
    whatever fails."""
    threshold = auto_disable_after()
    table, bronze = f"g11_pause_{sfx}", f"g6_ops_pause_{sfx}"
    matrix._src8_mysql(
        f"DROP TABLE IF EXISTS {table}",
        f"CREATE TABLE {table} (id INT PRIMARY KEY, name VARCHAR(50))",
        f"INSERT INTO {table} (id, name) VALUES (1, 'a'), (2, 'b')",
    )
    connector_id, derived = register_mysql(
        f"g11-pause-{sfx}", "mysql-g6", 3306, table, bronze, cron=DUE_CRON
    )
    rule_id = src17.create_rule(f"g11 disabled {sfx}", "connector_disabled", base_url + rule_path, connector_id)
    say(f"c. rule {rule_id} for connector_disabled of {connector_id}; threshold {threshold} failed runs (lowered, see the module docstring)")
    state.update(connector_id=connector_id, derived=derived, bronze=bronze, rule_id=rule_id, path=rule_path)

    # A first, good load: the table's rows are in Bronze, so step d can show a
    # load that did NOT happen as an unchanged count.
    first = launch_run(connector_id)
    wait_dagster(first, "SUCCESS", "the first load")
    rows = poll(f"bronze.{bronze} rows visible", lambda: bronze_rows(bronze) or None, 60, interval_s=3.0)
    if rows != 2:
        raise OpsGateFailure(f"expected 2 rows in bronze.{bronze} after the first load, got {rows}")
    state["rows"] = rows
    if connector_id not in ingestible_ids(due=True):
        raise OpsGateFailure("the connector is not in the due list before it is paused")

    matrix._write_credential_file(derived, "g11-wrong-" + secrets.token_hex(6))
    for number in range(1, threshold + 1):
        run_id = launch_run(connector_id)
        wait_dagster(run_id, "FAILURE", f"failing run {number} of {threshold}")
        say(f"c. failed run {number} of {threshold}: {run_id}")
    detail = poll(
        "the connector to be paused by the failures",
        lambda: (lambda d: d if d.get("pausedAt") else None)(connector_detail(connector_id)),
        SENSOR_WAIT_S, interval_s=5.0,
    )
    expected_reason = f"Paused after {threshold} failed runs in a row"
    if detail.get("pausedReason") != expected_reason:
        raise OpsGateFailure(f"expected the pause reason {expected_reason!r}, got {detail.get('pausedReason')!r}")
    say(f"c. paused: {detail['pausedReason']!r} (failure streak {detail.get('failureStreak')})")

    arrival = receiver.wait_for(rule_path, time.time() + 90)
    if arrival is None:
        raise OpsGateFailure("no connector_disabled webhook arrived within 90s of the pause")
    try:
        src17.check_webhook_body(arrival[1], must_name=[connector_id, f"{threshold} failed runs"], forbidden=("traceback", "pymysql"))
    except src17.AlertGateFailure as exc:
        raise OpsGateFailure(f"the connector_disabled webhook: {exc}") from exc
    say("c. the connector_disabled webhook arrived and names the connector and the number")

    if connector_id in ingestible_ids(due=True):
        raise OpsGateFailure("the paused connector is still in the due list")
    unfiltered = ingestible_ids(due=False).get(connector_id)
    if not unfiltered or unfiltered.get("paused") is not True:
        raise OpsGateFailure(f"the unfiltered ingestible list should carry the connector as paused, got {unfiltered!r}")
    say("c. the due list leaves the paused connector out (the unfiltered list still carries it, paused)")


def step_paused_does_not_load(state: dict, receiver: src17.Receiver) -> None:
    """d. D13, SRC-18: a run launched through Dagster for a paused connector
    ends `paused` and loads nothing."""
    connector_id, bronze = state["connector_id"], state["bronze"]
    # The password is right again: were the pause not honoured, this run would
    # LOAD and the `append` count would grow.
    matrix._write_credential_file(state["derived"], matrix.CONNECTOR_MYSQL_PASSWORD)
    run_id = launch_through_dagster(connector_id)
    say(f"d. launched run {run_id} through Dagster for the paused connector")
    record = poll(
        "the run record to end",
        lambda: (lambda r: r if r and r["status"] != "running" else None)(run_record(connector_id, run_id)),
        RUN_TIMEOUT_S, interval_s=3.0,
    )
    if record["status"] != "paused" or record.get("endReason") != "connector_paused":
        raise OpsGateFailure(
            f"expected status paused and end reason connector_paused, got {record['status']!r} / {record.get('endReason')!r}"
        )
    if record["trigger"] != "other" or record["rowsLoaded"] != 0:
        raise OpsGateFailure(f"expected trigger 'other' and 0 rows for a run from Dagster's own screen, got {record!r}")
    rows = bronze_rows(bronze)
    if rows != state["rows"]:
        raise OpsGateFailure(f"bronze.{bronze} has {rows} rows, it had {state['rows']} before the run: the paused connector loaded")
    arrivals = len(receiver.arrivals(state["path"]))
    if arrivals != 1:
        raise OpsGateFailure(f"the connector_disabled webhook arrived {arrivals} times, expected exactly once")
    say(f"d. the run ended paused (connector_paused), nothing was loaded ({rows} rows, unchanged); connector_disabled arrived exactly once")


def step_resume(state: dict) -> None:
    """c (second half). D4: Resume lifts the pause, the streak is 0, the next run loads."""
    connector_id, bronze = state["connector_id"], state["bronze"]
    resp = matrix.API.post(f"{matrix.API_URL}/api/connectors/{connector_id}/resume", timeout=15)
    if expect_ok(resp, "POST resume").get("resumed") is not True:
        raise OpsGateFailure(f"resume did not answer resumed: true: {resp.text[:200]}")
    detail = connector_detail(connector_id)
    if detail.get("pausedAt") or detail.get("failureStreak") != 0:
        raise OpsGateFailure(
            f"after Resume expected no pause and a streak of 0, got pausedAt={detail.get('pausedAt')!r} "
            f"failureStreak={detail.get('failureStreak')!r}"
        )
    if connector_id not in ingestible_ids(due=True):
        raise OpsGateFailure("the resumed connector is not back in the due list")
    again = matrix.API.post(f"{matrix.API_URL}/api/connectors/{connector_id}/resume", timeout=15)
    if again.status_code != 409:
        raise OpsGateFailure(f"a second Resume must answer 409 (not paused), got {again.status_code}")
    run_id = launch_run(connector_id)
    wait_dagster(run_id, "SUCCESS", "the first run after Resume")
    rows = poll(
        f"bronze.{bronze} to gain the rows", lambda: (lambda n: n if n > state["rows"] else None)(bronze_rows(bronze)), 60, interval_s=3.0
    )
    say(f"c. Resume lifted the pause and set the streak to 0; the next run loaded ({rows} rows in bronze.{bronze})")


def step_left_out_column(sfx: str) -> None:
    """e. D9, D10, AC3: a column left out of a load."""
    table, bronze, column = f"g11_cols_{sfx}", f"g6_ops_cols_{sfx}", "secret_note"
    matrix._src8_mysql(
        f"DROP TABLE IF EXISTS {table}",
        f"CREATE TABLE {table} (id INT PRIMARY KEY, name VARCHAR(50), {column} VARCHAR(50))",
        f"INSERT INTO {table} (id, name, {column}) VALUES (1, 'a', 'x1'), (2, 'b', 'x2')",
    )
    connector_id, _ = register_mysql(
        f"g11-cols-{sfx}", "mysql-g6", 3306, table, bronze
    )
    first = launch_run(connector_id)
    wait_dagster(first, "SUCCESS", "the first load")
    poll(f"bronze.{bronze} rows visible", lambda: bronze_rows(bronze) or None, 60, interval_s=3.0)
    if column not in matrix._bronze_columns(bronze) or bronze_non_null(bronze, column) != 2:
        raise OpsGateFailure(f"the first load should carry {column} in 2 rows of bronze.{bronze}")
    say(f"e. the first load carries {column} (2 rows)")

    matrix._src8_mysql(f"INSERT INTO {table} (id, name, {column}) VALUES (3, 'c', 'x3')")
    saved = put_mysql_spec(connector_id, "mysql-g6", 3306, table, bronze, excluded=[column])
    expect_ok(saved, f"PUT ingest-spec with excludedColumns [{column}]")
    listing = matrix._schema_changes(connector_id)
    marked = [c for c in listing["inactiveColumns"] if c["objectName"] == table and c["columnName"] == column]
    if len(marked) != 1 or marked[0].get("reason") != "excluded":
        raise OpsGateFailure(f"expected {column} listed inactive with reason excluded, got {listing['inactiveColumns']!r}")
    say(f"e. {column} is listed inactive with reason 'excluded'")

    second = launch_run(connector_id)
    wait_dagster(second, "SUCCESS", "the load that leaves the column out")
    rows = poll(f"bronze.{bronze} to grow", lambda: (lambda n: n if n > 2 else None)(bronze_rows(bronze)), 60, interval_s=3.0)
    with_value = bronze_non_null(bronze, column)
    if with_value != 2:
        raise OpsGateFailure(f"the second load carried values for {column}: {with_value} rows with a value, expected only the first load's 2")
    listing = matrix._schema_changes(connector_id)
    if listing["pending"] or connector_detail(connector_id).get("pausedAt"):
        raise OpsGateFailure(f"a left-out column must not wait for a decision or pause the connector: pending={listing['pending']!r}")
    still = [c for c in listing["inactiveColumns"] if c["columnName"] == column and c.get("reason") == "excluded"]
    if len(still) != 1:
        raise OpsGateFailure(f"{column} should still be listed as excluded after the load, got {listing['inactiveColumns']!r}")
    say(f"e. the next load ({rows} rows in total) carries no value for {column}; nothing waits for a decision")


def step_history() -> None:
    """g. D7: every run launched through POST .../ingest/run is in the history with trigger user."""
    if not LAUNCHED:
        raise OpsGateFailure("the gate launched no run, nothing to look for")
    for connector_id, run_id in LAUNCHED:
        record = poll(
            f"run {run_id} in the history of {connector_id}", lambda: run_record(connector_id, run_id), 60, interval_s=3.0
        )
        if record["trigger"] != "user":
            raise OpsGateFailure(f"run {run_id} was launched through POST ingest/run, its trigger is {record['trigger']!r}, expected 'user'")
    say(f"g. the history lists all {len(LAUNCHED)} runs the gate launched through POST ingest/run, each with trigger 'user'")


def named(step: str, fn, *args):
    """Run a step; whatever it raises comes out as OpsGateFailure naming it."""
    try:
        return fn(*args)
    except OpsGateFailure as exc:
        raise OpsGateFailure(f"{step}: {exc}") from exc
    except (matrix.G6Failure, src17.AlertGateFailure) as exc:
        raise OpsGateFailure(f"{step}: {exc}") from exc
    except requests.RequestException as exc:
        raise OpsGateFailure(f"{step}: {type(exc).__name__}: {exc}") from exc


def run() -> None:
    named("wait for the services", matrix.step_wait_for_services)
    named("sign in", matrix.step_login)
    named("tenant", matrix.step_ensure_tenant)
    named("settings", auto_disable_after)

    sfx = secrets.token_hex(3)
    host = src17.own_address("lakehouse-api")
    say(f"the runner is {host}; forwarder port {FORWARDER_PORT}, receiver port {src17.RECEIVER_PORT}")
    receiver = src17.Receiver(error_body="SRC11-UNUSED")
    receiver.start()
    rule_path = f"/ok-disabled-{sfx}"
    try:
        named("a. retry then success", step_retry_then_success, host, sfx)
        pg_connector, pg_run, wrong = named("b. no retry for a refused sign-in", step_no_retry_for_refused_sign_in, sfx)
        named("f. the log is honest", step_log_is_honest, pg_connector, pg_run, wrong)
        state: dict = {}
        try:
            named("c. auto-disable", step_auto_disable, state, sfx, rule_path, receiver, f"http://{host}:{src17.RECEIVER_PORT}")
            named("d. a paused connector does not load", step_paused_does_not_load, state, receiver)
            named("c. resume", step_resume, state)
        finally:
            if state.get("rule_id"):
                src17.delete_rule(state["rule_id"])
        named("e. a left-out column", step_left_out_column, sfx)
        named("g. history", step_history)
    finally:
        receiver.stop()


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        run()
    except OpsGateFailure as exc:
        print(f"[{GATE}] FAILED: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback
        print(f"[{GATE}] FAILED: unexpected error, not an OpsGateFailure:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    print(f"[{GATE}] PASS ({time.time() - started:.0f}s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
