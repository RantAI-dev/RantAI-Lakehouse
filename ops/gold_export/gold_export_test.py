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
  5. DATA-5 / ADR 0016 — the OUTSIDE reader: create a service identity
     with the `iceberg:read` scope, rotate it once for its key, and read
     the published table through the API's door (`/api/iceberg`) with
     `pyiceberg` — a real Iceberg REST client, not a hand-rolled request.
     The row count must match what was seeded, a write must be refused
     403, another namespace must be refused 403, and the consumers route
     must list the tool with one read.

Same `g1-test-runner`/`g3a-test-runner` shape: run inside the compose
network (`gold-export-test-runner` in `docker-compose.yml`), plain
`requests` over the API's own HTTP surface — plus `pyiceberg` for the
outside reader, a THIRD-PARTY Iceberg client the API knows nothing
about, which is the point: the door is proven against the client it
exists for. No dependency on the Rust/Python application code it
verifies.

The outside-reading steps need `OUTSIDE_S3_ENDPOINT` on `lakehouse-api`
(the published storage address). The runner is INSIDE the compose
network, so the published address is the in-network one
(`http://rustfs:9000`); the endpoint REWRITE itself is covered by the
Rust unit test (`routes::iceberg_door`'s `rewrite_endpoint`).
"""

from __future__ import annotations

import os
import random
import sys
import time
import uuid

import requests
import boto3
import pyarrow as pa
from pyiceberg.catalog import load_catalog
from pyiceberg.exceptions import ForbiddenError

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
RUSTFS_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
class GoldExportFailure(Exception):
    pass


# DATA-2 review BLOCKER 2: credentials passed via env (no default in docker-compose.yml)
S3_ACCESS_KEY = os.environ.get("CONNECTOR_S3_ACCESS_KEY", "").strip()
S3_SECRET_KEY = os.environ.get("CONNECTOR_S3_SECRET_KEY", "").strip()
if not S3_ACCESS_KEY or not S3_SECRET_KEY:
    raise GoldExportFailure(
        "set CONNECTOR_S3_ACCESS_KEY and CONNECTOR_S3_SECRET_KEY: the gate has no default credential (DATA-2 review BLOCKER 2)"
    )

S3_BUCKET = os.environ.get("LAKEHOUSE_WAREHOUSE_BUCKET", "lakehouse-warehouse")
LAKEKEEPER_BASE_URI = os.environ.get("LAKEKEEPER_BASE_URI", "http://lakekeeper:8181")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
GOLD_SOURCE_SCHEMA = os.environ.get("GOLD_SOURCE_SCHEMA", "serving")
GOLD_EXPORT_RUN_TOKEN = os.environ.get("GOLD_EXPORT_RUN_TOKEN", "")
MART_NAME = os.environ.get("GOLD_MART_NAME", "gold_export_smoke")
ROW_COUNT = int(os.environ.get("GOLD_EXPORT_ROW_COUNT", "7"))
# DATA-5 (ADR 0016): the outside tool's service identity. The name
# carries a random suffix because this test may run more than once
# against the same deployment (a service identity's name is its natural
# key, and a taken name is a 409), and the id — not the name — is what
# the consumers route is checked against.
OUTSIDE_TOOL_NAME = f"gold-export-outside-tool-{uuid.uuid4().hex[:8]}"
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "").strip()
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "").strip()
if not AUTH_EMAIL or not AUTH_PASSWORD:
    raise GoldExportFailure(
        "set AUTH_BOOTSTRAP_EMAIL and AUTH_BOOTSTRAP_PASSWORD: the gate has no default credential (DATA-2 review BLOCKER 2)"
    )

API = requests.Session()
if GOLD_EXPORT_RUN_TOKEN:
    API.headers["x-run-token"] = GOLD_EXPORT_RUN_TOKEN


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


def _load_lakekeeper_metadata(mart_name: str) -> dict:
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
        f"{LAKEKEEPER_BASE_URI}/catalog/v1/{wh_id}/namespaces/gold/tables/{mart_name}",
        headers={"Authorization": f"Bearer {export_token}"},
        timeout=10,
    )
    if not load.ok:
        raise GoldExportFailure(f"Lakekeeper loadTable failed: {load.status_code} {load.text}")
    return load.json().get("metadata", {})


def get_s3_parquet_files(mart_name: str) -> list[str]:
    metadata = _load_lakekeeper_metadata(mart_name)
    location = metadata.get("location", "")
    if not location:
        raise GoldExportFailure(f"missing location in table metadata: {metadata}")
    location_path = location.split("://", 1)[1] if "://" in location else location
    bucket, _, key_prefix = location_path.partition("/")
    bucket_name = bucket or S3_BUCKET
    prefix = f"{key_prefix.strip('/')}/data/"
    s3 = boto3.client(
        "s3",
        endpoint_url=RUSTFS_ENDPOINT,
        aws_access_key_id=S3_ACCESS_KEY,
        aws_secret_access_key=S3_SECRET_KEY,
        region_name="us-east-1",
    )
    paginator = s3.get_paginator("list_objects_v2")
    files: list[str] = []
    for page in paginator.paginate(Bucket=bucket_name, Prefix=prefix):
        for obj in page.get("Contents", []):
            key = obj["Key"]
            if key.endswith(".parquet"):
                files.append(key)
    return sorted(files)


def step_verify_lakekeeper_metadata_directly() -> None:
    """Independent proof, bypassing `lakehouse-api` entirely: ask
    Lakekeeper's own REST catalog for the table and confirm
    `format-version: 2` straight from its metadata — not merely trusting
    `lakehouse-api`'s self-reported `formatVersion` field."""
    metadata = _load_lakekeeper_metadata(MART_NAME)
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


def step_read_back_reports_snapshot_fields() -> None:
    """WS6: `GET /api/gold/export/{mart}` now also reports
    `snapshotId`/`exportedAt` (`routes::gold::read_back`), independent of
    the row count `step_read_back_via_api` already checked."""
    resp = API.get(f"{API_URL}/api/gold/export/{MART_NAME}", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"GET /api/gold/export/{MART_NAME} failed: {resp.status_code} {resp.text}")
    body = resp.json()
    if body.get("snapshotId") is None:
        raise GoldExportFailure(f"expected a non-null snapshotId after a real export, got {body}")
    if not body.get("exportedAt"):
        raise GoldExportFailure(f"expected a non-empty exportedAt after a real export, got {body}")
    print(f"[gold-export] snapshotId={body.get('snapshotId')} exportedAt={body.get('exportedAt')}")


def step_history_lists_this_run() -> None:
    """WS6: `GET /api/gold/exports?mart=` (`routes::gold::exports`, backed
    by `console.gold_export_run`) must list at least the run this test
    itself just triggered, with a `rowsExported` matching what was
    seeded."""
    resp = API.get(f"{API_URL}/api/gold/exports", params={"mart": MART_NAME}, timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"GET /api/gold/exports failed: {resp.status_code} {resp.text}")
    body = resp.json()
    runs = body.get("runs", [])
    if not runs:
        raise GoldExportFailure(f"expected at least one export run recorded, got {body}")
    latest = runs[0]
    if latest.get("status") != "success":
        raise GoldExportFailure(f"expected the latest run to be status=success, got {latest}")
    if latest.get("rowsExported") != ROW_COUNT:
        raise GoldExportFailure(
            f"expected the latest run's rowsExported to be {ROW_COUNT}, got {latest.get('rowsExported')}"
        )
    print(f"[gold-export] history shows {len(runs)} run(s), latest: {latest}")


def step_create_outside_tool() -> tuple[str, str]:
    """DATA-5 (ADR 0016): an outside tool is a service identity with the
    `iceberg:read` scope. Creating it issues no credential (the contract
    has no field for a secret), so the test ROTATES it once and keeps the
    raw key — the one and only time the API hands it out."""
    created = API.post(
        f"{API_URL}/api/identity/service-identities",
        json={
            "name": OUTSIDE_TOOL_NAME,
            "scopes": ["iceberg:read"],
            "environment": "production",
        },
        timeout=30,
    )
    if created.status_code != 201:
        raise GoldExportFailure(
            f"creating the outside tool's service identity failed: "
            f"{created.status_code} {created.text}"
        )
    identity_id = created.json().get("id")
    if not identity_id:
        raise GoldExportFailure(
            f"the created service identity carries no id: {created.json()}"
        )

    rotated = API.post(
        f"{API_URL}/api/identity/service-identities/{identity_id}/rotate",
        timeout=30,
    )
    if not rotated.ok:
        raise GoldExportFailure(
            f"rotating the outside tool's credential failed: "
            f"{rotated.status_code} {rotated.text}"
        )
    key = rotated.json().get("secret")
    if not key:
        raise GoldExportFailure(
            f"the rotation returned no key: {rotated.json()}"
        )
    print(
        f"[gold-export] outside tool {OUTSIDE_TOOL_NAME} ({identity_id}) "
        "has its key (iceberg:read)"
    )
    return identity_id, key


def _outside_catalog(key: str):
    """The outside tool's view of the catalog: the API's DOOR
    (`/api/iceberg`), never Lakekeeper's own port.

    `pyiceberg`'s REST catalog is a real Iceberg client, so this step
    proves that a standard client can read a published Gold table through
    the API — the thing ADR 0016 exists for. The warehouse is passed by
    NAME; the door answers the client's config call with the warehouse's
    own prefix (its id), which the client then uses in every path.
    """
    return load_catalog(
        "rest",
        uri=f"{API_URL}/api/iceberg",
        token=key,
        warehouse=LAKEKEEPER_WAREHOUSE,
    )


def step_outside_tool_reads_the_published_table(key: str) -> None:
    """The outside tool reads the published table through the door and
    sees exactly the rows that were seeded — the whole point of DATA-5:
    outside reading works, and the rows are the same ones the console
    exports."""
    catalog = _outside_catalog(key)
    table = catalog.load_table(("gold", MART_NAME))
    rows = table.scan().to_arrow().num_rows
    if rows != ROW_COUNT:
        raise GoldExportFailure(
            f"the outside tool read {rows} rows through the door, "
            f"expected {ROW_COUNT}"
        )
    print(
        f"[gold-export] the outside tool read {rows} rows of gold.{MART_NAME} "
        "through the door"
    )


def step_the_door_refuses_a_write_and_another_namespace(key: str) -> None:
    """The door reads: a write is refused 403, and a table in another
    namespace is refused 403 (ADR 0016 point 2). Neither reaches the
    catalog."""
    catalog = _outside_catalog(key)
    schema = pa.schema([pa.field("region", pa.string(), nullable=False)])

    try:
        catalog.create_table(("gold", f"{MART_NAME}_written_by_a_tool"), schema)
    except ForbiddenError as exc:
        print(f"[gold-export] the door refused a write: {exc}")
    else:
        raise GoldExportFailure(
            "the door accepted a create_table in gold: a write must be refused"
        )

    try:
        catalog.load_table(("bronze", MART_NAME))
    except ForbiddenError as exc:
        print(f"[gold-export] the door refused another namespace: {exc}")
    else:
        raise GoldExportFailure(
            "the door served a table outside gold: only the gold namespace is served"
        )


def step_consumers_route_lists_the_outside_tool(identity_id: str) -> None:
    """DATA-5: the count is the API's own. The consumers route lists the
    tool that read the published table, with its reads and its last read.

    The not-set-up case ("Not measured", never a count of zero) is
    covered by the Rust integration test
    (`tests/iceberg_door.rs`'s
    `the_gold_consumers_route_reports_not_measured_when_outside_reading_is_off`):
    this stack has outside reading ON, which is the only state in which
    there is a count to list."""
    resp = API.get(f"{API_URL}/api/gold/export/{MART_NAME}/consumers", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"GET .../consumers failed: {resp.status_code} {resp.text}")
    body = resp.json()
    if body.get("supported") is not True:
        raise GoldExportFailure(f"expected supported: true, got {body}")
    if body.get("windowDays") != 7:
        raise GoldExportFailure(f"expected windowDays: 7, got {body}")
    consumers = body.get("consumers") or []
    mine = [c for c in consumers if c.get("identityId") == identity_id]
    if len(mine) != 1:
        raise GoldExportFailure(
            f"expected exactly one entry for the outside tool {identity_id}, "
            f"got {consumers}"
        )
    entry = mine[0]
    if entry.get("reads") != 1:
        raise GoldExportFailure(f"expected reads == 1, got {entry}")
    if not entry.get("name"):
        raise GoldExportFailure(f"expected the tool's name, got {entry}")
    if not entry.get("lastReadAt"):
        raise GoldExportFailure(f"expected the tool's last read, got {entry}")
    print(f"[gold-export] the consumers route lists the outside tool: {entry}")


def step_optimize_final_with_if_changed_skips() -> None:
    """DATA-10: OPTIMIZE TABLE FINAL changes modification_time on MergeTree
    parts but does not change table contents. Calling export with ?ifChanged=true
    must skip."""
    ch_query(f"OPTIMIZE TABLE {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` FINAL")
    resp = API.post(f"{API_URL}/api/gold/export/{MART_NAME}?ifChanged=true", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"POST ?ifChanged=true after OPTIMIZE failed: {resp.status_code} {resp.text}")
    body = resp.json()
    print(f"[gold-export] ifChanged after OPTIMIZE FINAL: {body}")
    if body.get("skipped") is not True:
        raise GoldExportFailure(f"expected skipped: true after OPTIMIZE FINAL, got {body}")


def step_exchange_tables_same_rows_with_if_changed_skips() -> None:
    """DATA-10: Rewriting the mart with identical rows via staging and EXCHANGE TABLES
    changes all part files and modification times, but content fingerprint remains identical.
    Calling export with ?ifChanged=true must skip."""
    staging_mart = f"{MART_NAME}_staging"
    ch_query(f"DROP TABLE IF EXISTS {GOLD_SOURCE_SCHEMA}.`{staging_mart}`")
    ch_query(
        f"CREATE TABLE {GOLD_SOURCE_SCHEMA}.`{staging_mart}` AS {GOLD_SOURCE_SCHEMA}.`{MART_NAME}`"
    )
    ch_query(
        f"INSERT INTO {GOLD_SOURCE_SCHEMA}.`{staging_mart}` "
        f"SELECT * FROM {GOLD_SOURCE_SCHEMA}.`{MART_NAME}`"
    )
    ch_query(
        f"EXCHANGE TABLES {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` AND {GOLD_SOURCE_SCHEMA}.`{staging_mart}`"
    )
    ch_query(f"DROP TABLE IF EXISTS {GOLD_SOURCE_SCHEMA}.`{staging_mart}`")

    resp = API.post(f"{API_URL}/api/gold/export/{MART_NAME}?ifChanged=true", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"POST ?ifChanged=true after EXCHANGE TABLES failed: {resp.status_code} {resp.text}")
    body = resp.json()
    print(f"[gold-export] ifChanged after EXCHANGE TABLES: {body}")
    if body.get("skipped") is not True:
        raise GoldExportFailure(f"expected skipped: true after EXCHANGE TABLES, got {body}")


def step_insert_row_with_if_changed_exports() -> None:
    """DATA-10: Inserting a single new row changes the content fingerprint.
    Calling export with ?ifChanged=true must export instead of skipping."""
    ch_query(
        f"INSERT INTO {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` (region, total_sales, order_count, updated_at) "
        "VALUES ('region-new-row', 123.45, 99, now())"
    )
    resp = API.post(f"{API_URL}/api/gold/export/{MART_NAME}?ifChanged=true", timeout=30)
    if not resp.ok:
        raise GoldExportFailure(f"POST ?ifChanged=true after insert failed: {resp.status_code} {resp.text}")
    body = resp.json()
    print(f"[gold-export] ifChanged after insert row: {body}")
    if body.get("skipped") is True:
        raise GoldExportFailure(f"expected export after INSERT, but was skipped: {body}")
    expected_rows = ROW_COUNT + 1
    if body.get("rowsExported") != expected_rows:
        raise GoldExportFailure(
            f"rowsExported mismatch after insert: wanted {expected_rows}, got {body.get('rowsExported')}"
        )


def step_job_export_9_copies_and_trim_to_7() -> None:
    """DATA-2 (T5): publish 9 changed copies through the job path:
    1. Wait for Dagster webserver.
    2. Enable publication for MART_NAME via PUT /api/gold/export/{mart}/publication.
    3. In 9 cycles, mutate mart rows in ClickHouse, trigger gold_export_job via
       POST /api/pipelines/gold_export_job/trigger, and poll Dagster GraphQL DAGSTER_URL
       until run status is SUCCESS.
    4. Assertions after 9 cycles:
       - 7 copies remain (the newest 7).
       - Query S3 object store: assert exactly 7 parquet files remain and no removed
         copies' files exist.
       - Assert publication detail GET /api/gold/export/{mart}/publication returns trim
         with copiesKept: 7, status: "success", and non-null oldestKeptExportedAt.
    """
    print("[gold-export] waiting for Dagster webserver...")
    _wait_for(
        "dagster-webserver",
        lambda: requests.get(DAGSTER_URL.replace("/graphql", "/server_info"), timeout=5).status_code == 200,
        60,
    )

    # 1. Enable publication for MART_NAME
    pub_resp = API.put(
        f"{API_URL}/api/gold/export/{MART_NAME}/publication",
        json={"enabled": True},
        timeout=10,
    )
    if not pub_resp.ok:
        raise GoldExportFailure(
            f"PUT /api/gold/export/{MART_NAME}/publication failed: {pub_resp.status_code} {pub_resp.text}"
        )
    pub_body = pub_resp.json()
    if pub_body.get("enabled") is not True:
        raise GoldExportFailure(f"expected publication enabled: true, got {pub_body}")
    print(f"[gold-export] publication enabled for {MART_NAME}")

    # Track baseline files and cycle-added files
    baseline_files = set(get_s3_parquet_files(MART_NAME))
    cycle_files: dict[int, str] = {}
    known_files = set(baseline_files)

    query = (
        "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename "
        "... on Run { status } } }"
    )

    for cycle in range(1, 10):
        # Mutate mart rows in ClickHouse so ifChanged exports a new copy
        ch_query(
            f"INSERT INTO {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` "
            f"(region, total_sales, order_count, updated_at) "
            f"VALUES ('region-job-c{cycle}', {2000.0 + cycle}, {cycle * 5}, now())"
        )

        # Trigger gold_export_job via lakehouse-api
        trig_resp = API.post(f"{API_URL}/api/pipelines/gold_export_job/trigger", timeout=10)
        if not trig_resp.ok:
            raise GoldExportFailure(
                f"POST /api/pipelines/gold_export_job/trigger failed (cycle {cycle}): "
                f"{trig_resp.status_code} {trig_resp.text}"
            )
        run_id = trig_resp.json().get("id")
        if not run_id:
            raise GoldExportFailure(f"missing run id in trigger response: {trig_resp.json()}")

        def check_run() -> bool:
            r = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
            r.raise_for_status()
            st = r.json().get("data", {}).get("pipelineRunOrError", {}).get("status")
            print(f"[gold-export] cycle {cycle} run {run_id} status={st}")
            if st == "FAILURE":
                raise GoldExportFailure(f"cycle {cycle} run {run_id} FAILED")
            return st == "SUCCESS"

        _wait_for(f"gold_export_job cycle {cycle} run {run_id} SUCCESS", check_run, 90, interval_s=2.0)

        current_files = get_s3_parquet_files(MART_NAME)
        new_files = [f for f in current_files if f not in known_files]
        if new_files:
            cycle_files[cycle] = new_files[-1]
            known_files.add(new_files[-1])
        print(f"[gold-export] cycle {cycle} done: {len(current_files)} parquet files present in S3")

    # Assertions after 9 cycles:
    final_files = get_s3_parquet_files(MART_NAME)
    print(f"[gold-export] final parquet files ({len(final_files)}): {final_files}")
    if len(final_files) != 7:
        raise GoldExportFailure(
            f"expected exactly 7 parquet files in S3 after 9 cycles and trim, got {len(final_files)}: {final_files}"
        )

    # Prove removed copies' files no longer exist in S3
    for f in baseline_files:
        if f in final_files:
            raise GoldExportFailure(f"baseline parquet file {f} was not removed from S3!")
    if 1 in cycle_files:
        c1_file = cycle_files[1]
        if c1_file in final_files:
            raise GoldExportFailure(f"cycle 1 parquet file {c1_file} was not removed from S3!")
        print(f"[gold-export] verified cycle 1 file {c1_file} is absent from S3")
    if 2 in cycle_files:
        c2_file = cycle_files[2]
        if c2_file in final_files:
            raise GoldExportFailure(f"cycle 2 parquet file {c2_file} was not removed from S3!")
        print(f"[gold-export] verified cycle 2 file {c2_file} is absent from S3")

    # Assert 7 copies remain via ClickHouse DataLakeCatalog query
    ch_copies_raw = ch_query(
        f"SELECT count(DISTINCT _exported_at) FROM icecat_gold_export.`gold.{MART_NAME}` WHERE 1 FORMAT TabSeparated"
    ).strip()
    if int(ch_copies_raw) != 7:
        raise GoldExportFailure(
            f"expected 7 distinct copies in gold.{MART_NAME}, got {ch_copies_raw}"
        )
    print(f"[gold-export] confirmed 7 distinct copies in icecat_gold_export.gold.{MART_NAME}")

    # Assert publication detail GET /api/gold/export/{mart}/publication
    detail_resp = API.get(f"{API_URL}/api/gold/export/{MART_NAME}/publication", timeout=10)
    if not detail_resp.ok:
        raise GoldExportFailure(
            f"GET /api/gold/export/{MART_NAME}/publication failed: {detail_resp.status_code} {detail_resp.text}"
        )
    detail = detail_resp.json()
    print(f"[gold-export] publication detail: {detail}")
    trim = detail.get("trim")
    if not trim:
        raise GoldExportFailure(f"expected trim object in publication detail, got null/missing in: {detail}")
    if trim.get("copiesKept") != 7:
        raise GoldExportFailure(f"expected trim.copiesKept == 7, got {trim.get('copiesKept')}")
    if trim.get("status") != "success":
        raise GoldExportFailure(f"expected trim.status == 'success', got {trim.get('status')}")
    if not trim.get("oldestKeptExportedAt"):
        raise GoldExportFailure(f"expected non-null trim.oldestKeptExportedAt, got {trim.get('oldestKeptExportedAt')}")
    print(
        f"[gold-export] publication detail confirms trim: copiesKept={trim.get('copiesKept')}, "
        f"oldestKeptExportedAt={trim.get('oldestKeptExportedAt')}, status={trim.get('status')}"
    )


def main() -> int:
    try:
        step_wait_for_services()
        step_login()
        step_seed_gold_mart()
        step_trigger_export()
        step_verify_lakekeeper_metadata_directly()
        step_read_back_via_api()
        step_read_back_reports_snapshot_fields()
        step_history_lists_this_run()
        # DATA-5 (ADR 0016): the outside reader. Before the DATA-10
        # steps, so the published table still holds exactly the seeded
        # rows and the read count is exactly one.
        tool_id, tool_key = step_create_outside_tool()
        step_outside_tool_reads_the_published_table(tool_key)
        step_the_door_refuses_a_write_and_another_namespace(tool_key)
        step_consumers_route_lists_the_outside_tool(tool_id)
        step_optimize_final_with_if_changed_skips()
        step_exchange_tables_same_rows_with_if_changed_skips()
        step_insert_row_with_if_changed_exports()
        step_job_export_9_copies_and_trim_to_7()
    except GoldExportFailure as exc:
        print(f"[gold-export] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[gold-export] PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
