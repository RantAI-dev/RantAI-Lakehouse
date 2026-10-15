#!/usr/bin/env python3
"""DATA-2 T1b spike: test removing orphan files via ClickHouse on Gold Iceberg table."""

from __future__ import annotations

import datetime
import os
import random
import sys
import time
from typing import Any

import pyarrow.fs as pafs
from pyiceberg.catalog.rest import RestCatalog
from pyiceberg.expressions import LessThan
import requests

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
LAKEKEEPER_BASE_URI = os.environ.get("LAKEKEEPER_BASE_URI", "http://lakekeeper:8181")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
RUSTFS_ENDPOINT = os.environ.get("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000")
S3_ACCESS_KEY = os.environ.get("CONNECTOR_S3_ACCESS_KEY", "rustfsadmin")
S3_SECRET_KEY = os.environ.get("CONNECTOR_S3_SECRET_KEY", "rustfsadmin")
S3_BUCKET = os.environ.get("LAKEHOUSE_WAREHOUSE_BUCKET", "lakehouse-warehouse")
GOLD_SOURCE_SCHEMA = os.environ.get("GOLD_SOURCE_SCHEMA", "serving")
MART_NAME = "gold_export_spike"
GOLD_EXPORT_RUN_TOKEN = os.environ.get("GOLD_EXPORT_RUN_TOKEN", "ci-gold-export-token-not-a-real-secret")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")

TOKEN_FILE = "/tokens/gold-export.jwt"

API = requests.Session()
API.headers["x-run-token"] = GOLD_EXPORT_RUN_TOKEN


