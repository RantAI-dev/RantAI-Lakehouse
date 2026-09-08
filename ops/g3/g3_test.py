#!/usr/bin/env python3
"""G3/P4 maintenance-job acceptance test — CI-runnable, functional (not
the interactive performance measurement; see `docs/plans/G3-RESULT.md` for
the one-time synthetic-load / before-after numbers that decided the Trino
escape hatch). This test proves, on every CI run, that:

1. `bronze_maintenance_job` is visible and triggerable through the SAME
   `lakehouse-dagster`-backed pipeline routes G3a already proved work.
2. It runs the measured-working subset of the maintenance chain
   (`remove_orphan_files`, dry-run then applied — `expire_snapshots` is no
   longer supported for Iceberg tables backed by a transactional catalog
   as of ClickHouse 26.8, see `docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`
   and `dagster/dispar_orchestrate/maintenance.py`'s module doc) over a
   real Bronze table without erroring.
3. The resulting `dry_run`/applied metrics are visible through
   `GET /api/governance/maintenance` — the console-facing surface this
   phase adds — proving the metrics-surfacing mechanism, not just the
   Dagster run, actually works end to end.

Same compose-network constraint as `ops/g3a/g3a_test.py` (Lakekeeper
advertises compose-internal hostnames) and the same shape: bind-mounted
script, stock Python base image, run inside the network via the
`g3-maintenance-test-runner` compose service (`dagster` profile).

This test triggers `bronze_ingest_job` (P3) first if no Bronze table is
registered yet — ensuring a table exists for maintenance to act on even
when run standalone (not chained after `g3a-test-runner`), while still
being a no-op re-ingest if `g3a-test-runner` already ran in the same CI
job.
"""

from __future__ import annotations

import os
import sys
import time

import requests

CH_URL = os.environ.get("CH_URL", "http://clickhouse:8123")
CH_USER = os.environ.get("CH_USER", "default")
CH_PASSWORD = os.environ.get("CH_PASSWORD", "")
API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
INGEST_JOB_NAME = os.environ.get("BRONZE_JOB_NAME", "bronze_ingest_job")
MAINTENANCE_JOB_NAME = os.environ.get("MAINTENANCE_JOB_NAME", "bronze_maintenance_job")
BRONZE_TABLE_NAME = os.environ.get("BRONZE_TABLE_NAME", "g3a_orders")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")

API = requests.Session()


class G3Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0) -> None:
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[g3] ready: {name}")
                return
        except Exception as exc:  # noqa: BLE001
            last_err = exc
        time.sleep(interval_s)
    raise G3Failure(f"timed out waiting for {name!r}: {last_err}")


def ch_query(sql: str) -> str:
    resp = requests.post(
        CH_URL, auth=(CH_USER, CH_PASSWORD), data=sql.encode("utf-8"), timeout=30
    )
    if not resp.ok:
        raise G3Failure(f"ClickHouse query failed ({resp.status_code}): {resp.text}\nSQL: {sql}")
    return resp.text


def step_wait_for_services() -> None:
    _wait_for("ClickHouse", lambda: ch_query("SELECT 1 FORMAT TabSeparated").strip() == "1", 60)
    _wait_for(
        "lakehouse-api", lambda: API.get(f"{API_URL}/health", timeout=5).status_code == 200, 60
    )
    _wait_for(
        "Dagster webserver",
        lambda: requests.get(
            DAGSTER_URL.replace("/graphql", "/server_info"), timeout=5
        ).status_code
        == 200,
        90,
    )


def step_login() -> None:
    login = API.post(
        f"{API_URL}/api/auth/login",
        json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
        timeout=10,
    )
    if not login.ok:
        raise G3Failure(f"login failed: {login.status_code} {login.text}")
    if login.json().get("mustChangePassword"):
        rotated = API.post(
            f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10
        )
        if not rotated.ok:
            raise G3Failure(f"password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = API.post(
            f"{API_URL}/api/auth/login",
            json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD},
            timeout=10,
        )
        if not relogin.ok:
            raise G3Failure(f"re-login failed: {relogin.status_code} {relogin.text}")
    print("[g3] logged in as bootstrap admin")


def trigger_and_wait(job_name: str, timeout_s: int) -> str:
    resp = API.post(f"{API_URL}/api/pipelines/{job_name}/trigger", timeout=10)
    if not resp.ok:
        raise G3Failure(f"trigger {job_name} failed: {resp.status_code} {resp.text}")
    run_id = resp.json().get("id") or resp.json().get("runId") or resp.json().get("run_id")
    if not run_id:
        raise G3Failure(f"trigger response had no runId: {resp.json()}")
    print(f"[g3] launched {job_name} run {run_id}")

    query = (
        "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename "
        "... on Run { status } } }"
    )

    def check() -> bool:
        r = requests.post(DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10)
        r.raise_for_status()
        status = r.json().get("data", {}).get("pipelineRunOrError", {}).get("status")
        print(f"[g3] run {run_id} ({job_name}) status={status}")
        if status == "FAILURE":
            raise G3Failure(f"run {run_id} ({job_name}) FAILED")
        return status == "SUCCESS"

    _wait_for(f"{job_name} run {run_id} SUCCESS", check, timeout_s, interval_s=3.0)
    return run_id


