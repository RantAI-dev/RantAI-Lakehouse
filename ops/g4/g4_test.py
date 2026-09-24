#!/usr/bin/env python3
"""G4 acceptance test (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3, P5):

1. INSERT, UPDATE, and DELETE on the Postgres source
   (`p5_cdc.orders`) are all visible in ClickHouse — reading the Bronze
   Iceberg table `debezium-server-iceberg` writes through Lakekeeper —
   within an agreed latency.
2. Replication slot cleanup is verified on connector delete: after the
   test, the connector's Debezium Server process is stopped and its
   Postgres replication slot is dropped
   (`ops/debezium/deprovision_connector.sh`); the slot must not remain,
   pinning WAL, afterward.

# Chosen latency budget: 20 seconds

`docs/plans/P5-RESULT.md`'s manual measurement observed each CDC event
committed to Iceberg roughly 0.2-1.5s after the source transaction (the
`CommitReport`/`Committed N events` log lines land within that window of
the `INSERT`/`UPDATE`/`DELETE` statement completing). 20 seconds is a
10-100x margin over that measured commit latency — generous enough to
absorb CI scheduling jitter and the debezium-server-iceberg image's own
per-batch flush interval, while still being tight enough that a real
regression (e.g. the connector silently not running) fails the gate
promptly rather than timing out at some much larger number that would
mask a slow but real problem.

# Why `count() WHERE 1`, never a bare `count()`

`docs/plans/P5-RESULT.md`'s (A) finding: ClickHouse 26.3's bare
`count()`/`count(*)`/`count(<col>)` over a merge-on-read Iceberg table with
equality deletes takes a metadata-only fast path that does NOT apply
delete filtering and overcounts. Every row-count check in this test uses a
`WHERE` predicate (forcing the correct, row-scanning path) — this is not
optional stylistic choice, it is the one workaround that measurement
requires.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
import urllib.parse

import requests

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
LAKEKEEPER_CATALOG_URI = os.environ.get("CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog")
RUSTFS_S3_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
# R1 (ADR 0011): see `ops/g3a/g3a_test.py`'s identical comment —
# `catalog_credential` needs the OAuth2 `client_id:client_secret` form,
# pointed at `ops/oidc-mock`'s `/token` endpoint. Empty on a pre-R1 or
# authz-disabled stack.
CH_OAUTH_CLIENT_ID = os.environ.get("CH_OAUTH_CLIENT_ID", "")
CH_OAUTH_SERVER_URI = os.environ.get("CH_OAUTH_SERVER_URI", "")

# Duplicated from ops/g3a/g3a_test.py's step_login/API session — g4_test.py
# has no shared package with g3a_test.py to import this from (each g*
# gate script is a standalone entrypoint), matching this file's existing
# precedent of duplicating ch_query/_wait_for from the same source.
API_URL = os.environ.get("API_URL", "http://lakehouse-api:8089")
# oidc-mock's /authorize + /token endpoints that
# step_oidc_authorization_code_round_trip drives lakehouse-api's
# /api/auth/oidc/{start,callback} against. Mirrors this file's existing
# `API_URL` env-var-with-default pattern, not a new style.
OIDC_MOCK_URL = os.environ.get("OIDC_MOCK_URL", "http://oidc-mock:8090")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "admin@example.invalid")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "changeme")
ALERTS_RUN_TOKEN = os.environ.get("ALERTS_RUN_TOKEN", "")

API = requests.Session()


def step_login() -> None:
    """Every `/api/*` route this step calls (`/api/alerts`, `/api/overview/
    alerts`, `/api/overview/alerts/{id}/silence`) is `Policy::RequiresAuth`
    or stricter (`policy.rs`, confirmed) — log in as the bootstrap admin
    and let `API` carry the session cookie, exactly as
    `ops/g3a/g3a_test.py::step_login` already does for the identical
    reason."""
    login = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        raise G4Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        if not rotated.ok:
            raise G4Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
        if not relogin.ok:
            raise G4Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[g4] logged in as bootstrap admin")


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
            "target": "http://127.0.0.1:1/g4-webhook-sink",
            "severity": "critical",
        },
        timeout=10,
    )
    if not resp.ok:
        raise G4Failure(f"failed to seed the WAL alert rule: {resp.status_code} {resp.text}")
    print("[g4] seeded WAL alert rule (idempotent upsert)")


def ch_auth_settings() -> str:
    if not CH_OAUTH_CLIENT_ID:
        return ""
    return (
        f", catalog_credential = '{CH_OAUTH_CLIENT_ID}:unused', "
        f"oauth_server_uri = '{CH_OAUTH_SERVER_URI}'"
    )

CATALOG_DB = "icecat_g4"
TABLE = "`default.p5cdc_p5_cdc_orders`"

LATENCY_BUDGET_SECONDS = 20.0


class G4Failure(Exception):
    pass


def ch_query(sql: str) -> str:
    resp = requests.post(CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30)
    if not resp.ok:
        raise G4Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


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


def _wait_for(name: str, check, timeout_s: float, interval_s: float = 1.0):
    deadline = time.time() + timeout_s
    last_err = None
    while time.time() < deadline:
        try:
            value = check()
            if value is not None:
                elapsed = timeout_s - (deadline - time.time())
                print(f"[g4] {name}: visible after {elapsed:.1f}s")
                return value
        except Exception as exc:  # noqa: BLE001
            last_err = exc
        time.sleep(interval_s)
    raise G4Failure(f"timed out after {timeout_s}s waiting for {name}: {last_err}")


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


def step_wait_for_services() -> None:
    _wait_for(
        "ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1" or True, 60
    )
    pg_exec("SELECT 1;")
    print("[g4] ClickHouse and Postgres are reachable")


def step_create_catalog_database() -> None:
    ch_query(
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} "
        f"ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}', "
        f"storage_endpoint = '{RUSTFS_S3_ENDPOINT}'{ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1"
    )


def step_wait_for_table_registered() -> None:
    def check():
        text = ch_query(
            f"SHOW TABLES FROM {CATALOG_DB} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
        )
        return "default.p5cdc_p5_cdc_orders" if "p5cdc_p5_cdc_orders" in text else None

    _wait_for("Bronze table registered in Lakekeeper (initial snapshot committed)", check, 60)


def current_amount(order_id: int):
    text = ch_query(
        f"SELECT amount FROM {CATALOG_DB}.{TABLE} WHERE id = {order_id} AND __deleted = 'false' "
        "SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
    ).strip()
    return text or None


def is_deleted(order_id: int):
    text = ch_query(
        f"SELECT __deleted FROM {CATALOG_DB}.{TABLE} WHERE id = {order_id} "
        "SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
    ).strip()
    return text == "true" if text else None


def step_insert_visible_within_budget() -> None:
    new_id = 9001
    pg_exec(f"DELETE FROM p5_cdc.orders WHERE id = {new_id};")
    t0 = time.time()
    pg_exec(f"INSERT INTO p5_cdc.orders (id, customer, amount) VALUES ({new_id}, 'g4_insert', 42.00);")

    def check():
        val = current_amount(new_id)
        return val if val == "42" else None

    _wait_for(f"INSERT (id={new_id})", check, LATENCY_BUDGET_SECONDS)
    print(f"[g4] INSERT latency budget: {LATENCY_BUDGET_SECONDS}s (measured basis: docs/plans/P5-RESULT.md)")
    return new_id, t0


def step_update_visible_within_budget(order_id: int) -> None:
    pg_exec(f"UPDATE p5_cdc.orders SET amount = 77.77 WHERE id = {order_id};")

    def check():
        val = current_amount(order_id)
        return val if val == "77.77" else None

    _wait_for(f"UPDATE (id={order_id})", check, LATENCY_BUDGET_SECONDS)


def step_delete_visible_within_budget(order_id: int) -> None:
    pg_exec(f"DELETE FROM p5_cdc.orders WHERE id = {order_id};")

    def check():
        return True if is_deleted(order_id) else None

    _wait_for(f"DELETE (id={order_id})", check, LATENCY_BUDGET_SECONDS)


def step_row_counts_use_a_where_predicate() -> None:
    """Documents/asserts the (A) workaround at the point of use: a bare
    count() is not trustworthy on this table (docs/plans/P5-RESULT.md), so
    this test never uses one and this step proves the WHERE-qualified form
    still returns a sane, non-zero count."""
    count = ch_query(
        f"SELECT count() FROM {CATALOG_DB}.{TABLE} WHERE id > 0 SETTINGS allow_database_iceberg=1 "
        "FORMAT TabSeparated"
    ).strip()
    if not count.isdigit() or int(count) < 1:
        raise G4Failure(f"WHERE-qualified count() returned unexpected value: {count!r}")
    print(f"[g4] WHERE-qualified row count: {count}")


def step_verify_slot_cleanup_on_connector_delete() -> None:
    """G4's second acceptance criterion: a removed connector must not
    leave a replication slot behind pinning WAL."""
    slot_before = pg_exec(
        "SELECT count(*) FROM pg_replication_slots WHERE slot_name = 'p5cdc_slot';"
    )
    if slot_before != "1":
        raise G4Failure(f"expected the p5cdc_slot replication slot to exist before deprovisioning, got count={slot_before}")
    wal_retained_before = pg_exec(
        "SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn) FROM pg_replication_slots "
        "WHERE slot_name = 'p5cdc_slot';"
    )
    print(f"[g4] slot 'p5cdc_slot' exists before deprovisioning, wal_retained_bytes={wal_retained_before}")

    script = os.environ.get("DEPROVISION_SCRIPT", "/opt/deprovision_connector.sh")
    # WS3 plan review X4: `deprovision_connector.sh` now takes the slot and
    # publication names as two explicit arguments instead of deriving both
    # from one `<connector_slug>` (a slot's real name is registry-owned and
    # need not resemble its connector's slug — see
    # `dispar_orchestrate.replication_metrics`'s module doc for why the
    # same guess was removed there). The demo CDC connector's names are the
    # `p5cdc_slot`/`p5cdc_pub` pair this file already asserts on above.
    result = subprocess.run(["sh", script, "p5cdc_slot", "p5cdc_pub"], capture_output=True, text=True, check=False)
    print(result.stdout)
    if result.returncode != 0:
        raise G4Failure(f"deprovision_connector.sh failed: {result.stderr}")

    slot_after = pg_exec(
        "SELECT count(*) FROM pg_replication_slots WHERE slot_name = 'p5cdc_slot';"
    )
    if slot_after != "0":
        raise G4Failure(
            f"replication slot 'p5cdc_slot' still exists after deprovisioning (count={slot_after}) "
            "— it would keep pinning WAL indefinitely (R5)"
        )
    print("[g4] slot 'p5cdc_slot' no longer exists after deprovisioning — WAL is no longer pinned")


WAL_BREACH_SLOT_NAME = "g4_wal_breach_slot"


def step_wal_breach_produces_a_silenceable_alert_instance() -> None:
    """WS5 acceptance (grand plan §7): a WAL breach on the CDC slot
    produces a real alert_instance row via /api/alerts/run, and that row
    can be silenced through the API.

    C3-F2: the prior revision breached WAL by `docker compose stop
    debezium-server`, run from inside `g4-test-runner` (`docker-compose.
    yml`'s `g4-test-runner` block: `image: python:3.12-slim`, no Docker
    CLI installed, no `/var/run/docker.sock` mount — confirmed this
    revision). That call could never succeed; the step raised
    `FileNotFoundError` before growing any WAL, so this half of the gate
    never actually ran. Mounting the Docker socket into a test container
    to fix this was considered and rejected deliberately, not overlooked:
    it would hand this gate root-equivalent control of the host's Docker
    daemon just to grow a number.

    Breach mechanism instead: open a SECOND, deliberately unconsumed
    logical replication slot with `pg_create_logical_replication_slot`,
    using the same `pgoutput` output plugin the real CDC slot uses
    (`docker-compose.yml`'s `debezium-server` command block:
    `debezium.source.plugin.name=pgoutput`). A slot with no consumer
    attached is `active = false` in `pg_replication_slots` from the
    moment it is created, and `replication_metrics.py::_status_for`
    treats `not active` as `critical` independently of WAL byte volume
    ("a disconnected [consumer] still pins WAL ... exactly why this
    checks `active` independently of the byte thresholds"). WAL is still
    grown with the existing ordinary UPDATEs against the five rows the
    compose seed already inserted (docker-compose.yml, ids 1..5) —
    REPLICA IDENTITY FULL means each UPDATE writes a full-row WAL image,
    so no new rows and no primary-key collision — so the slot's
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
    (`p5cdc_slot`) ever went unhealthy for an unrelated reason. This step
    never touches `p5cdc_slot` or `debezium-server` — the CDC pipeline
    the rest of this gate exercises keeps running, untouched, throughout.

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
            pg_exec(f"UPDATE p5_cdc.orders SET amount = amount + 0.01 WHERE id = {(i % 5) + 1};")
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
    try:
        step_wait_for_services()
        step_login()
        step_seed_wal_alert_rule()
        step_create_catalog_database()
        step_wait_for_table_registered()
        order_id, _ = step_insert_visible_within_budget()
        step_update_visible_within_budget(order_id)
        step_delete_visible_within_budget(order_id)
        step_row_counts_use_a_where_predicate()
        step_wal_breach_produces_a_silenceable_alert_instance()
        step_verify_slot_cleanup_on_connector_delete()
        step_oidc_authorization_code_round_trip()
    except G4Failure as exc:
        print(f"[g4] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g4] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
