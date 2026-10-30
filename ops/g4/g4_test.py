#!/usr/bin/env python3
"""G4 acceptance test: PostgreSQL change capture on `Kafka Connect` (`SRC-4` slice 2, decision
S2-j; ADR 0016).

Since slice 3 (plan task T3.6) the steps every change-capture gate shares live in `cdc_gate.py`
(the connector registered through the API, the password kept out of Kafka Connect's config, the
first copy, the freshness budget, a pause and a worker restart, a schema change, `DELETE`), and
`g4_mysql_test.py` and `g4_mssql_test.py` use the same module. What stays here is what only
PostgreSQL has: `PgSource` (psql, a replication slot and a publication, `generate_series`), the
WAL-breach alert and the OIDC round trip of the original G4 gate. The steps, their order and the
output of this gate are those of slice 2.

# Source of the two tables

A schema `g4_cdc` of the stack's own `postgres:16` with two identical tables (`orders` in
"current row per key" mode, `orders_history` in "history" mode), 20,000 seeded rows each,
`REPLICA IDENTITY FULL` (so a delete event carries the old row), one publication scoped to
those two tables (never `FOR ALL TABLES`), and a table outside the publication, `wal_noise`,
that the WAL-breach step writes to.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
import urllib.parse

import requests

import cdc_gate
from cdc_gate import API, API_URL, G4Failure, _wait_for

OIDC_MOCK_URL = os.environ.get("OIDC_MOCK_URL", "http://oidc-mock:8090")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
ALERTS_RUN_TOKEN = os.environ.get("ALERTS_RUN_TOKEN", "")

CDC_SLOT_NAME = "g4_slot"
CDC_PUBLICATION_NAME = "g4_pub"
CDC_SCHEMA = "g4_cdc"


def pg_exec(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-tqA", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise G4Failure(f"psql failed: {result.stderr}\nSQL: {sql}")
    return result.stdout.strip()


def pg_rows(sql: str) -> list[tuple[str, ...]]:
    result = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-tqA", "-F", "\t", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise G4Failure(f"psql failed: {result.stderr}\nSQL: {sql}")
    return [tuple(line.split("\t")) for line in result.stdout.splitlines() if line]


class PgSource(cdc_gate.Source):
    cdc_type = "PostgreSQL CDC"
    gate = "g4"
    catalog_db = "icecat_g4"
    schema = CDC_SCHEMA
    host_for_registry = "postgres:5432"
    now = "now()"
    needs_schema_history = False
    # AC7: this gate also stops `kafka-connect` and watches the lag alert (`cdc_gate`).
    lag_alert_gate = True

    def __init__(self) -> None:
        self.password = os.environ["PGPASSWORD"]

    def missing_case(self) -> cdc_gate.MissingCase:
        """AC3: a connector that names a publication that does not exist. It selects no table, so
        nothing is started in Kafka Connect for it, and no slot is created."""
        dial = {**self.dial(), "slotName": "g4_slot_missing", "publicationName": "g4_pub_missing"}
        return cdc_gate.MissingCase(
            dial=dial, source_objects=[], password=self.password,
            setting="publication", message="The publication named in the connector does not exist.",
        )

    def ping(self) -> None:
        pg_exec("SELECT 1;")

    def setup(self) -> None:
        """The gate's own source (it replaces `g4-source-init`'s fixed seed since slice 2)."""
        pg_exec(f"DROP SCHEMA IF EXISTS {CDC_SCHEMA} CASCADE; CREATE SCHEMA {CDC_SCHEMA};")
        for table in cdc_gate.TABLES.values():
            pg_exec(
                f"CREATE TABLE {CDC_SCHEMA}.{table} (id bigint PRIMARY KEY, customer text, "
                "amount numeric(12,2), status text, updated_at timestamptz); "
                f"ALTER TABLE {CDC_SCHEMA}.{table} REPLICA IDENTITY FULL; "
                f"INSERT INTO {CDC_SCHEMA}.{table} SELECT g, 'seed_' || g, ((g % 1000) + 0.5)::numeric(12,2), "
                f"'seed', now() FROM generate_series(1, {cdc_gate.SEED_ROWS}) g;"
            )
        pg_exec(
            f"CREATE TABLE {CDC_SCHEMA}.wal_noise (id int PRIMARY KEY, v int NOT NULL); "
            f"ALTER TABLE {CDC_SCHEMA}.wal_noise REPLICA IDENTITY FULL; "
            f"INSERT INTO {CDC_SCHEMA}.wal_noise SELECT g, 0 FROM generate_series(1, 5) g;"
        )
        pg_exec(
            f"DROP PUBLICATION IF EXISTS {CDC_PUBLICATION_NAME}; "
            f"CREATE PUBLICATION {CDC_PUBLICATION_NAME} FOR TABLE "
            + ", ".join(f"{CDC_SCHEMA}.{table}" for table in cdc_gate.TABLES.values())
            + ";"
        )

    def execute(self, sql: str) -> None:
        pg_exec(sql)

    def rows(self, sql: str) -> list[tuple[str, ...]]:
        return pg_rows(sql)

    def qualified(self, table: str) -> str:
        return f"{CDC_SCHEMA}.{table}"

    def numbers(self, lo: int, hi: int) -> str:
        return f"SELECT g FROM generate_series({lo}, {hi}) g"

    def dial(self) -> dict:
        return {
            "driver": "postgres", "host": os.environ["PGHOST"], "port": int(os.environ["PGPORT"]),
            "database": os.environ["PGDATABASE"], "user": os.environ["PGUSER"],
            "slotName": CDC_SLOT_NAME, "publicationName": CDC_PUBLICATION_NAME,
        }

    def add_column_and_observe(self) -> None:
        pg_exec(f"ALTER TABLE {self.qualified('orders')} ADD COLUMN note text;")
        pg_exec(f"UPDATE {self.qualified('orders')} SET note = 'schema_note' WHERE id = 2;")

    def before_delete(self, connector_id: str) -> None:
        slot_before = pg_exec(f"SELECT count(*) FROM pg_replication_slots WHERE slot_name = '{CDC_SLOT_NAME}';")
        if slot_before != "1":
            raise G4Failure(f"expected the {CDC_SLOT_NAME} replication slot to exist before the delete, got count={slot_before}")
        retained = pg_exec(
            f"SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn) FROM pg_replication_slots WHERE slot_name = '{CDC_SLOT_NAME}';"
        )
        print(f"[g4] slot {CDC_SLOT_NAME!r} exists before the delete, wal_retained_bytes={retained}", flush=True)

    def after_delete(self, connector_id: str) -> None:
        def gone():
            slots = pg_exec(f"SELECT count(*) FROM pg_replication_slots WHERE slot_name = '{CDC_SLOT_NAME}';")
            pubs = pg_exec(f"SELECT count(*) FROM pg_publication WHERE pubname = '{CDC_PUBLICATION_NAME}';")
            if slots == "0" and pubs == "0":
                return True
            raise G4Failure(f"after DELETE: slots={slots}, publications={pubs}")

        _wait_for("the replication slot and publication are gone", gone, 90, interval_s=3)
        print(f"[g4] slot {CDC_SLOT_NAME!r} no longer exists after the delete: WAL is no longer pinned (R5)", flush=True)


WAL_ALERT_RULE_ID = "al_g4_p5cdc_wal"


def step_seed_wal_alert_rule() -> None:
    """An ordinary `AlertKind::Alert` rule against `serving.
    replication_slot_health` — WAL-slot health needs no new alert kind
    (WS5 plan review Y4); `replication_metrics.py`'s own module doc names
    this exact rule shape (`mart = replication_slot_health`, `measure =
    unhealthy`). `console.alert_rule` is a `ReplacingMergeTree` keyed on
    `id` (`lakehouse-alerts::ensure`), so re-`PUT`ing the same id every
    run is a correct, idempotent upsert, not a conditional "if missing"
    check. `PUT`, not `POST`: `POST /api/alerts` ignores a caller's `id`
    and mints a new `al_<hex>` every time (`lakehouse_alerts::save_rule`
    with `id: None`), so each run added another rule under an id this gate
    never looked for."""
    resp = API.put(
        f"{API_URL}/api/alerts",
        json={
            "id": WAL_ALERT_RULE_ID,
            "name": "g4 WAL slot health",
            "type": "alert",
            "mart": "replication_slot_health",
            "measure": "unhealthy",
            "agg": "max",
            "op": ">",
            "threshold": 0,
            "channel": "webhook",
            # A sink nothing listens on. It was `http://127.0.0.1:1/...`, which
            # SEC-10 now refuses when the rule is saved (loopback can never be
            # allow-listed, by design). 192.0.2.0/24 is TEST-NET-1, reserved
            # for documentation and never routed, so this is still a dead
            # target, but one the address check admits: the delivery fails
            # at connect, as it did before.
            "target": "http://192.0.2.1:9/g4-webhook-sink",
            "severity": "critical",
        },
        timeout=10,
    )
    if not resp.ok:
        raise G4Failure(f"failed to seed the WAL alert rule: {resp.status_code} {resp.text}")
    print("[g4] seeded WAL alert rule (idempotent upsert)")


def _dagster_job_loaded(job: str):
    """True once a loaded Dagster repository lists `job`; None otherwise
    (`_wait_for` keeps polling on None)."""
    query = "{ repositoriesOrError { ... on RepositoryConnection { nodes { jobs { name } } } } }"
    resp = requests.post(DAGSTER_URL, json={"query": query}, timeout=5)
    resp.raise_for_status()
    nodes = (resp.json().get("data") or {}).get("repositoriesOrError", {}).get("nodes") or []
    return True if any(j.get("name") == job for n in nodes for j in n.get("jobs", [])) else None


def _dagster_run_finished(run_id: str):
    """True once the run succeeded; raises if it failed; None while it is
    still queued or running (`_wait_for` keeps polling on None)."""
    query = "query($id:ID!){ runOrError(runId:$id){ ... on Run { status } } }"
    resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"id": run_id}}, timeout=5)
    resp.raise_for_status()
    status = ((resp.json().get("data") or {}).get("runOrError") or {}).get("status")
    if status == "SUCCESS":
        return True
    if status in ("FAILURE", "CANCELED"):
        raise G4Failure(f"replication_slot_check_job run {run_id} ended {status}")
    return None


WAL_BREACH_SLOT_NAME = "g4_wal_breach_slot"


def step_wal_breach_produces_a_silenceable_alert_instance() -> None:
    """WS5 acceptance (grand plan §7): a WAL breach on the CDC slot
    produces a real alert_instance row via /api/alerts/run, and that row
    can be silenced through the API.

    C3-F2: the breach cannot be made by stopping the connector's container:
    `g4-test-runner` (`docker-compose.yml`) is `python:3.12-slim` with no Docker
    CLI and no `/var/run/docker.sock` mount, and mounting the Docker socket into
    a test container would hand this gate root-equivalent control of the host's
    Docker daemon just to grow a number. (The one container restart this gate
    needs, `kafka-connect`, is done by the CI job, see the module doc.)

    Breach mechanism: open a SECOND, deliberately unconsumed
    logical replication slot with `pg_create_logical_replication_slot`,
    using the same `pgoutput` output plugin the real CDC slot uses
    (`plugin.name=pgoutput` in the connector config `lakehouse-store` renders). A slot with no consumer
    attached is `active = false` in `pg_replication_slots` from the
    moment it is created, and `replication_metrics.py::_status_for`
    treats `not active` as `critical` independently of WAL byte volume
    ("a disconnected [consumer] still pins WAL ... exactly why this
    checks `active` independently of the byte thresholds"). WAL is still
    grown with ordinary UPDATEs against the five rows of `wal_noise`, a
    table of this gate's schema that is NOT in the connector's publication
    (so the connector and the Bronze comparison never see them) —
    REPLICA IDENTITY FULL means each UPDATE writes a full-row WAL image —
    so the slot's
    `wal_retained_bytes` is also genuinely non-trivial, not just
    `active = false` on an idle slot.

    `replication_slot_check_job` reads every logical slot on the source
    (`SELECT ... FROM pg_replication_slots WHERE slot_type = 'logical'`,
    no slot-name filter), and the seeded alert rule
    (`step_seed_wal_alert_rule`) is `max(unhealthy) > 0` over ALL slots
    in `serving.replication_slot_health` — it cannot distinguish which
    slot is unhealthy, only that at least one is. That is fine for what
    this step needs (a real, non-fabricated breach that fires the rule)
    but it means the rule would fire identically if the REAL CDC slot
    (`g4_slot`) ever went unhealthy for an unrelated reason. This step
    never touches `g4_slot` or the connector — the CDC pipeline the rest
    of this gate exercises keeps running, untouched, throughout.

    The breach slot is dropped in a `finally`: an unconsumed logical
    slot pins WAL indefinitely (the same R5 risk this whole job exists
    to catch) and would fill the disk of any environment that left it
    behind, CI runner included.
    """
    pg_exec(
        f"SELECT pg_create_logical_replication_slot('{WAL_BREACH_SLOT_NAME}', 'pgoutput');"
    )
    print(f"[g4] created deliberately unconsumed slot {WAL_BREACH_SLOT_NAME!r} (plugin=pgoutput)")
    try:
        for i in range(2000):
            pg_exec(f"UPDATE {CDC_SCHEMA}.wal_noise SET v = v + 1 WHERE id = {(i % 5) + 1};")
        # The webserver answering is not the code location having loaded:
        # compose can recreate `dagster-code-location` when this runner
        # starts, and a launch in that window fails with
        # PipelineNotFoundError (seen in CI). Wait until Dagster lists it.
        _wait_for(
            "Dagster code location (replication_slot_check_job loaded)",
            lambda: _dagster_job_loaded("replication_slot_check_job"),
            120,
            2.0,
        )
        # replication_slot_check_job writes lake.bronze_meta.replication_slot
        # on its own 15-minute schedule (replication_metrics.py:455,
        # cron_schedule="*/15 * * * *") — run it once, out of band, via
        # Dagster's launchRun mutation so this gate does not wait 15
        # minutes for a fresh row.
        launch = requests.post(
            DAGSTER_URL,
            json={
                "query": "mutation($job:String!){ launchRun(executionParams:{selector:{repositoryLocationName:\"dispar_orchestrate.definitions\",repositoryName:\"__repository__\",jobName:$job}, runConfigData:\"{}\"}){ __typename ... on LaunchRunSuccess { run { id } } ... on PythonError { message } } }",
                "variables": {"job": "replication_slot_check_job"},
            },
            timeout=30,
        )
        launch.raise_for_status()
        launched = launch.json().get("data", {}).get("launchRun", {})
        if launched.get("__typename") != "LaunchRunSuccess":
            raise G4Failure(f"failed to launch replication_slot_check_job: {launch.text}")
        # Wait for THIS run to finish while the breach slot still exists. A
        # fixed sleep let the run start after this step had already
        # evaluated and dropped the slot (seen: the run recorded only the
        # healthy CDC slot).
        run_id = launched["run"]["id"]
        _wait_for(
            f"replication_slot_check_job run {run_id} finished",
            lambda: _dagster_run_finished(run_id),
            180,
            2.0,
        )

        run_resp = API.post(
            f"{API_URL}/api/alerts/run",
            headers={"x-run-token": ALERTS_RUN_TOKEN} if ALERTS_RUN_TOKEN else {},
            timeout=30,
        )
        run_resp.raise_for_status()
        results = run_resp.json()["results"]
        fired = next((r for r in results if r.get("id") == WAL_ALERT_RULE_ID and r.get("fired")), None)
        if fired is None:
            raise G4Failure(f"expected a fired result for {WAL_ALERT_RULE_ID}, got: {results}")

        alerts_resp = API.get(f"{API_URL}/api/overview/alerts", timeout=10)
        alerts_resp.raise_for_status()
        # Match the instance produced by THIS rule specifically — not
        # "any open alert" (WS5 plan review U8), since other rules may
        # also be open in a shared CI environment.
        open_alert = next(
            (a for a in alerts_resp.json() if a["status"] == "open" and a.get("ruleId") == WAL_ALERT_RULE_ID),
            None,
        )
        if open_alert is None:
            raise G4Failure(f"no open alert_instance row for {WAL_ALERT_RULE_ID} after a fired WAL breach")
        if open_alert.get("severity") != "critical":
            raise G4Failure(
                f"expected the fired instance's severity to be copied verbatim from the seeded "
                f"rule's severity ('critical'), got: {open_alert.get('severity')!r}"
            )

        # The 15-minute dedup holds (insert_from_fired_rule's `fired_at >
        # now - 15min` check, independent of silence -- the rule is not
        # yet silenced at this point): a second /api/alerts/run call while
        # the breach is still ongoing must NOT insert a second
        # alert_instance row for this rule, even though the rule still
        # fires on every evaluation.
        dedup_run_resp = API.post(
            f"{API_URL}/api/alerts/run",
            headers={"x-run-token": ALERTS_RUN_TOKEN} if ALERTS_RUN_TOKEN else {},
            timeout=30,
        )
        dedup_run_resp.raise_for_status()
        alerts_after_dedup_run = API.get(f"{API_URL}/api/overview/alerts", timeout=10)
        alerts_after_dedup_run.raise_for_status()
        instances_for_rule_after_dedup = [
            a for a in alerts_after_dedup_run.json() if a.get("ruleId") == WAL_ALERT_RULE_ID
        ]
        if len(instances_for_rule_after_dedup) != 1:
            raise G4Failure(
                f"expected exactly one alert_instance row for {WAL_ALERT_RULE_ID} within the "
                f"15-minute dedup window, got {len(instances_for_rule_after_dedup)}: "
                f"{instances_for_rule_after_dedup}"
            )

        silence_resp = API.post(
            f"{API_URL}/api/overview/alerts/{open_alert['id']}/silence",
            json={"untilMinutes": 60},
            timeout=10,
        )
        if silence_resp.status_code != 200 or not silence_resp.json().get("silencedUntil"):
            raise G4Failure(f"silence did not return a populated silencedUntil: {silence_resp.text}")

        # Silencing suppresses the next insert (and, per
        # `deliver_unless_silenced`/`persist_fired_results`, the next
        # delivery too -- both check the same `SilenceSource`): re-running
        # /api/alerts/run now that the instance is silenced must not
        # produce a new OPEN row for this rule (the existing row moved to
        # "acknowledged" when it was silenced, and stays there). This gate
        # has no webhook receiver to inspect delivery directly (no HTTP
        # surface exists to prove a webhook was NOT sent), so delivery
        # suppression is asserted by code reference here, not measured --
        # what IS externally observable, and asserted below, is that no
        # second open instance appears.
        rerun_resp = API.post(
            f"{API_URL}/api/alerts/run",
            headers={"x-run-token": ALERTS_RUN_TOKEN} if ALERTS_RUN_TOKEN else {},
            timeout=30,
        )
        rerun_resp.raise_for_status()
        alerts_after_rerun = API.get(f"{API_URL}/api/overview/alerts", timeout=10)
        alerts_after_rerun.raise_for_status()
        instances_for_rule = [
            a for a in alerts_after_rerun.json() if a.get("ruleId") == WAL_ALERT_RULE_ID
        ]
        open_after_rerun = [a for a in instances_for_rule if a["status"] == "open"]
        if open_after_rerun:
            raise G4Failure(
                f"a re-run of /api/alerts/run after silencing produced a new open "
                f"instance for {WAL_ALERT_RULE_ID} (dedup/silence-suppression did not "
                f"hold): {open_after_rerun}"
            )
        print("[g4] WAL breach produced a fired, silenceable alert_instance row; re-run after silence stayed suppressed")
    finally:
        # Not optional: an unconsumed logical slot pins WAL on the source
        # Postgres server indefinitely (R5) until it is dropped — leaving
        # this slot behind after the gate exits would fill the disk of
        # whatever environment ran it, same failure mode this job exists
        # to catch. `ON_ERROR_STOP=1` (pg_exec) means a missing slot here
        # (e.g. creation itself failed above) raises rather than silently
        # no-op'ing, so a broken cleanup is visible in gate output instead
        # of masked.
        pg_exec(f"SELECT pg_drop_replication_slot('{WAL_BREACH_SLOT_NAME}');")
        print(f"[g4] dropped slot {WAL_BREACH_SLOT_NAME!r} — WAL is no longer pinned")


def step_oidc_authorization_code_round_trip() -> None:
    """Drive a full authorization-code + PKCE login through lakehouse-api's
    real /api/auth/oidc/{start,callback} routes against oidc-mock's
    /authorize + authorization_code /token support (ops/oidc-mock/server.py)
    — not a mocked HTTP
    layer, a real three-hop redirect chain over `requests.Session` so
    cookies persist exactly as a browser's would. A separate `Session`
    from this file's module-level `API` (used by step_login and every
    alerts step above): that one already carries a bootstrap-admin
    `lh_session` cookie, and reusing it here would make it impossible to
    tell whether `/api/auth/me` below is reporting the OIDC-minted
    session's permissions or the pre-existing admin one's.
    """
    session = requests.Session()

    # Hop 1: lakehouse-api issues the PKCE challenge/state/nonce and
    # redirects to oidc-mock's /authorize. `allow_redirects=False` at
    # every hop so this test controls and asserts on each redirect
    # individually, rather than following the whole chain blind and only
    # checking the final response.
    start = session.get(f"{API_URL}/api/auth/oidc/start", allow_redirects=False, timeout=10)
    if start.status_code != 302:
        raise G4Failure(f"/api/auth/oidc/start did not redirect: HTTP {start.status_code}")
    authorize_url = start.headers.get("Location", "")
    if not authorize_url.startswith(f"{OIDC_MOCK_URL}/authorize"):
        raise G4Failure(f"/start redirected somewhere other than oidc-mock's /authorize: {authorize_url}")
    if "code_challenge_method=S256" not in authorize_url:
        raise G4Failure("authorize URL is missing code_challenge_method=S256")
    flow_cookie = session.cookies.get("lh_oidc_flow")
    if not flow_cookie:
        raise G4Failure("lh_oidc_flow cookie was not set by /start")

    # Hop 2: oidc-mock's /authorize. A browser would GET it, be
    # served a login-screen stand-in whose hidden fields carry the OIDC
    # request parameters, and POST those back with the chosen identity —
    # so this test does the same. Posting `login_hint` alone would submit
    # empty redirect_uri/nonce/code_challenge fields (the mock's POST
    # handler reads its form body, not the URL's query string), and the
    # code it minted would then be bound to nothing, failing at hop 3 for
    # a reason that has nothing to do with lakehouse-api.
    authorize_params = {
        key: values[0]
        for key, values in urllib.parse.parse_qs(
            urllib.parse.urlparse(authorize_url).query
        ).items()
    }
    login_form = session.get(authorize_url, allow_redirects=False, timeout=10)
    if login_form.status_code != 200:
        raise G4Failure(
            f"oidc-mock /authorize did not serve its login screen: HTTP {login_form.status_code}"
        )
    authorize = session.post(
        authorize_url,
        data={
            "login_hint": "test-analyst",
            "redirect_uri": authorize_params.get("redirect_uri", ""),
            "nonce": authorize_params.get("nonce", ""),
            "code_challenge": authorize_params.get("code_challenge", ""),
            "state": authorize_params.get("state", ""),
        },
        allow_redirects=False,
        timeout=10,
    )
    if authorize.status_code != 302:
        raise G4Failure(f"oidc-mock /authorize did not redirect: HTTP {authorize.status_code}")
    callback_url = authorize.headers.get("Location", "")
    if "/api/auth/oidc/callback" not in callback_url:
        raise G4Failure(f"oidc-mock redirected somewhere other than the callback: {callback_url}")

    # Hop 3: lakehouse-api's callback exchanges the code (PKCE verifier
    # from the flow cookie, never re-sent by the client), verifies the id
    # token (signature/iss/aud/exp/nbf/nonce via
    # OidcAuthenticator::authenticate_with_nonce), and mints a
    # real session.
    callback = session.get(callback_url, allow_redirects=False, timeout=10)
    if callback.status_code != 302:
        raise G4Failure(f"/api/auth/oidc/callback did not redirect: HTTP {callback.status_code}")
    if "lh_session" not in session.cookies:
        raise G4Failure("no lh_session cookie was set after the OIDC callback")
    if "lh_oidc_flow" in session.cookies:
        raise G4Failure("lh_oidc_flow cookie was not cleared after the callback consumed it")

    # Prove the session is real and carries the role OIDC_ROLE_MAP mapped
    # test-analyst's "lakehouse-analysts" group to (g4-test-runner sets
    # OIDC_ROLE_MAP=lakehouse-analysts=Analyst — see the compose change
    # below), not merely that a cookie exists.
    me = session.get(f"{API_URL}/api/auth/me", timeout=10)
    if me.status_code != 200:
        raise G4Failure(f"GET /api/auth/me failed after OIDC login: HTTP {me.status_code}")
    permissions = me.json().get("permissions", [])
    if "query:read" not in permissions:
        raise G4Failure(f"OIDC-mapped Analyst role did not grant query:read: permissions={permissions}")
    print("[g4] OIDC authorization-code + PKCE round trip: session minted, Analyst role's query:read confirmed")


def main() -> int:
    return cdc_gate.run(
        PgSource(),
        before_delete=lambda _connector_id: (step_seed_wal_alert_rule(), step_wal_breach_produces_a_silenceable_alert_instance()),
        after_delete=step_oidc_authorization_code_round_trip,
    )


if __name__ == "__main__":
    raise SystemExit(main())
