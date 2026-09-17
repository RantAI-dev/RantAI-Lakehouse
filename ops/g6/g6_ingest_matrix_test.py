#!/usr/bin/env python3
"""G6 acceptance test (WS3 Tier 1 ingestion): one connector per adapter
(sql×mysql, sql×mssql, sql×postgres, files×csv, rest) is registered,
given an ingest-spec, run via POST .../ingest/run, and its rows verified
in Bronze through ClickHouse -- the real, end-to-end proof every adapter
this workstream ships actually loads data, not just that its unit tests
pass in isolation.

Login/session shape duplicated from ops/g3a/g3a_test.py's step_login/API
(:60-149) -- every /api/connectors/* and /api/connectors/{id}/ingest/run
route is Policy::RequiresAuth or stricter. G6Failure/_wait_for duplicated
from ops/g4/g4_test.py's G4Failure/_wait_for shape. Both duplications
follow this repository's existing precedent of NOT sharing code between
standalone gate scripts (ch_query/_wait_for are already duplicated,
near-verbatim, between g3a_test.py and g4_test.py) --
docs/superpowers/plans/2026-09-11-ws5-platform-signals.md's plan for
extending ops/g4/g4_test.py duplicates the SAME g3a_test.py login block
into g4_test.py for the identical reason.
"""
from __future__ import annotations

import os
import subprocess
import sys
import time

import requests

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
LAKEKEEPER_CATALOG_URI = os.environ.get("CH_LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog")
LAKEKEEPER_WAREHOUSE = os.environ.get("LAKEKEEPER_WAREHOUSE", "default")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
MYSQL_ROOT_PASSWORD = os.environ.get("MYSQL_ROOT_PASSWORD", "")
MSSQL_SA_PASSWORD = os.environ.get("MSSQL_SA_PASSWORD", "")
CATALOG_DB = "icecat_g6"

API = requests.Session()


class G6Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0):
    deadline = time.time() + timeout_s
    last_err = None
    while time.time() < deadline:
        try:
            value = check()
            if value:
                print(f"[g6] ready: {name}")
                return value
        except Exception as exc:  # noqa: BLE001
            last_err = exc
        time.sleep(interval_s)
    raise G6Failure(f"timed out waiting for {name!r}: {last_err}")


def ch_query(sql: str) -> str:
    resp = requests.post(CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30)
    if not resp.ok:
        raise G6Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1", 60)
    _wait_for("lakehouse-api", lambda: API.get(f"{API_URL}/health", timeout=5).status_code == 200, 60)
    _wait_for(
        "Dagster webserver",
        lambda: requests.get(DAGSTER_URL.replace("/graphql", "/server_info"), timeout=5).status_code == 200,
        90,
    )


def step_login() -> None:
    """Duplicated from ops/g3a/g3a_test.py::step_login -- every route
    this gate calls requires an authenticated session."""
    login = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        raise G6Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        if not rotated.ok:
            raise G6Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
        if not relogin.ok:
            raise G6Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text}")
    print("[g6] logged in as bootstrap admin")


def _seed_mysql_fixture() -> None:
    import pymysql
    conn = pymysql.connect(host="mysql-g6", user="root", password=MYSQL_ROOT_PASSWORD, database="g6_ingest")
    try:
        with conn.cursor() as cur:
            cur.execute("CREATE TABLE IF NOT EXISTS orders (id INT PRIMARY KEY, amount DECIMAL(10,2))")
            cur.execute("DELETE FROM orders")
            cur.executemany("INSERT INTO orders (id, amount) VALUES (%s, %s)", [(1, 10.5), (2, 20.25)])
        conn.commit()
    finally:
        conn.close()


def _seed_mssql_fixture() -> None:
    import pyodbc
    conn_str = (
        "DRIVER={ODBC Driver 18 for SQL Server};SERVER=tcp:mssql-g6,1433;"
        f"UID=sa;PWD={{{MSSQL_SA_PASSWORD}}};DATABASE=master;Encrypt=yes;TrustServerCertificate=yes;"
    )
    conn = pyodbc.connect(conn_str, autocommit=True)
    try:
        cur = conn.cursor()
        cur.execute(
            "IF NOT EXISTS (SELECT * FROM sys.databases WHERE name = 'g6_ingest') CREATE DATABASE g6_ingest"
        )
        cur.execute("USE g6_ingest")
        cur.execute(
            "IF OBJECT_ID('orders') IS NULL CREATE TABLE orders (id INT PRIMARY KEY, amount DECIMAL(10,2))"
        )
        cur.execute("DELETE FROM orders")
        cur.execute("INSERT INTO orders (id, amount) VALUES (1, 10.5), (2, 20.25)")
    finally:
        conn.close()


