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
    result = subprocess.run(["sh", script, "p5cdc"], capture_output=True, text=True, check=False)
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


def main() -> int:
    try:
        step_wait_for_services()
        step_create_catalog_database()
        step_wait_for_table_registered()
        order_id, _ = step_insert_visible_within_budget()
        step_update_visible_within_budget(order_id)
        step_delete_visible_within_budget(order_id)
        step_row_counts_use_a_where_predicate()
        step_verify_slot_cleanup_on_connector_delete()
    except G4Failure as exc:
        print(f"[g4] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g4] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