def step_ensure_bronze_table_exists() -> None:
    count_text = ch_query(
        "EXISTS TABLE lake.`bronze_meta.dataset_catalog` FORMAT TabSeparated"
    ).strip()
    has_any = False
    if count_text == "1":
        rows = ch_query(
            "SELECT count() FROM lake.`bronze_meta.dataset_catalog` FORMAT TabSeparated"
        ).strip()
        has_any = rows not in ("", "0")
    if has_any:
        print("[g3] a Bronze table is already registered — skipping re-ingest")
        return
    trigger_and_wait(INGEST_JOB_NAME, 180)


def step_maintenance_job_listed() -> None:
    resp = API.get(f"{API_URL}/api/pipelines", timeout=10)
    if not resp.ok:
        raise G3Failure(f"GET /api/pipelines failed: {resp.status_code} {resp.text}")
    names = [p.get("id") or p.get("name") for p in resp.json().get("pipelines", [])]
    if MAINTENANCE_JOB_NAME not in names:
        raise G3Failure(f"{MAINTENANCE_JOB_NAME!r} not in GET /api/pipelines: {names}")
    print(f"[g3] {MAINTENANCE_JOB_NAME!r} is visible via GET /api/pipelines")


def step_verify_r10_schema_drift_guard() -> None:
    """R10 (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §5): the
    `bronze_meta.*` registry schema now has exactly one owner —
    `dagster/dispar_orchestrate/bronze_catalog.py`'s `EXPECTED_SCHEMAS` —
    and a mismatch between that owner's expectation and the table
    actually sitting in ClickHouse must fail loudly, not be silently
    tolerated by a bare `CREATE TABLE IF NOT EXISTS`.

    This constructs a deliberately WRONG `TableSchema` for the real,
    already-existing `lake.bronze_meta.dataset_catalog` table (extra
    bogus column) and asserts `_assert_or_create_schema` raises
    `SchemaDriftError` — proving the guard fires on a real mismatch — then
    re-checks the CORRECT schema to prove a matching schema passes clean
    and the table itself was never touched.
    """
    import bronze_catalog  # mounted alongside this script — see docker-compose.yml

    target = bronze_catalog.ClickHouseTarget(url=CH_URL, user=CH_USER, password=CH_PASSWORD)

    real_schema = next(
        s for s in bronze_catalog.EXPECTED_SCHEMAS if s.table_name == "bronze_meta.dataset_catalog"
    )
    mismatched_schema = bronze_catalog.TableSchema(
        table_name=real_schema.table_name,
        columns=real_schema.columns + (("bogus_extra_column", "String"),),
        engine=real_schema.engine,
        order_by=real_schema.order_by,
    )
    try:
        bronze_catalog._assert_or_create_schema(target, mismatched_schema)
    except bronze_catalog.SchemaDriftError as exc:
        print(f"[g3] R10 guard correctly raised SchemaDriftError on a deliberate mismatch: {exc}")
    else:
        raise G3Failure(
            "R10 guard failed to fire: _assert_or_create_schema accepted a "
            "TableSchema with an extra bogus column against the real "
            "bronze_meta.dataset_catalog table"
        )

    # The guard must also pass clean against the CORRECT schema — proving
    # this isn't just permanently broken/always-raising, and that the
    # mismatch check above never mutated the real table.
    bronze_catalog._assert_or_create_schema(target, real_schema)
    print("[g3] R10 guard passes clean against the correct schema")


def step_verify_metrics_surfaced() -> None:
    resp = API.get(f"{API_URL}/api/governance/maintenance", timeout=10)
    if not resp.ok:
        raise G3Failure(f"GET /api/governance/maintenance failed: {resp.status_code} {resp.text}")
    runs = resp.json().get("maintenance", [])
    if not runs:
        raise G3Failure("GET /api/governance/maintenance returned no runs after a maintenance job")
    tables = [r.get("tableName") for r in runs]
    print(f"[g3] GET /api/governance/maintenance shows {len(runs)} run(s) for tables: {tables}")
    for r in runs:
        if "dryRun" not in r or "applied" not in r:
            raise G3Failure(f"maintenance run missing dryRun/applied metrics: {r}")


def main() -> int:
    try:
        step_wait_for_services()
        step_login()
        step_ensure_bronze_table_exists()
        step_verify_r10_schema_drift_guard()
        step_maintenance_job_listed()
        trigger_and_wait(MAINTENANCE_JOB_NAME, 180)
        step_verify_metrics_surfaced()
    except G3Failure as exc:
        print(f"[g3] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g3] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
