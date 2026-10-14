#!/usr/bin/env python3
"""SRC-17 gate (finding F4, the delivery review of 2026-10-12): an alert for a
failed load reaches its webhook within 5 minutes of the run's end, and a
webhook that fails is listed as not delivered, with a fixed reason and none of
the receiver's text.

SRC-7 promised a webhook "within 5 minutes" of a failed run. Until now only
the API's route tests, with the orchestrator and the receiver faked, said it
could happen; no gate ran a real Dagster run, the real sensor
(`pipeline_run_failed_sensor`) and the real API delivery against a real
receiver. This one does, in a job of its own (`g6-alerts`), so no existing gate
job grows.

# Steps

1. `connector failure -> webhook`: a webhook rule for failed runs of one
   connector, a MySQL connector whose password is wrong (the `mysql-g6`
   fixture, a refused credential fails the run at once, in about 15 s,
   `ingest_factory._is_authentication_refusal`), and a run. The gate receives
   the webhook and prints the seconds from the run's end (Dagster's `endTime`,
   the very value the API uses as the run's end, `load_alerts::run_end`) to the
   webhook's arrival. It fails above 300 s. The body must name the connector
   and the run, and carry no source or driver error text.
2. `webhook answers 500 -> not delivered`: a second rule for the same
   connector whose target answers 500 with a body that holds a marker string.
   After the same run, `GET /api/overview/alerts` (the console's Alerts list)
   shows that alert with `delivered: false` and the fixed reason "the webhook
   server answered with an error", and the response holds none of the
   receiver's body.
3. `upload failure -> webhook`: an `upload_failure` rule, an upload whose load
   fails honestly (a CSV with a header and no row below it: the load job ends
   with "The file has no rows below the header row.", not retried), and the
   same arrival check and bound.

E-mail is not measured here, on purpose: the sender has no mode without
verified TLS, so it cannot reach a plain SMTP sink without a product change
(`docs/superpowers/plans/2026-10-13-src-17-alert-delivery.md`, D-d).

# The receiver

A listener the gate starts in a thread of its own process, bound on all
addresses of the runner container. The webhook URL names the runner's address
on the compose network, which the API container reaches directly (found with
a UDP "connect" towards the API, which sends nothing; `SRC17_RECEIVER_HOST`
overrides it). `docker compose run` does not give the container its service
name as a network alias unless `--use-aliases` is passed, so the gate does not
rely on the name.

`SEC-10` (alert webhooks cannot reach internal addresses) is built: the API
refuses this receiver's address by default. The gate's documented opt-in is
`WEBHOOK_ALLOWED_CIDRS` on `lakehouse-api`, set in
`ops/g6/docker-compose.g6.override.yml` to the pinned gate subnet only (the
runner's receiver lives inside it); the default stays closed and the gate does
not weaken it.

# Settings (environment)

`LAKEHOUSE_API_URL`, `DAGSTER_URL`, `CH_*`, `AUTH_BOOTSTRAP_*` as
`g6_ingest_matrix_test.py` reads them (its login, tenant, registration and
launch helpers are imported, as `g6_oracle_test.py` does); `SRC17_RECEIVER_HOST`
and `SRC17_RECEIVER_PORT` (default 8099) for the receiver.

Exit code 0 and `[g6-alerts] PASS`, or `[g6-alerts] FAILED: <step>: ...` on
stderr and exit code 1. The administrator who creates the rules is the
bootstrap account, as in the other gates: a rule for all connectors or for
uploads needs an administrator who sees every tenant, and a rule that names one
connector needs `alert:write` in the connector's tenant.
"""
from __future__ import annotations

import json
import os
import secrets
import socket
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import requests

import g6_ingest_matrix_test as matrix

GATE = "g6-alerts"
# The run's end to the webhook's arrival: the SRC-7 promise.
WITHIN_SECONDS = 300
RECEIVER_PORT = int(os.environ.get("SRC17_RECEIVER_PORT", "8099"))
# `lakehouse_notify::DeliveryFailure::WebhookServerError`, the fixed sentence
# the Alerts list shows for a receiver that answers an error.
SERVER_ERROR_REASON = "the webhook server answered with an error"
# Source and driver text a webhook body must never carry (principle 4, D8 of
# SRC-7): what pymysql and the server say about a refused credential, and the
# driver's own name.
FORBIDDEN_IN_BODY = ("access denied", "pymysql", "operationalerror", "traceback", "mysql", "using password")
RUN_FAILURE_TIMEOUT_S = 120


