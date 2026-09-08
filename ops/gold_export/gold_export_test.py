#!/usr/bin/env python3
"""Gold export acceptance test (ADR 0010) — round trip proof:

  1. Seed a real `serving.*` Gold mart in ClickHouse (an ordinary
     MergeTree table — never Iceberg, so this step has nothing to do with
     R11's bare-`count()` guard).
  2. Trigger the export: `POST /api/gold/export/{mart}` on `lakehouse-api`
     — reads the mart from ClickHouse, appends it to the `gold` Iceberg
     namespace through Lakekeeper, via `iceberg-rust` (the exact write
     path G1(a) proved).
  3. Independently confirm, straight against Lakekeeper's own REST API
     (not through `lakehouse-api`, not through `ClickHouse`), that the
     Iceberg table exists at format-version 2.
  4. Read the table back through `iceberg-rust`
     (`GET /api/gold/export/{mart}`) and assert its row count matches what
     was seeded — proving the whole ClickHouse -> Rust -> Lakekeeper ->
     Iceberg -> Rust round trip, with matching counts.

Same `g1-test-runner`/`g3a-test-runner` shape: run inside the compose
network (`gold-export-test-runner` in `docker-compose.yml`), plain
`requests`, no dependency on the Rust/Python application code it verifies.
"""

from __future__ import annotations

import os
import random
import sys
import time

import requests

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
LAKEKEEPER_BASE_URI = os.environ.get("LAKEKEEPER_BASE_URI", "http://lakekeeper:8181")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
GOLD_SOURCE_SCHEMA = os.environ.get("GOLD_SOURCE_SCHEMA", "serving")
GOLD_EXPORT_RUN_TOKEN = os.environ.get("GOLD_EXPORT_RUN_TOKEN", "")
MART_NAME = os.environ.get("GOLD_MART_NAME", "gold_export_smoke")
ROW_COUNT = int(os.environ.get("GOLD_EXPORT_ROW_COUNT", "7"))
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")

API = requests.Session()
if GOLD_EXPORT_RUN_TOKEN:
    API.headers["x-run-token"] = GOLD_EXPORT_RUN_TOKEN


class GoldExportFailure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0) -> None:
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[gold-export] ready: {name}")
                return
        except Exception as exc:  # noqa: BLE001 - report the real cause below
            last_err = exc
        time.sleep(interval_s)
    raise GoldExportFailure(f"timed out waiting for {name!r}: {last_err}")


def ch_query(sql: str) -> str:
    resp = requests.post(
        CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30
    )
    if not resp.ok:
        raise GoldExportFailure(
            f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}"
        )
    return resp.text


def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1", 60)
    _wait_for(
        "lakehouse-api",
        lambda: API.get(f"{API_URL}/health", timeout=5).status_code == 200,
        60,
    )
    _wait_for(
        "Lakekeeper",
        lambda: requests.get(f"{LAKEKEEPER_BASE_URI}/management/v1/info", timeout=5).status_code
        in (200, 401, 403),
        60,
    )


def step_login() -> None:
    """`/api/gold/export/{mart}`'s `POLICY_TABLE` floor is
    `Policy::RequiresAuth` — a valid session/bearer credential is required
    to even reach the handler, regardless of `GOLD_EXPORT_RUN_TOKEN`
    (`routes::gold::check_export_token` is a SECOND, narrower guard
    layered on top, not a substitute — same two-layer shape
    `/api/alerts/run` uses). Logs in as the bootstrap admin the same way
    `ops/g3a/g3a_test.py`'s `step_login` does, including the forced
    first-login password rotation."""
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise GoldExportFailure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise GoldExportFailure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise GoldExportFailure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[gold-export] logged in as bootstrap admin")


def step_seed_gold_mart() -> None:
    """A real Gold aggregate mart shape: `region`/`total_sales`/
    `order_count`/`updated_at` — one MergeTree table, ordinary (non-
    Iceberg) `ClickHouse`, exactly the kind of `serving.*` table ADR 0010
    is exporting."""
    ch_query(f"CREATE DATABASE IF NOT EXISTS {GOLD_SOURCE_SCHEMA}")
    ch_query(f"DROP TABLE IF EXISTS {GOLD_SOURCE_SCHEMA}.`{MART_NAME}`")
    ch_query(
        f"CREATE TABLE {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` ("
        "region String, total_sales Float64, order_count UInt64, "
        "updated_at DateTime"
        ") ENGINE = MergeTree ORDER BY region"
    )
    rows = []
    for i in range(ROW_COUNT):
        region = f"region-{i}"
        total_sales = round(random.uniform(100.0, 9999.0), 2)
        order_count = random.randint(1, 500)
        rows.append(f"('{region}', {total_sales}, {order_count}, now())")
    ch_query(
        f"INSERT INTO {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` "
        f"(region, total_sales, order_count, updated_at) VALUES {', '.join(rows)}"
    )
    # Qualified with a WHERE, per R11 (`ops/lint/check_bare_iceberg_count.py`)
    # — not that this table is Iceberg at all (it's MergeTree, R11 doesn't
    # apply), but this test keeps the same discipline everywhere it counts.
    seeded = ch_query(
        f"SELECT count() FROM {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` WHERE 1 "
        "FORMAT TabSeparated"
    ).strip()
    if int(seeded) != ROW_COUNT:
        raise GoldExportFailure(f"seed row count mismatch: wanted {ROW_COUNT}, got {seeded}")
    print(f"[gold-export] seeded {ROW_COUNT} rows into {GOLD_SOURCE_SCHEMA}.{MART_NAME}")