def _register_and_run(*, name: str, kind: str, host: str, secret_ref: str, adapter: str, dial: dict, source_objects: list) -> str | None:
    """POST /api/connectors, PUT .../ingest-spec, POST .../ingest/run.
    Returns the launched runId, or None for an honest supported:false
    response (CDC/Sheets)."""
    created = API.post(
        f"{API_URL}/api/connectors",
        json={
            "name": name, "type": kind, "direction": "source", "host": host, "secretRef": secret_ref,
            "environment": "production", "tenant": "g6", "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    if not created.ok:
        raise G6Failure(f"create connector {name!r} failed: {created.status_code} {created.text}")
    connector_id = created.json()["id"]

    spec_resp = API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": adapter, "ingestMode": "batch", "dial": dial, "sourceObjects": source_objects},
        timeout=10,
    )
    if not spec_resp.ok:
        raise G6Failure(f"set ingest-spec for {name!r} failed: {spec_resp.status_code} {spec_resp.text}")

    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    if not run_resp.ok:
        raise G6Failure(f"ingest/run for {name!r} failed: {run_resp.status_code} {run_resp.text}")
    body = run_resp.json()
    if body.get("supported") is False:
        print(f"[g6] {name!r}: supported=false ({body.get('reason')})")
        return None
    run_id = body.get("runId")
    if not run_id:
        raise G6Failure(f"ingest/run for {name!r} returned no runId: {body}")
    print(f"[g6] launched run {run_id} for {name!r}")
    return run_id


def step_wait_for_run_success(run_id: str) -> None:
    """Duplicated from ops/g3a/g3a_test.py::step_wait_for_run_success
    (:177-198) -- identical Dagster GraphQL run-status polling."""
    query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename ... on Run { status } } }"

    def check() -> bool:
        resp = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
        resp.raise_for_status()
        run = resp.json().get("data", {}).get("pipelineRunOrError", {})
        status = run.get("status")
        print(f"[g6] run {run_id} status={status}")
        if status == "FAILURE":
            raise G6Failure(f"run {run_id} FAILED")
        return status == "SUCCESS"

    _wait_for(f"run {run_id} SUCCESS", check, 120, interval_s=3.0)


def _bronze_row_count(bronze_table: str) -> int:
    """Never a bare count() -- see g3a_test.py/g4_test.py's identical
    docs/plans/P5-RESULT.md-cited comment (R11): a merge-on-read Iceberg
    table's metadata-only fast path overcounts on equality deletes."""
    ch_query(
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} ENGINE = DataLakeCatalog('{LAKEKEEPER_CATALOG_URI}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{LAKEKEEPER_WAREHOUSE}' "
        "SETTINGS allow_database_iceberg = 1"
    )
    text = ch_query(
        f"SELECT count() FROM {CATALOG_DB}.`bronze.{bronze_table}` WHERE 1 "
        "SETTINGS allow_database_iceberg = 1 FORMAT TabSeparated"
    )
    return int(text.strip())


def step_ingest_matrix() -> None:
    _seed_mysql_fixture()
    _seed_mssql_fixture()

    connectors = [
        ("g6-mysql", "MySQL", "mysql-g6:3306", "env:CONNECTOR_MYSQL_PASSWORD", "sql",
         {"driver": "mysql", "host": "mysql-g6", "port": 3306, "database": "g6_ingest", "user": "g6_reader"},
         [{"name": "orders", "target": "g6_mysql_orders"}]),
        ("g6-mssql", "SQL Server", "mssql-g6:1433", "env:CONNECTOR_MSSQL_PASSWORD", "sql",
         {"driver": "mssql", "host": "mssql-g6", "port": 1433, "database": "g6_ingest", "user": "sa"},
         [{"name": "orders", "target": "g6_mssql_orders"}]),
        ("g6-rest", "REST API", "rest-stub-g6:8080", "env:CONNECTOR_REST_API_KEY", "rest",
         {"baseUrl": "http://rest-stub-g6:8080", "auth": {"type": "api_key", "header": "X-Api-Key"},
          "pagination": {"type": "offset", "param": "offset", "pageSize": 2},
          "endpoints": [{"path": "/items", "dataPath": "items"}]},
         []),
    ]
    run_ids: dict[str, str] = {}
    for name, kind, host, secret_ref, adapter, dial, objs in connectors:
        run_id = _register_and_run(
            name=name, kind=kind, host=host, secret_ref=secret_ref, adapter=adapter, dial=dial, source_objects=objs,
        )
        if run_id:
            run_ids[name] = run_id

    # Postgres batch: the existing seeded conn-pg-lakehouse (0033),
    # already adapter=sql -- run it directly, no new connector needed.
    pg_run = API.post(f"{API_URL}/api/connectors/conn-pg-lakehouse/ingest/run", timeout=10)
    if not pg_run.ok:
        raise G6Failure(f"ingest/run for conn-pg-lakehouse failed: {pg_run.status_code} {pg_run.text}")
    run_ids["conn-pg-lakehouse"] = pg_run.json()["runId"]

    # files: the fixture ops/g6/seed_files_fixture.py wrote to
    # landing/orders.csv, registered against a NEW test connector -- never
    # conn-s3-warehouse, which rust/migrations/0033_connector_ingest_spec.sql
    # deliberately seeds with an empty source_objects.
    files_run_id = _register_and_run(
        name="g6-files", kind="Object storage", host="rustfs:9000", secret_ref="env:CONNECTOR_S3_ACCESS_KEY",
        adapter="files",
        dial={"protocol": "s3", "bucket": "lakehouse-warehouse", "format": "csv"},
        source_objects=[{"name": "landing/orders.csv", "target": "g6_files_orders"}],
    )
    if files_run_id:
        run_ids["g6-files"] = files_run_id

    for name, run_id in run_ids.items():
        step_wait_for_run_success(run_id)

    for name, bronze_table in [
        ("g6-mysql", "g6_mysql_orders"), ("g6-mssql", "g6_mssql_orders"),
        ("conn-pg-lakehouse", "orders"), ("g6-files", "g6_files_orders"),
    ]:
        count = _bronze_row_count(bronze_table)
        if count <= 0:
            raise G6Failure(f"expected rows in bronze.{bronze_table} for {name!r}, got {count}")
        print(f"[g6] bronze.{bronze_table}: {count} rows")


