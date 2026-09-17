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
    `id` (`lakehouse-alerts::ensure`), so re-`POST`ing the same id every
    run is a correct, idempotent upsert, not a conditional "if missing"
    check."""
    resp = API.post(
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


def step_wal_breach_produces_a_silenceable_alert_instance() -> None:
    """WS5 acceptance (grand plan §7): a WAL breach on the CDC slot
    produces a real alert_instance row via /api/alerts/run, and that row
    can be silenced through the API.

    Breach mechanism: pause consumption by stopping the debezium-server
    container (ops/debezium/ has exactly one script,
    deprovision_connector.sh — confirmed this revision — which removes
    the slot outright, so it cannot be reused here). WAL is grown with
    ordinary UPDATEs against the five rows the compose seed already
    inserted (docker-compose.yml:2043-2049, ids 1..5) — REPLICA IDENTITY
    FULL means each UPDATE writes a full-row WAL image, so no new rows
    and no primary-key collision, unlike the prior revision's INSERT loop
    (WS5 plan review U8).
    """
    project = os.environ.get("G4_COMPOSE_PROJECT", "g4ci")
    subprocess.run(["docker", "compose", "-p", project, "stop", "debezium-server"], check=True)
    try:
        for i in range(2000):
            pg_exec(f"UPDATE p5_cdc.orders SET amount = amount + 0.01 WHERE id = {(i % 5) + 1};")
        # replication_metrics_job writes lake.bronze_meta.replication_slot
        # on its own 15-minute schedule (replication_metrics.py:451,
        # cron_schedule="*/15 * * * *") — run it once, out of band, via
        # Dagster's launchRun mutation so this gate does not wait 15
        # minutes for a fresh row.
        launch = requests.post(
            DAGSTER_URL,
            json={
                "query": "mutation($job:String!){ launchRun(executionParams:{selector:{repositoryLocationName:\"dispar_orchestrate\",repositoryName:\"__repository__\",jobName:$job}, runConfigData:\"{}\"}){ __typename ... on LaunchRunSuccess { run { id } } ... on PythonError { message } } }",
                "variables": {"job": "replication_metrics_job"},
            },
            timeout=30,
        )
        launch.raise_for_status()
        launched = launch.json().get("data", {}).get("launchRun", {})
        if launched.get("__typename") != "LaunchRunSuccess":
            raise G4Failure(f"failed to launch replication_metrics_job: {launch.text}")
        time.sleep(10)  # let the job write its row before evaluating rules

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
        subprocess.run(["docker", "compose", "-p", project, "start", "debezium-server"], check=True)


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
    except G4Failure as exc:
        print(f"[g4] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g4] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
