#!/usr/bin/env python3
"""G8 time-travel acceptance test (`DATA-16`, finding F2): proves that a
governed Bronze table read at a PAST version keeps its masking and row
filter, and that the version pin actually reads the past.

`POST /api/query/run` on ClickHouse accepts `SETTINGS iceberg_snapshot_id =
<id>` and reads an Iceberg table as it stood after an earlier load
(`DATA-16` F1, run by hand against a live stack). Nothing proved that the
policy rewrite (`sql_rewrite.rs`) still applies when that setting is
present, so a past version could be a way around a mask. This gate is that
proof. It sits beside `g8_governance_test.py` (which proves masking on a
`MergeTree` table at its current state only) and stands alone, like every
gate script in this repo: it imports nothing from that file or from
`ops/g3a/g3a_test.py`, so a bug shared with them cannot hide here.

# What it does

1. Waits for `lakehouse-api` and the Dagster webserver, logs in as the
   bootstrap admin.
2. Loads the Bronze table twice through `bronze_ingest_job` (the same job
   `g3a_test.py` runs, launched through `POST /api/pipelines/{id}/trigger`).
   That job writes with the default load plan, `append`
   (`adapters/sink.py::LoadPlan`), so each load adds the whole source again
   as a new Iceberg snapshot and the table's row count grows with every
   load. Between the loads this script inserts a few rows into the demo
   source table with `psql`, so the two versions differ by more than a
   repeat of the same rows.
3. Reads the table's versions from `GET /api/lakehouse/tables/bronze/<table>`
   and takes the newest snapshot as the current one and the newest one that
   existed after the first load as the older one.
4. Seeds an Analyst login and authors, through `POST /api/governance/
   policies`, a ready policy for role Analyst on `bronze.<table>` that
   masks `customer` and filters rows with `id <= 1000`.
5. Through `POST /api/query/run` on ClickHouse with the older version
   pinned, asserts that the admin gets clear text and every row of the
   older version; that the Analyst gets `***` in `customer`, only the rows
   the filter allows (fewer than the admin's, and exactly the admin's
   `id <= 1000` count), and the same `amount` values the admin sees.
6. Asserts the same for the Analyst with the pin written inside a subquery
   and inside a CTE: each is masked or refused (`422`), never clear text.
7. Asserts the admin's count at the older version is below the current
   version's count and equals the snapshot's own recorded total, so the pin
   is shown to have read the past and not the present.

# What this gate cannot prove

- It observes `lakehouse-api`'s HTTP responses only, not the SQL ClickHouse
  ran or the policy engine's decision. A rewrite that hardcoded `***` for a
  column called `customer` would pass; steps 5 and 7 narrow that by asking
  the same questions of the admin, who must get clear text.
- A `422` from the subquery and CTE shapes is accepted as "refused", the
  way `g8_governance_test.py` accepts it. It cannot tell the policy
  engine's refusal from ClickHouse rejecting a `SETTINGS` clause in that
  position; both are safe, neither leaks.
- One version applies to every Iceberg table in a query (`DATA-16` F7), so
  this gate reads one table at one version. Two tables at two versions are
  out of scope.
- Trino is out of scope (`DATA-21`): it has no version pin the API can read.
- It has never been run on this machine (no containers are started while
  writing it); the first real run is CI's `g8-time-travel` job.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time

import requests
from argon2 import PasswordHasher  # argon2-cffi, installed by the runner's command

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "http://dagster-webserver:3000/graphql")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "ci@example.com")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "ci-password-not-real-123")
JOB_NAME = os.environ.get("BRONZE_JOB_NAME", "bronze_ingest_job")
BRONZE_TABLE_NAME = os.environ.get("BRONZE_TABLE_NAME", "g3a_orders")
SOURCE_SCHEMA = os.environ.get("BRONZE_SOURCE_SCHEMA", "ingest_demo")
SOURCE_TABLE = os.environ.get("BRONZE_SOURCE_TABLE", "orders")
# The `DataLakeCatalog` database `clickhouse-iceberg-init` creates and the
# API reads through (`ICEBERG_QUERY_DB`).
ICEBERG_DB = os.environ.get("ICEBERG_QUERY_DB", "icecat_api")

# Gate-owned fixture identity (RFC 2606 `.invalid`, as the other gates use).
ANALYST_EMAIL = "g8-time-travel-analyst@lakehouse.invalid"
ANALYST_PASSWORD = "g8-time-travel-password-not-real-456"  # noqa: S105 -- gate fixture, not a real secret

POLICY_NAME = "g8-time-travel-mask"
MASKED_COLUMN = "customer"
FILTER_LIMIT = 1000  # the policy's row filter is `id <= FILTER_LIMIT`
LATE_ROWS = 10  # rows added to the source between the two loads
LATE_CUSTOMER = "g8tt_late"

TABLE = f"{ICEBERG_DB}.`bronze.{BRONZE_TABLE_NAME}`"


class G8TimeTravelFailure(Exception):
    """Raised for any acceptance-criterion violation or infrastructure
    failure this gate cannot proceed past."""


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 2.0) -> None:
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            if check():
                print(f"[g8-tt] ready: {name}")
                return
        except Exception as exc:  # noqa: BLE001 - report the real cause below
            last_err = exc
        time.sleep(interval_s)
    raise G8TimeTravelFailure(f"timed out waiting for {name!r}: {last_err}")


def psql(sql: str) -> str:
    """Runs `sql` against the console's own Postgres through the `psql`
    client, with connection parameters from the standard `PG*` variables
    the runner's compose service sets (the demo source table lives in the
    same database)."""
    result = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-tqA", "-c", sql],
        capture_output=True,
        text=True,
        check=False,
        timeout=30,
    )
    if result.returncode != 0:
        raise G8TimeTravelFailure(f"psql failed: {result.stderr}\nSQL: {sql}")
    return result.stdout.strip()


def login(email: str, password: str, who: str) -> requests.Session:
    """Logs in, handling the bootstrap admin's forced first-login password
    rotation the way the other gates do. Every call checks `.ok`, so a 401
    here is never mistaken for a later, unrelated failure."""
    session = requests.Session()
    resp = session.post(
        f"{API_URL}/api/auth/login", json={"email": email, "password": password}, timeout=10
    )
    if not resp.ok:
        raise G8TimeTravelFailure(f"{who} login failed: {resp.status_code} {resp.text}")
    if resp.json().get("mustChangePassword"):
        rotated = session.post(
            f"{API_URL}/api/auth/change-password", json={"newPassword": password}, timeout=10
        )
        if not rotated.ok:
            raise G8TimeTravelFailure(f"{who} password rotation failed: {rotated.status_code} {rotated.text}")
        relogin = session.post(
            f"{API_URL}/api/auth/login", json={"email": email, "password": password}, timeout=10
        )
        if not relogin.ok:
            raise G8TimeTravelFailure(f"{who} re-login failed: {relogin.status_code} {relogin.text}")
    print(f"[g8-tt] logged in as {who}")
    return session


def step_wait_for_services() -> None:
    _wait_for(
        "lakehouse-api",
        lambda: requests.get(f"{API_URL}/health", timeout=5).status_code == 200,
        90,
    )
    _wait_for(
        "Dagster webserver",
        lambda: requests.get(DAGSTER_URL.replace("/graphql", "/server_info"), timeout=5).status_code
        == 200,
        90,
    )


def step_seed_analyst_login() -> None:
    """One `app_user` + `auth_identity` + `app_user_role` row, inserted
    directly (`POST /api/identity/users` creates no password credential),
    the same pattern `g8_governance_test.py` uses. Idempotent upserts."""
    user_id = psql(
        "INSERT INTO app_user (id, name, email, status) "
        f"VALUES (gen_random_uuid(), 'G8 Time Travel Analyst', '{ANALYST_EMAIL}', 'active') "
        "ON CONFLICT (email) DO UPDATE SET status = 'active' RETURNING id;"
    )
    phc_hash = PasswordHasher().hash(ANALYST_PASSWORD)
    psql(
        "INSERT INTO auth_identity "
        "(provider, external_subject, app_user_id, password_hash, must_change_password) "
        f"VALUES ('local', '{user_id}', '{user_id}', '{phc_hash}', false) "
        "ON CONFLICT (provider, external_subject) DO UPDATE SET password_hash = EXCLUDED.password_hash;"
    )
    psql(
        "INSERT INTO app_user_role (user_id, role_id) "
        f"SELECT '{user_id}', r.id FROM role r WHERE r.name = 'Analyst' "
        "ON CONFLICT DO NOTHING;"
    )
    print("[g8-tt] seeded Analyst login (idempotent upsert)")


def trigger_and_wait(admin: requests.Session) -> None:
    """Launches `bronze_ingest_job` through the API and waits for Dagster
    to report SUCCESS (the `launchRun` path `g3a_test.py` proves)."""
    resp = admin.post(f"{API_URL}/api/pipelines/{JOB_NAME}/trigger", timeout=10)
    if not resp.ok:
        raise G8TimeTravelFailure(f"POST trigger failed: {resp.status_code} {resp.text}")
    body = resp.json()
    run_id = body.get("id") or body.get("runId") or body.get("run_id")
    if not run_id:
        raise G8TimeTravelFailure(f"trigger response had no run id: {body}")
    print(f"[g8-tt] launched run {run_id}")
    query = (
        "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename "
        "... on Run { status } } }"
    )

    def check() -> bool:
        poll = requests.post(
            DAGSTER_URL, json={"query": query, "variables": {"rid": run_id}}, timeout=10
        )
        poll.raise_for_status()
        status = poll.json().get("data", {}).get("pipelineRunOrError", {}).get("status")
        if status == "FAILURE":
            raise G8TimeTravelFailure(f"run {run_id} FAILED")
        return status == "SUCCESS"

    _wait_for(f"run {run_id} SUCCESS", check, 240, interval_s=3.0)


def table_snapshots(admin: requests.Session) -> list[dict]:
    resp = admin.get(f"{API_URL}/api/lakehouse/tables/bronze/{BRONZE_TABLE_NAME}", timeout=15)
    if not resp.ok:
        raise G8TimeTravelFailure(f"GET table detail failed: {resp.status_code} {resp.text}")
    return resp.json().get("snapshots", [])


def newest(snapshots: list[dict]) -> dict:
    if not snapshots:
        raise G8TimeTravelFailure("the table has no snapshots")
    return max(snapshots, key=lambda s: s["timestampMs"])


def step_load_twice(admin: requests.Session) -> tuple[dict, dict]:
    """Two loads with a source change between them. Returns `(older,
    current)` snapshots: the newest one after the first load, and the
    newest one after the second. The ids are strings and are never passed
    through `int`/`float` (a 64-bit id would lose digits)."""
    trigger_and_wait(admin)
    older = newest(table_snapshots(admin))
    psql(
        f"INSERT INTO {SOURCE_SCHEMA}.{SOURCE_TABLE} (customer, amount) "
        f"SELECT '{LATE_CUSTOMER}', g::numeric FROM generate_series(1, {LATE_ROWS}) AS g;"
    )
    print(f"[g8-tt] added {LATE_ROWS} rows to the demo source between the loads")
    trigger_and_wait(admin)
    current = newest(table_snapshots(admin))
    if current["id"] == older["id"]:
        raise G8TimeTravelFailure("the second load produced no new snapshot")
    print(f"[g8-tt] older version {older['id']}, current version {current['id']}")
    return older, current


def step_author_policy(admin: requests.Session) -> None:
    """A ready policy for role Analyst on `bronze.<table>` that masks one
    column and filters rows on another, authored through the real route so
    `PolicyCondition::parse` validates the shape. Read-then-create (not
    swallowing a 409), so an unrelated name clash still fails the gate."""
    key = f"bronze.{BRONZE_TABLE_NAME}"
    conditions = (
        '{"roles":["Analyst"],"table":"' + key + '","mask":["' + MASKED_COLUMN + '"],'
        '"rowFilter":"id <= ' + str(FILTER_LIMIT) + '"}'
    )
    existing = admin.get(f"{API_URL}/api/governance/policies", timeout=10)
    if not existing.ok:
        raise G8TimeTravelFailure(f"failed to list policies: {existing.status_code} {existing.text}")
    if any(p.get("name") == POLICY_NAME for p in existing.json()):
        print(f"[g8-tt] policy {POLICY_NAME!r} already authored; skipping POST")
        return
    resp = admin.post(
        f"{API_URL}/api/governance/policies",
        json={
            "name": POLICY_NAME,
            "kind": "Row filter",
            "subjects": "Analyst (g8 time-travel gate)",
            "resources": key,
            "effect": "Permit with obligation",
            "conditions": conditions,
            "activate": True,
        },
        timeout=10,
    )
    if not resp.ok:
        raise G8TimeTravelFailure(f"failed to author the policy: {resp.status_code} {resp.text}")
    print(f"[g8-tt] authored policy {POLICY_NAME!r}")


def run_query(session: requests.Session, sql: str) -> requests.Response:
    return session.post(
        f"{API_URL}/api/query/run", json={"sql": sql, "engine": "clickhouse"}, timeout=60
    )


def rows_of(session: requests.Session, sql: str, who: str) -> list[dict]:
    resp = run_query(session, sql)
    if not resp.ok:
        raise G8TimeTravelFailure(f"{who} query failed: {resp.status_code} {resp.text}\nSQL: {sql}")
    return resp.json().get("rows", [])


def count_of(session: requests.Session, where: str, pin: str | None, who: str) -> int:
    """A row count, always with a WHERE (R11: a bare Iceberg count can
    overcount on delete files, `docs/plans/P5-RESULT.md`)."""
    settings = f" SETTINGS iceberg_snapshot_id = {pin}" if pin else ""
    rows = rows_of(session, f"SELECT count() AS n FROM {TABLE} WHERE {where}{settings}", who)
    if len(rows) != 1:
        raise G8TimeTravelFailure(f"{who} count returned {len(rows)} rows")
    return int(rows[0]["n"])


def step_assert_pinned_reads(admin: requests.Session, analyst: requests.Session, older: dict) -> None:
    pin = older["id"]
    older_total = count_of(admin, "1", pin, "admin")
    current_total = count_of(admin, "1", None, "admin")
    recorded = (older.get("summary") or {}).get("totalRecords")
    if recorded is not None and older_total != recorded:
        raise G8TimeTravelFailure(
            f"the pinned count ({older_total}) is not the snapshot's recorded total ({recorded})"
        )
    if not 0 < older_total < current_total:
        raise G8TimeTravelFailure(
            f"the pin did not read the past: older version {older_total} rows, current {current_total}"
        )
    print(f"[g8-tt] admin: older version {older_total} rows, current {current_total} rows")

    allowed = count_of(admin, f"id <= {FILTER_LIMIT}", pin, "admin")
    analyst_total = count_of(analyst, "1", pin, "analyst")
    if analyst_total >= older_total:
        raise G8TimeTravelFailure(
            f"the row filter did not apply at the older version: analyst {analyst_total} of {older_total}"
        )
    if analyst_total != allowed:
        raise G8TimeTravelFailure(
            f"the analyst's rows ({analyst_total}) are not the admin's filtered rows ({allowed})"
        )
    print(f"[g8-tt] analyst: {analyst_total} of {older_total} rows at the older version")

    columns = f"id, {MASKED_COLUMN}, amount"
    tail = f"ORDER BY id LIMIT 5 SETTINGS iceberg_snapshot_id = {pin}"
    admin_rows = rows_of(admin, f"SELECT {columns} FROM {TABLE} WHERE id <= {FILTER_LIMIT} {tail}", "admin")
    analyst_rows = rows_of(analyst, f"SELECT {columns} FROM {TABLE} WHERE 1 {tail}", "analyst")
    if not admin_rows or len(admin_rows) != len(analyst_rows):
        raise G8TimeTravelFailure(
            f"cannot compare: admin {len(admin_rows)} rows, analyst {len(analyst_rows)} rows"
        )
    for a_row, an_row in zip(admin_rows, analyst_rows):
        if a_row[MASKED_COLUMN] in ("***", ""):
            raise G8TimeTravelFailure(f"the admin's own read was masked or empty: {a_row}")
        if an_row[MASKED_COLUMN] != "***":
            raise G8TimeTravelFailure(f"the older version leaked {MASKED_COLUMN} to the analyst: {an_row}")
        if an_row["id"] != a_row["id"] or an_row["amount"] != a_row["amount"]:
            raise G8TimeTravelFailure(
                f"the unmasked columns differ between admin and analyst: {a_row} vs {an_row}"
            )
    print("[g8-tt] analyst: masked column is ***, unmasked columns are clear and equal to the admin's")


def step_assert_nested_pins_not_clear(analyst: requests.Session, older: dict) -> None:
    """The pin written inside a subquery and inside a CTE: each must be
    masked or refused with 422, never clear text. A 401/403 is never an
    acceptable refusal (that would be the session losing authentication)."""
    pin = older["id"]
    inner = f"SELECT id, {MASKED_COLUMN} FROM {TABLE} SETTINGS iceberg_snapshot_id = {pin}"
    shapes = {
        "subquery": f"SELECT {MASKED_COLUMN} FROM ({inner}) AS s ORDER BY id LIMIT 5",
        "cte": f"WITH x AS ({inner}) SELECT {MASKED_COLUMN} FROM x ORDER BY id LIMIT 5",
    }
    for name, sql in shapes.items():
        resp = run_query(analyst, sql)
        if resp.status_code in (401, 403):
            raise G8TimeTravelFailure(f"{name}: the check itself failed to authenticate: {resp.status_code}")
        if resp.status_code == 422:
            print(f"[g8-tt] pin in a {name} was refused (422), acceptable")
            continue
        if not resp.ok:
            raise G8TimeTravelFailure(f"{name}: unexpected failure {resp.status_code} {resp.text}")
        rows = resp.json().get("rows", [])
        if not rows:
            raise G8TimeTravelFailure(f"{name}: no rows, so masking cannot be shown")
        if any(r.get(MASKED_COLUMN) != "***" for r in rows):
            raise G8TimeTravelFailure(f"{name}: the older version leaked {MASKED_COLUMN}: {rows[0]}")
        print(f"[g8-tt] pin in a {name} was masked, acceptable")


def main() -> int:
    try:
        step_wait_for_services()
        admin = login(AUTH_EMAIL, AUTH_PASSWORD, "bootstrap admin")
        step_seed_analyst_login()
        older, _current = step_load_twice(admin)
        step_author_policy(admin)
        analyst = login(ANALYST_EMAIL, ANALYST_PASSWORD, "analyst")
        step_assert_pinned_reads(admin, analyst, older)
        step_assert_nested_pins_not_clear(analyst, older)
    except G8TimeTravelFailure as exc:
        print(f"[g8-tt] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g8-tt] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
