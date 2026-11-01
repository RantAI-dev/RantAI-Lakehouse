#!/usr/bin/env python3
"""One readiness wait for `Trino`, shared by the two G8 gates.

# Why this file exists (DATA-21 S2 review BLOCKER 1)

Both G8 gates prove something on `Trino`: `g8_governance_test.py` proves
that a masking policy still masks, and `g8_time_travel_test.py` proves
that a past version still keeps it. Both used to send their first query
the moment the compose stack reported `service_started` — and `Trino`'s
coordinator answers HTTP while it is still booting. A statement sent in
that window fails with `SERVER_STARTING_UP` ("The server is still starting
up"), which the console reports as a 422. CI saw exactly that on the
`trino` leg of the `g8-governance` job (run 41285813391) after an earlier
run of the same job had passed: a gate that passes by timing is a flaky
gate.

So BOTH legs wait here, before their first query on the `Trino` engine,
until `Trino` can answer:

1. `GET /v1/info` reports `"starting": false` — the coordinator's own
   readiness flag. The compose `trino` service's healthcheck proves only
   that `/v1/info` ANSWERS, which it does while the server is still
   starting, so that healthcheck is not a readiness signal and cannot be
   the gate's; and
2. a `SELECT 1` through `Trino` succeeds — the flag says the coordinator
   is up, the statement says it can actually plan and run one.

The wait is bounded (five minutes by default) and its failure message
says which of the two conditions was still untrue when the budget ran
out, so a red gate says "Trino was not ready" instead of "Analyst query
failed: 422".

# Why the wait is one module and not a copy in each gate

`AGENTS.md` rule 4 (grep for the existing helper before writing one; a
duplicated guard is a finding) and the S2 round 2 review's own
instruction: one helper, both gates. This module holds the WAIT and
nothing else — no assertion, no fixture, no shared state — so the two
gates' *assertions* still cannot hide a shared bug from each other (see
`g8_time_travel_test.py`'s module docstring, which says it imports
nothing from `g8_governance_test.py`).

# Why the `SELECT 1` goes to `Trino` directly, not through the console

The wait is about the ENGINE, not about the console's own path to it.
`POST /api/query/run` also runs the policy rewrite, the tenant check and
the audit trail, and a fault in any of those would make this wait report
the wrong problem (or never finish at all). The gates' own queries still
go through the console, so the console's path is proved by the gates'
assertions — this wait only proves the coordinator underneath is ready.

`X-Trino-User` is an unauthenticated claim on this coordinator (see the
`trino` service's own block comment in `docker-compose.yml`), and the
file-based access control grants `iceberg` read-only to every user that
is not `trino-maintenance` — the one name the rules file grants WRITE
access to. The readiness query must never claim that name.
"""

from __future__ import annotations

import os
import time

import requests

# The coordinator's address as the gates reach it from inside the compose
# network. `G8_TRINO_URL` is the gate's OWN name for it, deliberately not
# the deployment's `TRINO_URL`: that one may hold a HOST-facing address
# (the `trino-host-port` opt-in publishes Trino on the host), and a gate
# that runs inside the compose network must always dial the in-network
# name.
TRINO_URL = os.environ.get("G8_TRINO_URL", "http://trino:8080")

# The budget for the wait. `Trino`'s own start-up on this stack is a
# couple of minutes (the coordinator, then the Iceberg connector's
# metadata round trip to Lakekeeper), so five minutes is a CEILING, not a
# target: a coordinator that needs longer is a broken coordinator, and the
# gate must say so instead of waiting forever.
READY_TIMEOUT_S = 300

# How often the wait re-checks. Two seconds is short enough that a
# coordinator which becomes ready costs the gate nothing, and long enough
# that a coordinator which never starts is not hammered.
POLL_INTERVAL_S = 2.0

# How long the wait goes without saying anything, before it reports that
# it is still waiting. The CI logs are the only place this is visible, and
# a gate that says nothing for five minutes looks like a hung job.
PROGRESS_EVERY_S = 30

# The name the readiness query claims. Never `trino-maintenance` (see the
# module docstring): the access-control rules grant that one WRITE access
# to the `iceberg` catalog, and a readiness check needs no privilege.
READINESS_USER = "g8-readiness"

# The statement the readiness check sends. It reads nothing — it proves
# the coordinator can plan and run a statement at all, which is what the
# gates' own first query needs.
READINESS_SQL = "SELECT 1"

# How many `nextUri` pages one attempt follows before it gives up. A
# `SELECT 1` answers in one or two pages; the cap only stops a
# misbehaving coordinator from keeping one attempt (and so the whole
# wait) alive.
MAX_PAGES = 20