class AlertGateFailure(Exception):
    """A step of this gate failed; the message starts with the step's name.
    When it quotes an API reference id (SEC-11 hides upstream error text
    behind one), the note from `matrix.reference_hint` names the log lines to
    read (PR #105 CI fix)."""

    def __init__(self, message: object = "") -> None:
        text = str(message)
        super().__init__(text + matrix.reference_hint(text))


def say(line: str) -> None:
    print(f"[{GATE}] SRC-17 {line}", flush=True)


# ── the receiver ──────────────────────────────────────────────────────────


class Receiver:
    """Records every POST by path. A path that starts with `/err` answers 500
    with `error_body`, anything else 200."""

    def __init__(self, error_body: str) -> None:
        self.error_body = error_body
        self._lock = threading.Lock()
        self._arrivals: dict[str, list[tuple[float, str]]] = {}
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802 -- the name http.server dispatches on
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length).decode("utf-8", errors="replace")
                outer._record(self.path, body)
                if self.path.startswith("/err"):
                    payload = outer.error_body.encode("utf-8")
                    self.send_response(500)
                else:
                    payload = b"ok"
                    self.send_response(200)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, *args) -> None:  # noqa: D401 -- silence http.server's stderr access log
                return

        self._server = ThreadingHTTPServer(("0.0.0.0", RECEIVER_PORT), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, name="src17-receiver", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()

    def _record(self, path: str, body: str) -> None:
        with self._lock:
            self._arrivals.setdefault(path, []).append((time.time(), body))

    def arrivals(self, path: str) -> list[tuple[float, str]]:
        with self._lock:
            return list(self._arrivals.get(path, []))

    def wait_for(self, path: str, deadline: float) -> tuple[float, str] | None:
        """The first arrival at `path`, or None when `deadline` (epoch seconds)
        passes without one."""
        while True:
            got = self.arrivals(path)
            if got:
                return got[0]
            if time.time() >= deadline:
                return None
            time.sleep(0.5)


def own_address(towards_host: str) -> str:
    """The runner's address on the network it shares with `towards_host`.
    A UDP socket that is "connected" sends nothing; it only makes the kernel
    pick the source address of the route."""
    override = os.environ.get("SRC17_RECEIVER_HOST", "")
    if override:
        return override
    target = socket.gethostbyname(towards_host)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.connect((target, 9))
        return sock.getsockname()[0]


# ── pure checks, unit-testable without a stack ────────────────────────────


def check_webhook_body(raw: str, *, must_name: list[str], forbidden: tuple[str, ...] = FORBIDDEN_IN_BODY) -> None:
    """The body names every string of `must_name` and holds none of `forbidden`
    (case-insensitive). Raises AlertGateFailure."""
    try:
        body = json.loads(raw)
    except ValueError as exc:
        raise AlertGateFailure(f"the webhook body is not JSON: {raw[:200]!r}") from exc
    message = str(body.get("message", "")) + " " + str(body.get("title", ""))
    for needle in must_name:
        if needle not in message:
            raise AlertGateFailure(f"the webhook body does not name {needle!r}: {raw[:300]!r}")
    lowered = raw.lower()
    for bad in forbidden:
        if bad in lowered:
            raise AlertGateFailure(f"the webhook body carries source or driver text ({bad!r}): {raw[:300]!r}")


def seconds_after(ended_at: float, arrived_at: float) -> float:
    return arrived_at - ended_at


def instance_of_rule(alerts: list[dict], rule_id: str) -> dict | None:
    return next((a for a in alerts if a.get("ruleId") == rule_id), None)


# ── API and Dagster calls ─────────────────────────────────────────────────


def create_rule(name: str, kind: str, target: str, connector: str | None) -> str:
    """`POST /api/alerts` (`alerts::create`): `{"ok": true, "rule": {"id": ...}}`.
    Needs `alert:write`; a rule for uploads or for all connectors also needs an
    administrator who sees every tenant (`authorise_rule_scope`)."""
    body = {"name": name, "type": kind, "channel": "webhook", "target": target, "severity": "high"}
    if connector is not None:
        body["connector"] = connector
    resp = matrix.API.post(f"{matrix.API_URL}/api/alerts", json=body, timeout=10)
    if resp.status_code != 200:
        raise AlertGateFailure(f"create the {kind} rule {name!r}: expected 200, got {resp.status_code} {resp.text[:300]}")
    rule_id = (resp.json().get("rule") or {}).get("id")
    if not rule_id:
        raise AlertGateFailure(f"create the {kind} rule {name!r}: the answer carries no rule id: {resp.text[:300]}")
    return rule_id


def delete_rule(rule_id: str) -> None:
    """Best effort: a throwaway stack does not need it, a developer's local one does."""
    try:
        matrix.API.delete(f"{matrix.API_URL}/api/alerts", params={"id": rule_id}, timeout=10)
    except requests.RequestException as exc:
        say(f"cleanup: the rule {rule_id} could not be deleted ({type(exc).__name__})")


def list_alert_instances() -> tuple[list[dict], str]:
    """`GET /api/overview/alerts` (`overview::list_alerts`), the list the
    Alerts page reads. Returns the items and the raw text of the response."""
    resp = matrix.API.get(f"{matrix.API_URL}/api/overview/alerts", timeout=10)
    if resp.status_code != 200:
        raise AlertGateFailure(f"GET /api/overview/alerts: expected 200, got {resp.status_code} {resp.text[:300]}")
    return resp.json(), resp.text


def wait_for_run_failure(run_id: str, what: str) -> float:
    """Poll Dagster until the run is FAILURE and return its `endTime` (epoch
    seconds; the time the gate first saw the FAILURE, with a printed note, if
    Dagster has none). A run that SUCCEEDS is a failure of this gate."""
    query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename ... on Run { status endTime } } }"
    seen: dict = {}

    def check() -> bool:
        resp = requests.post(matrix.DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
        resp.raise_for_status()
        run = resp.json().get("data", {}).get("pipelineRunOrError", {})
        status = run.get("status")
        say(f"{what} run {run_id} status={status}")
        if status == "SUCCESS":
            raise AlertGateFailure(f"{what} run {run_id} SUCCEEDED, but it was set up to fail")
        if status == "FAILURE":
            seen["end_time"] = run.get("endTime")
            seen["seen_at"] = time.time()
            return True
        return False

    try:
        matrix._wait_for(f"{what} run {run_id} FAILURE", check, RUN_FAILURE_TIMEOUT_S, interval_s=3.0)
    except matrix.G6Failure as exc:
        raise AlertGateFailure(str(exc)) from exc
    if seen["end_time"] is None:
        say(f"note: Dagster reported no endTime for run {run_id}; measuring from when the gate first saw FAILURE")
        return float(seen["seen_at"])
    return float(seen["end_time"])


def expect_arrival(receiver: Receiver, path: str, ended_at: float, what: str) -> str:
    """Wait for the webhook at `path` until `WITHIN_SECONDS` after `ended_at`,
    print the seconds, and return the body."""
    arrival = receiver.wait_for(path, ended_at + WITHIN_SECONDS)
    if arrival is None:
        raise AlertGateFailure(
            f"{what}: no webhook arrived within {WITHIN_SECONDS}s of the run's end "
            "(PIPELINE_RUN_TOKEN set on lakehouse-api and dagster-code-location? the sensor ticks every 30s)"
        )
    arrived_at, body = arrival
    seconds = seconds_after(ended_at, arrived_at)
    say(f"{what}: the webhook arrived {seconds:.1f}s after the run ended (limit {WITHIN_SECONDS}s)")
    if seconds > WITHIN_SECONDS:
        raise AlertGateFailure(f"{what}: the webhook arrived {seconds:.1f}s after the run ended, over the {WITHIN_SECONDS}s limit")
    return body


# ── steps ─────────────────────────────────────────────────────────────────


def register_failing_connector(suffix: str) -> tuple[str, str]:
    """A MySQL connector on the `mysql-g6` fixture whose credential is not the
    user's password, with its ingest spec. Returns (connector id, the wrong
    password). The same set-up as the SRC-7 step of `g6_ingest_matrix_test.py`."""
    wrong_password = "g17-wrong-" + secrets.token_hex(4)
    body = matrix._create_connector(
        name=f"g17-failing-{suffix}", kind="MySQL", host="mysql-g6:3306",
        credential={"source": "file", "primary": "password"},
    )
    connector_id = body["id"]
    derived = (body.get("credential") or {}).get("primary")
    if not derived:
        raise AlertGateFailure(f"the create response carried no derived credential name: {body}")
    matrix._write_credential_file(derived, wrong_password)
    spec = matrix.API.put(
        f"{matrix.API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "sql", "ingestMode": "batch",
            "dial": {"driver": "mysql", "host": "mysql-g6", "port": 3306, "database": "g6_ingest", "user": "g6_reader"},
            "sourceObjects": [{"name": "orders", "target": f"g17_failing_{suffix}"}],
        },
        timeout=10,
    )
    if not spec.ok:
        raise AlertGateFailure(f"set the ingest spec failed: {spec.status_code} {spec.text[:300]}")
    return connector_id, wrong_password