def step_trigger_export() -> dict:
    resp = API.post(f"{API_URL}/api/gold/export/{MART_NAME}", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"POST /api/gold/export/{MART_NAME} failed: {resp.status_code} {resp.text}")
    body = resp.json()
    print(f"[gold-export] export response: {body}")
    if body.get("rowsExported") != ROW_COUNT:
        raise GoldExportFailure(
            f"rowsExported mismatch: wanted {ROW_COUNT}, got {body.get('rowsExported')}"
        )
    if body.get("formatVersion") != 2:
        raise GoldExportFailure(f"formatVersion mismatch: wanted 2, got {body.get('formatVersion')}")
    if body.get("namespace") != "gold":
        raise GoldExportFailure(f"namespace mismatch: wanted 'gold', got {body.get('namespace')!r}")
    return body


def step_verify_lakekeeper_metadata_directly() -> None:
    """Independent proof, bypassing `lakehouse-api` entirely: ask
    Lakekeeper's own REST catalog for the table and confirm
    `format-version: 2` straight from its metadata — not merely trusting
    `lakehouse-api`'s self-reported `formatVersion` field.

    The Iceberg REST spec's table paths are prefixed by a server-assigned
    `prefix` (Lakekeeper uses the warehouse's internal id, not its human
    name) — resolved here the same way `docker-compose.yml`'s
    `lakekeeper-authz-init` resolves `wh_id`, via the Management API's
    `GET /management/v1/warehouse` list, using the `admin` principal
    (control-plane bypass, per ADR 0011 — this is a metadata lookup, not a
    data read/write). The actual table load below authenticates as
    `gold-export`, the granted principal, not `admin`.
    """
    tokens_dir = os.environ.get("LAKEKEEPER_TOKENS_DIR", "/tokens")
    with open(f"{tokens_dir}/admin.jwt", encoding="utf-8") as f:
        admin_token = f.read().strip()
    with open(f"{tokens_dir}/gold-export.jwt", encoding="utf-8") as f:
        export_token = f.read().strip()

    warehouses = requests.get(
        f"{LAKEKEEPER_BASE_URI}/management/v1/warehouse",
        headers={"Authorization": f"Bearer {admin_token}"},
        timeout=10,
    )
    if not warehouses.ok:
        raise GoldExportFailure(f"Lakekeeper warehouse list failed: {warehouses.status_code} {warehouses.text}")
    wh_id = next(
        (w["id"] for w in warehouses.json().get("warehouses", []) if w.get("name") == LAKEKEEPER_WAREHOUSE),
        None,
    )
    if not wh_id:
        raise GoldExportFailure(f"warehouse {LAKEKEEPER_WAREHOUSE!r} not found: {warehouses.json()}")

    load = requests.get(
        f"{LAKEKEEPER_BASE_URI}/catalog/v1/{wh_id}/namespaces/gold/tables/{MART_NAME}",
        headers={"Authorization": f"Bearer {export_token}"},
        timeout=10,
    )
    if not load.ok:
        raise GoldExportFailure(f"Lakekeeper loadTable failed: {load.status_code} {load.text}")
    metadata = load.json().get("metadata", {})
    format_version = metadata.get("format-version")
    if format_version != 2:
        raise GoldExportFailure(f"Lakekeeper table metadata format-version: wanted 2, got {format_version}")
    print(f"[gold-export] Lakekeeper confirms gold.{MART_NAME} at format-version {format_version}")


def step_read_back_via_api() -> None:
    """`GET /api/gold/export/{mart}` reads the table back through
    `iceberg-rust` (not `ClickHouse`) and reports its row count — the
    round-trip proof."""
    resp = API.get(f"{API_URL}/api/gold/export/{MART_NAME}", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"GET /api/gold/export/{MART_NAME} failed: {resp.status_code} {resp.text}")
    body = resp.json()
    print(f"[gold-export] read-back response: {body}")
    if body.get("rowsInIceberg") != ROW_COUNT:
        raise GoldExportFailure(
            f"rowsInIceberg mismatch: wanted {ROW_COUNT}, got {body.get('rowsInIceberg')}"
        )
    if body.get("formatVersion") != 2:
        raise GoldExportFailure(f"formatVersion mismatch on read-back: got {body.get('formatVersion')}")
    print(f"[gold-export] round trip confirmed: {ROW_COUNT} rows in, {ROW_COUNT} rows read back")


def main() -> int:
    try:
        step_wait_for_services()
        step_login()
        step_seed_gold_mart()
        step_trigger_export()
        step_verify_lakekeeper_metadata_directly()
        step_read_back_via_api()
    except GoldExportFailure as exc:
        print(f"[gold-export] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[gold-export] PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