class TrinoNotReady(Exception):
    """Raised when `Trino` did not become ready inside the budget."""


def _info_is_ready(url: str) -> tuple[bool, str]:
    """`GET /v1/info`, and the coordinator's own `starting` flag.

    Returns `(ready, reason)`. `reason` says what was observed, so the
    failure message of the wait can quote it.
    """
    try:
        resp = requests.get(f"{url}/v1/info", timeout=10)
    except requests.RequestException as exc:
        return False, f"GET /v1/info failed: {exc}"
    if resp.status_code != 200:
        return False, f"GET /v1/info returned {resp.status_code}"
    try:
        body = resp.json()
    except ValueError as exc:
        return False, f"GET /v1/info did not return JSON: {exc}"
    if not isinstance(body, dict):
        return False, "GET /v1/info did not return a JSON object"
    if "starting" not in body:
        # Absent is NOT read as "not starting": an unknown readiness
        # shape is not a readiness signal, and guessing "ready" here is
        # exactly the flake this wait exists to remove.
        return False, "GET /v1/info carried no 'starting' field"
    if body["starting"] is not False:
        return False, f"GET /v1/info reports starting={body['starting']!r}"
    return True, "GET /v1/info reports starting=false"


def _select_one_succeeds(url: str) -> tuple[bool, str]:
    """`SELECT 1` through `Trino`, following the coordinator's own
    `nextUri` pages until the statement is finished.

    Returns `(ok, reason)`. A statement error (which is what a
    still-starting coordinator answers with) is a failure, and its
    message is quoted, because "The server is still starting up" is
    exactly what the reader needs to see.
    """
    headers = {"X-Trino-User": READINESS_USER}
    try:
        resp = requests.post(
            f"{url}/v1/statement", data=READINESS_SQL.encode("utf-8"), headers=headers, timeout=30
        )
    except requests.RequestException as exc:
        return False, f"POST /v1/statement failed: {exc}"
    if resp.status_code != 200:
        return False, f"POST /v1/statement returned {resp.status_code}"
    try:
        body = resp.json()
    except ValueError as exc:
        return False, f"POST /v1/statement did not return JSON: {exc}"

    rows: list[list[object]] = []
    pages = 0
    while True:
        error = body.get("error")
        if error:
            message = error.get("message") if isinstance(error, dict) else str(error)
            return False, f"{READINESS_SQL} failed: {message}"
        rows.extend(body.get("data") or [])
        next_uri = body.get("nextUri")
        if not next_uri:
            break
        pages += 1
        if pages > MAX_PAGES:
            return False, f"{READINESS_SQL} still had pages after {MAX_PAGES} fetches"
        try:
            page = requests.get(next_uri, headers=headers, timeout=30)
        except requests.RequestException as exc:
            return False, f"GET the statement page failed: {exc}"
        if page.status_code != 200:
            return False, f"GET the statement page returned {page.status_code}"
        try:
            body = page.json()
        except ValueError as exc:
            return False, f"the statement page did not return JSON: {exc}"

    if not rows or str(rows[0][0]) != "1":
        return False, f"{READINESS_SQL} returned {rows!r}"
    return True, f"{READINESS_SQL} answered"


def wait_for_trino(
    url: str = TRINO_URL,
    timeout_s: int = READY_TIMEOUT_S,
    interval_s: float = POLL_INTERVAL_S,
) -> None:
    """Blocks until `Trino` can answer, or raises [`TrinoNotReady`].

    `Trino` is ready when `GET /v1/info` reports `"starting": false` AND
    a `SELECT 1` through it succeeds. The wait is bounded by
    `timeout_s`; when the budget runs out, the raised error says which of
    the two conditions was still untrue.

    # Errors

    Raises [`TrinoNotReady`] when `Trino` is not ready within `timeout_s`.
    """
    deadline = time.monotonic() + timeout_s
    attempts = 0
    reason = "the wait made no attempt"
    last_progress = time.monotonic()
    while True:
        attempts += 1
        ready, reason = _info_is_ready(url)
        if ready:
            ok, reason = _select_one_succeeds(url)
            if ok:
                print(
                    f"[g8-trino] Trino is ready at {url} after {attempts} attempt(s): {reason}",
                    flush=True,
                )
                return
        now = time.monotonic()
        if now - last_progress >= PROGRESS_EVERY_S:
            last_progress = now
            print(f"[g8-trino] still waiting for Trino at {url}: {reason}", flush=True)
        if now + interval_s >= deadline:
            break
        time.sleep(interval_s)
    raise TrinoNotReady(
        f"Trino at {url} was still not ready after {timeout_s}s ({attempts} attempt(s)): {reason}"
    )