def step_connector_failure(receiver: Receiver, base_url: str, suffix: str) -> None:
    """Steps 1 and 2: one failed run, two rules (a receiver that answers 200
    and one that answers 500)."""
    connector_id, wrong_password = register_failing_connector(suffix)
    ok_path = f"/ok-connector-{suffix}"
    err_path = f"/err-connector-{suffix}"
    ok_rule = create_rule(f"g17 connector ok {suffix}", "connector_failure", base_url + ok_path, connector_id)
    err_rule = create_rule(f"g17 connector 500 {suffix}", "connector_failure", base_url + err_path, connector_id)
    say(f"rules {ok_rule} (answers 200) and {err_rule} (answers 500) for failed runs of connector {connector_id}")
    try:
        run_id = matrix._post_ingest_run(connector_id).json().get("runId")
        if not run_id:
            raise AlertGateFailure("ingest/run returned no runId")
        say(f"launched run {run_id} (wrong credential; it must fail)")
        ended_at = wait_for_run_failure(run_id, "connector")

        body = expect_arrival(receiver, ok_path, ended_at, "connector failure -> webhook")
        check_webhook_body(body, must_name=[connector_id, run_id], forbidden=FORBIDDEN_IN_BODY + (wrong_password.lower(),))
        say("connector failure -> webhook: the body names the connector and the run, and carries no source or driver text")

        # Step 2. The 500 receiver was posted to by the same delivery; the
        # instance is written after the deliveries, so poll the list.
        if not receiver.wait_for(err_path, time.time() + 30):
            raise AlertGateFailure("webhook answers 500: the failing receiver was never posted to")
        deadline = time.time() + 60
        item, raw = None, ""
        while time.time() < deadline:
            alerts, raw = list_alert_instances()
            item = instance_of_rule(alerts, err_rule)
            if item is not None:
                break
            time.sleep(2.0)
        if item is None:
            raise AlertGateFailure(f"webhook answers 500: no alert of rule {err_rule} in GET /api/overview/alerts after 60s")
        if item.get("delivered") is not False or item.get("deliveryReason") != SERVER_ERROR_REASON:
            raise AlertGateFailure(
                f"webhook answers 500: expected delivered=false and reason {SERVER_ERROR_REASON!r}, "
                f"got delivered={item.get('delivered')!r} deliveryReason={item.get('deliveryReason')!r}"
            )
        if receiver.error_body in raw:
            raise AlertGateFailure("webhook answers 500: the Alerts list carries the receiver's response body")
        say(f"webhook answers 500: listed as not delivered, reason {SERVER_ERROR_REASON!r}, none of the receiver's text")
        good = instance_of_rule(alerts, ok_rule)
        if good is None or good.get("delivered") is not True:
            raise AlertGateFailure(f"webhook answers 500: the 200 rule's alert should be listed as delivered, got {good}")
        say("the rule whose receiver answered 200 is listed as delivered")
    finally:
        delete_rule(ok_rule)
        delete_rule(err_rule)