def ch_query(sql: str) -> str:
    resp = requests.post(
        CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=60
    )
    if not resp.ok:
        raise RuntimeError(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def parse_tsv_kv(text: str) -> dict[str, int]:
    counts: dict[str, int] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        key, _, value = line.partition("\t")
        try:
            counts[key] = int(value)
        except ValueError:
            pass
    return counts


def wait_for(name: str, check, timeout_s: int = 60, interval_s: float = 2.0) -> None:
    deadline = time.time() + timeout_s
    last_err = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[t1b] ready: {name}")
                return
        except Exception as exc:
            last_err = exc
        time.sleep(interval_s)
    raise RuntimeError(f"Timed out waiting for {name}: {last_err}")


def list_s3_parquet_files(s3_fs: pafs.S3FileSystem, table_location: str) -> list[str]:
    # table_location is like s3://lakehouse-warehouse/<uuid>
    path = table_location.replace("s3://", "")
    selector = pafs.FileSelector(path, recursive=True)
    infos = s3_fs.get_file_info(selector)
    return sorted(
        info.path
        for info in infos
        if info.type == pafs.FileType.File and info.path.endswith(".parquet")
    )


def main() -> int:
    print("[t1b] Starting T1b spike...")
    wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1")
    wait_for("lakehouse-api", lambda: API.get(f"{API_URL}/health", timeout=5).status_code == 200)
    wait_for(
        "Lakekeeper",
        lambda: requests.get(f"{LAKEKEEPER_BASE_URI}/management/v1/info", timeout=5).status_code
        in (200, 401, 403),
    )

    # 1. Login with forced password rotation handling
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise RuntimeError(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password",
            json={"newPassword": AUTH_PASSWORD},
            timeout=10,
        )
        if not rotated.ok:
            raise RuntimeError(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise RuntimeError(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[t1b] Logged in to lakehouse-api successfully.")

    # 2. Seed ClickHouse table serving.gold_export_spike
    ch_query(f"CREATE DATABASE IF NOT EXISTS {GOLD_SOURCE_SCHEMA}")
    ch_query(f"DROP TABLE IF EXISTS {GOLD_SOURCE_SCHEMA}.`{MART_NAME}`")
    ch_query(
        f"CREATE TABLE {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` ("
        "region String, total_sales Float64, order_count UInt64, "
        "updated_at DateTime"
        ") ENGINE = MergeTree ORDER BY region"
    )
    rows = []
    for i in range(7):
        region = f"region-{i}"
        total_sales = round(random.uniform(100.0, 9999.0), 2)
        order_count = random.randint(1, 500)
        rows.append(f"('{region}', {total_sales}, {order_count}, now())")
    ch_query(
        f"INSERT INTO {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` "
        f"(region, total_sales, order_count, updated_at) VALUES {', '.join(rows)}"
    )
    print("[t1b] Seeded 7 rows in serving.gold_export_spike")

    # 3. Perform 9 export cycles, modifying data between cycles
    for cycle in range(1, 10):
        if cycle > 1:
            # Insert a new row to change content fingerprint
            ch_query(
                f"INSERT INTO {GOLD_SOURCE_SCHEMA}.`{MART_NAME}` "
                f"(region, total_sales, order_count, updated_at) "
                f"VALUES ('region-cycle-{cycle}', {cycle * 111.1}, {cycle * 10}, now())"
            )
        resp = API.post(f"{API_URL}/api/gold/export/{MART_NAME}", timeout=30)
        if not resp.ok:
            raise RuntimeError(f"Export cycle {cycle} failed: {resp.status_code} {resp.text}")
        body = resp.json()
        print(f"[t1b] Export cycle {cycle}: rowsExported={body.get('rowsExported')}")
        time.sleep(1.0)

    # 4. Connect PyIceberg catalog with gold-export token
    with open(TOKEN_FILE, encoding="utf-8") as f:
        gold_token = f.read().strip()

    catalog = RestCatalog(
        name="lakekeeper",
        warehouse=LAKEKEEPER_WAREHOUSE,
        uri=f"{LAKEKEEPER_BASE_URI}/catalog",
        token=gold_token,
        **{
            "s3.endpoint": RUSTFS_ENDPOINT,
            "s3.access-key-id": S3_ACCESS_KEY,
            "s3.secret-access-key": S3_SECRET_KEY,
            "s3.region": "us-east-1",
            "s3.path-style-access": "true",
        },
    )

    table = catalog.load_table(f"gold.{MART_NAME}")
    table_location = table.location()
    print(f"[t1b] Loaded table gold.{MART_NAME} at {table_location}")

    s3_fs = pafs.S3FileSystem(
        endpoint_override=RUSTFS_ENDPOINT,
        access_key=S3_ACCESS_KEY,
        secret_key=S3_SECRET_KEY,
        scheme="http",
    )

    parquet_before = list_s3_parquet_files(s3_fs, table_location)
    print(f"[t1b] BEFORE delete: {len(parquet_before)} parquet files:")
    for p in parquet_before:
        print(f"  - {p}")

    df = table.scan().to_arrow()
    exported_at_col = df["_exported_at"].to_pylist()
    distinct_exported_at = sorted(set(exported_at_col), reverse=True)
    print(f"[t1b] Total rows: {len(df)}, distinct _exported_at count: {len(distinct_exported_at)}")
    for i, ts in enumerate(distinct_exported_at, 1):
        print(f"  Copy {i}: {ts}")

    cutoff_time = distinct_exported_at[6]  # 7th newest (0-indexed 6)
    print(f"[t1b] Cutoff timestamp (7th newest, keep 7): {cutoff_time}")

    print("[t1b] Calling table.delete(LessThan('_exported_at', cutoff_time))...")
    table.delete(LessThan("_exported_at", cutoff_time))

    current_snapshot = table.current_snapshot()
    current_snapshot_id = current_snapshot.snapshot_id if current_snapshot else None
    all_snapshots = table.snapshots()
    older_snapshot_ids = [s.snapshot_id for s in all_snapshots if s.snapshot_id != current_snapshot_id]
    print(f"[t1b] Expiring older snapshots: {older_snapshot_ids}")
    if older_snapshot_ids:
        table.maintenance.expire_snapshots().by_ids(older_snapshot_ids).commit()

    table.refresh()
    df_after = table.scan().to_arrow()
    print(f"[t1b] AFTER PyIceberg delete & expire: rows={len(df_after)}, snapshots={len(table.snapshots())}")

    parquet_after_pyiceberg = list_s3_parquet_files(s3_fs, table_location)
    print(f"[t1b] Parquet files after PyIceberg: {len(parquet_after_pyiceberg)}")

    # 5. Now test ClickHouse remove_orphan_files through DataLakeCatalog
    catalog_db = "icecat_gold_export"
    print(f"[t1b] Creating DataLakeCatalog database {catalog_db} in ClickHouse...")
    ch_query(
        f"""
        CREATE DATABASE IF NOT EXISTS {catalog_db}
        ENGINE = DataLakeCatalog('{LAKEKEEPER_BASE_URI}/catalog')
        SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}',
        storage_endpoint = '{RUSTFS_ENDPOINT}',
        catalog_credential = 'clickhouse-reader:unused',
        oauth_server_uri = 'http://oidc-mock:8090/token'
        SETTINGS allow_database_iceberg = 1
        """
    )

    tables_text = ch_query(f"SHOW TABLES FROM {catalog_db} SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated")
    print(f"[t1b] Tables in {catalog_db}:\n{tables_text.strip()}")

    # 5a. Test default age retention (3 days)
    print("\n[t1b] === Testing remove_orphan_files with default retention (dry_run=1) ===")
    sql_default_dry = (
        f"ALTER TABLE {catalog_db}.`gold.{MART_NAME}` "
        f"EXECUTE remove_orphan_files(dry_run=1) "
        f"SETTINGS allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_iceberg_remove_orphan_files=1 "
        f"FORMAT TabSeparated"
    )
    out_default_dry = ch_query(sql_default_dry)
    counts_default_dry = parse_tsv_kv(out_default_dry)
    print(f"[t1b] Default age dry_run output:\n{out_default_dry.strip()}")
    print(f"[t1b] Default age parsed counts: {counts_default_dry}")

    # 5b. Test setting iceberg_orphan_files_older_than_seconds=0
    print("\n[t1b] === Testing remove_orphan_files with iceberg_orphan_files_older_than_seconds=0 (dry_run=1) ===")
    sql_setting_dry = (
        f"ALTER TABLE {catalog_db}.`gold.{MART_NAME}` "
        f"EXECUTE remove_orphan_files(dry_run=1) "
        f"SETTINGS allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_iceberg_remove_orphan_files=1, "
        f"iceberg_orphan_files_older_than_seconds=0 "
        f"FORMAT TabSeparated"
    )
    out_setting_dry = ch_query(sql_setting_dry)
    counts_setting_dry = parse_tsv_kv(out_setting_dry)
    print(f"[t1b] Setting older_than=0 dry_run output:\n{out_setting_dry.strip()}")
    print(f"[t1b] Setting older_than=0 parsed counts: {counts_setting_dry}")

    # 5c. Test older_than argument syntax
    future_time_str = (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=1)).strftime("%Y-%m-%d %H:%M:%S")
    print(f"\n[t1b] === Testing remove_orphan_files(older_than='{future_time_str}', dry_run=1) ===")
    try:
        sql_arg_dry = (
            f"ALTER TABLE {catalog_db}.`gold.{MART_NAME}` "
            f"EXECUTE remove_orphan_files(older_than='{future_time_str}', dry_run=1) "
            f"SETTINGS allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_iceberg_remove_orphan_files=1 "
            f"FORMAT TabSeparated"
        )
        out_arg_dry = ch_query(sql_arg_dry)
        counts_arg_dry = parse_tsv_kv(out_arg_dry)
        print(f"[t1b] Argument older_than dry_run output:\n{out_arg_dry.strip()}")
        print(f"[t1b] Argument older_than parsed counts: {counts_arg_dry}")
    except Exception as exc:
        print(f"[t1b] Argument older_than failed: {exc}")

    # 5d. Applied run
    print("\n[t1b] === Testing applied run: remove_orphan_files(dry_run=0) ===")
    sql_applied = (
        f"ALTER TABLE {catalog_db}.`gold.{MART_NAME}` "
        f"EXECUTE remove_orphan_files(dry_run=0) "
        f"SETTINGS allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_iceberg_remove_orphan_files=1, "
        f"iceberg_orphan_files_older_than_seconds=0 "
        f"FORMAT TabSeparated"
    )
    out_applied = ch_query(sql_applied)
    counts_applied = parse_tsv_kv(out_applied)
    print(f"[t1b] Applied run output:\n{out_applied.strip()}")
    print(f"[t1b] Applied run parsed counts: {counts_applied}")

    # 6. Verify Parquet files in S3 after applied run
    parquet_after_applied = list_s3_parquet_files(s3_fs, table_location)
    print(f"\n[t1b] Parquet files AFTER applied remove_orphan_files: {len(parquet_after_applied)}")
    for p in parquet_after_applied:
        print(f"  - {p}")

    removed_files = set(parquet_before) - set(parquet_after_applied)
    print(f"[t1b] Removed parquet files count: {len(removed_files)}")
    for p in sorted(removed_files):
        print(f"  Removed: {p}")

    if len(parquet_after_applied) == 7 and len(removed_files) == 2:
        print("\n>>> GATE T1b PASSED: Exactly 2 orphan parquet data files were freed! <<<")
        print(">>> 7 parquet files remain in object store. <<<")
        return 0
    else:
        print(f"\n>>> GATE T1b FAILED: Expected 7 files remaining and 2 removed, got {len(parquet_after_applied)} remaining and {len(removed_files)} removed. <<<")
        return 1


if __name__ == "__main__":
    sys.exit(main())