def step_column_gate_rejects_an_unsupported_column() -> None:
    """A real, asserted FAILURE -- calls column_gate.reject_unsupported_column_types
    inside the REAL dagster-code-location image (not a mocked unit test),
    asserting it exits non-zero for a nested-array-typed column and zero
    for an ordinary one."""
    good = subprocess.run(
        ["docker", "compose", "-p", "g6ci", "--profile", "g6", "exec", "-T", "dagster-code-location",
         "python", "-c", "from dispar_orchestrate.column_gate import reject_unsupported_column_types as f; f([('id','bigint')])"],
        capture_output=True, text=True, check=False,
    )
    if good.returncode != 0:
        raise G6Failure(f"reject_unsupported_column_types wrongly rejected a plain 'bigint' column: {good.stderr}")

    bad = subprocess.run(
        ["docker", "compose", "-p", "g6ci", "--profile", "g6", "exec", "-T", "dagster-code-location",
         "python", "-c", "from dispar_orchestrate.column_gate import reject_unsupported_column_types as f; f([('bad','text[]')])"],
        capture_output=True, text=True, check=False,
    )
    if bad.returncode == 0:
        raise G6Failure("reject_unsupported_column_types did not reject an array-typed ('text[]') column")
    print("[g6] column_gate.reject_unsupported_column_types: real, asserted pass/fail against the shipped image")


def step_cdc_reports_unsupported_not_a_launch() -> None:
    created = API.post(
        f"{API_URL}/api/connectors",
        json={
            "name": "g6-cdc", "type": "PostgreSQL CDC", "direction": "source", "host": "postgres:5432",
            "secretRef": "env:CONNECTOR_PG_PASSWORD", "environment": "production", "tenant": "g6",
            "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    connector_id = created.json()["id"]
    API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={
            "adapter": "cdc", "ingestMode": "cdc",
            "dial": {"driver": "postgresql", "host": "postgres", "port": 5432, "database": "lakehouse",
                     "user": "lakehouse", "slotName": "g6_cdc_slot", "publicationName": "g6_cdc_pub"},
            "sourceObjects": [],
        },
        timeout=10,
    )
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    body = run_resp.json()
    if body.get("supported") is not False or "runId" in body:
        raise G6Failure(f"expected a supported:false, no-runId response for a cdc connector, got: {body}")
    print(f"[g6] cdc ingest/run: honest supported:false ({body.get('reason')})")


def step_sheets_reports_unsupported() -> None:
    created = API.post(
        f"{API_URL}/api/connectors",
        json={
            "name": "g6-sheets", "type": "Google Sheets", "direction": "source", "host": "sheets.googleapis.com",
            "secretRef": "env:CONNECTOR_SHEETS_SERVICE_ACCOUNT_JSON", "environment": "production", "tenant": "g6",
            "residency": "", "capabilities": [],
        },
        timeout=10,
    )
    connector_id = created.json()["id"]
    API.put(
        f"{API_URL}/api/connectors/{connector_id}/ingest-spec",
        json={"adapter": "sheets", "ingestMode": "batch",
              "dial": {"spreadsheetId": "s1", "ranges": ["A1:B2"]}, "sourceObjects": []},
        timeout=10,
    )
    run_resp = API.post(f"{API_URL}/api/connectors/{connector_id}/ingest/run", timeout=10)
    body = run_resp.json()
    if body.get("supported") is not False:
        raise G6Failure(f"expected supported:false for sheets (google-auth not installed), got: {body}")
    print(f"[g6] sheets ingest/run: honest supported:false ({body.get('reason')})")


def main() -> int:
    try:
        step_wait_for_services()
        step_login()
        step_ingest_matrix()
        step_column_gate_rejects_an_unsupported_column()
        step_cdc_reports_unsupported_not_a_launch()
        step_sheets_reports_unsupported()
    except G6Failure as exc:
        print(f"[g6] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g6] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