def step_upload_failure(receiver: Receiver, base_url: str, suffix: str) -> None:
    """Step 3: an upload whose load fails because the file has a header and no
    row (`file_ingest.NO_ROWS`, `allow_retries=False`)."""
    path = f"/ok-upload-{suffix}"
    rule_id = create_rule(f"g17 upload failure {suffix}", "upload_failure", base_url + path, None)
    say(f"rule {rule_id} for failed uploads")
    upload_id = ""
    try:
        filename = f"g17-header-only-{suffix}.csv"
        # Unique bytes, so another run's upload of the same file is no "duplicate".
        content = f"id_{suffix},name_{suffix}\n".encode()
        created = matrix.API.post(
            f"{matrix.API_URL}/api/uploads", files={"file": (filename, content, "text/csv")}, timeout=60
        )
        if created.status_code != 201:
            raise AlertGateFailure(f"POST /api/uploads: expected 201, got {created.status_code} {created.text[:300]}")
        upload_id = created.json()["id"]
        loaded = matrix.API.post(
            f"{matrix.API_URL}/api/uploads/{upload_id}/ingest",
            json={"bronzeTable": f"g17_upload_{suffix}", "mode": "replace", "encoding": "utf-8", "delimiter": ",", "headerRow": 0},
            timeout=60,
        )
        if loaded.status_code != 200 or not loaded.json().get("runId"):
            raise AlertGateFailure(f"POST /api/uploads/{upload_id}/ingest: expected 200 and a runId, got {loaded.status_code} {loaded.text[:300]}")
        run_id = loaded.json()["runId"]
        say(f"launched the load of upload {upload_id}, run {run_id} (a header and no row; it must fail)")
        ended_at = wait_for_run_failure(run_id, "upload")
        body = expect_arrival(receiver, path, ended_at, "upload failure -> webhook")
        check_webhook_body(body, must_name=[upload_id, run_id, filename], forbidden=("traceback",))
        say("upload failure -> webhook: the body names the file, the upload and the run")
    finally:
        delete_rule(rule_id)
        if upload_id:
            try:
                matrix.API.delete(f"{matrix.API_URL}/api/uploads/{upload_id}", timeout=30)
            except requests.RequestException as exc:
                say(f"cleanup: the upload {upload_id} could not be deleted ({type(exc).__name__})")


def named(step: str, fn, *args) -> None:
    """Run a step; whatever it raises comes out as AlertGateFailure naming it."""
    try:
        fn(*args)
    except AlertGateFailure as exc:
        raise AlertGateFailure(f"{step}: {exc}") from exc
    except matrix.G6Failure as exc:
        raise AlertGateFailure(f"{step}: {exc}") from exc
    except requests.RequestException as exc:
        raise AlertGateFailure(f"{step}: {type(exc).__name__}: {exc}") from exc


def run() -> None:
    named("wait for the services", matrix.step_wait_for_services)
    named("sign in", matrix.step_login)
    named("tenant", matrix.step_ensure_tenant)
    # Uploads belong to the active tenant (`tenant_scope::resolve`).
    matrix.API.headers["X-Tenant"] = matrix.TENANT_ID

    suffix = secrets.token_hex(3)
    receiver = Receiver(error_body=f"SRC17-RECEIVER-BODY-MARKER-{secrets.token_hex(6)}")
    receiver.start()
    try:
        host = own_address("lakehouse-api")
        base_url = f"http://{host}:{RECEIVER_PORT}"
        say(f"receiver listening on {base_url}")
        # SEC-10 (alert webhooks cannot reach internal addresses) is not built
        # yet. When it lands it will refuse this receiver's address by default
        # and must give this gate its documented opt-in; this gate must not
        # weaken the default.
        named("connector failure and webhook answering 500", step_connector_failure, receiver, base_url, suffix)
        named("upload failure", step_upload_failure, receiver, base_url, suffix)
    finally:
        receiver.stop()


def main() -> int:
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)
    started = time.time()
    try:
        run()
    except AlertGateFailure as exc:
        print(f"[{GATE}] FAILED: {exc}", file=sys.stderr)
        return 1
    except Exception:  # noqa: BLE001 -- any other error is a failure of the gate and must be named as one
        import traceback
        print(f"[{GATE}] FAILED: unexpected error, not an AlertGateFailure:\n{traceback.format_exc()}", file=sys.stderr)
        return 1
    print(f"[{GATE}] PASS ({time.time() - started:.0f}s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
