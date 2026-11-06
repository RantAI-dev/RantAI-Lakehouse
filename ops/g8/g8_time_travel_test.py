#!/usr/bin/env python3
"""G8 time-travel acceptance test (`DATA-16`, finding F2): proves that a
governed Bronze table read at a PAST version keeps its masking and row
filter, and that the version pin actually reads the past.

`POST /api/query/run` on ClickHouse accepts `SETTINGS iceberg_snapshot_id =
<id>` and reads an Iceberg table as it stood after an earlier load
(`DATA-16` F1, run by hand against a live stack); on Trino the same pin
is `FOR VERSION AS OF <id>` on the table (`DATA-21`). Nothing proved that
the policy rewrite (`sql_rewrite.rs`) still applies when that setting is
present, so a past version could be a way around a mask. This gate is that
proof. It sits beside `g8_governance_test.py` (which proves masking on a
`MergeTree` table at its current state only) and stands alone, like every
gate script in this repo: it imports nothing from that file or from
`ops/g3a/g3a_test.py`, so a bug shared with them cannot hide here. The
one thing it DOES share is the `Trino` readiness wait
(`ops/g8/trino_ready.py`, DATA-21 S2 review BLOCKER 1), and that helper
carries no assertion — it only waits, before the first query, for the
coordinator to be able to answer — so the independence claimed above is
unchanged.

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
5. Through `POST /api/query/run` with the older version pinned, asserts
   that the admin gets clear text and every row of the older version; that
   the Analyst gets `***` in `customer`, only the rows the filter allows
   (fewer than the admin's, and exactly the admin's `id <= 1000` count),
   and the same `amount` values the admin sees.
6. Asserts the same for the Analyst with the pin written inside a subquery
   and inside a CTE: each is masked or refused (`422`), never clear text.
7. Asserts the admin's count at the older version is below the current
   version's count and equals the snapshot's own recorded total, so the pin
   is shown to have read the past and not the present.

# The engine this run proves (DATA-21 S2)

This gate takes an ENGINE argument and proves the same past-version
assertions on that engine: `--engine trino` (or `G8_ENGINE=trino`) proves
them on `Trino`, and the default — no argument at all — proves them on
`ClickHouse`, which is exactly what this gate always did.

The two engines read the SAME Bronze table and are held by the SAME
policy — `Trino` reads it through Lakekeeper's Iceberg REST catalog
(`iceberg.bronze.<table>`) where `ClickHouse` reads it through the
`DataLakeCatalog` database (`icecat_api.`bronze.<table>``), and the
policy engine matches a rewritten statement's table by its CANONICAL
name (`bronze.<table>` — the catalog is not part of it). So one authored
policy governs the table on both engines.

They disagree on how the gate's own SQL is spelled, and each difference
was measured against the engine itself rather than guessed:

- the version pin: `ClickHouse` pins the WHOLE statement with
  `SETTINGS iceberg_snapshot_id = <id>` at its end, while `Trino` pins
  the TABLE with `FOR VERSION AS OF <id>` right after the table
  reference — `Trino` reads that clause in no other position (measured
  against `trinodb/trino:483`: `FROM t WHERE ... FOR VERSION AS OF 1`
  does not parse);
- the count: `ClickHouse` writes `count()`, `Trino` writes `count(*)`;
- the truth value: `Trino` has no integer truth value, so `WHERE 1` (the
  predicate that selects every row) is `WHERE true` there;
- the `ORDER BY`: `Trino` may only order by a column the outer select
  list carries, so the `Trino` shapes select the ordering column beside
  the masked one.

# What this gate cannot prove

- It observes `lakehouse-api`'s HTTP responses only, not the SQL the
  engine ran or the policy engine's decision. A rewrite that hardcoded
  `***` for a column called `customer` would pass; steps 5 and 7 narrow
  that by asking the same questions of the admin, who must get clear text.
- A `422` from the subquery and CTE shapes is accepted as "refused", the
  way `g8_governance_test.py` accepts it. It cannot tell the policy
  engine's refusal from the engine rejecting the version clause in that
  position; both are safe, neither leaks.
- One version applies to every Iceberg table in a query (`DATA-16` F7), so
  this gate reads one table at one version. Two tables at two versions are
  out of scope.
- It has never been run on this machine (no containers are started while
  writing it); the first real run is CI's `g8-time-travel` job.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time

import requests
from argon2 import PasswordHasher  # argon2-cffi, installed by the runner's command

# The one readiness wait for `Trino`, shared with the other G8 gate
# (`ops/g8/trino_ready.py`) — DATA-21 S2 review BLOCKER 1. It lives in
# its own module on purpose: this gate's ASSERTIONS stay independent of
# the governance gate's, and only the wait is shared.
import trino_ready

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

# SEC-20 scopes the query path to the tenant: with more than one seeded
# tenant (migration 0002 seeds four), a caller in no tenant -- and any
# caller outside the shared catalog's tenant -- is refused every shared
# table with the same 404 a missing table gets. This gate proves time
# travel keeps a policy for an analyst, so the analyst it seeds joins the
# migration-seeded tenant (stable id, meant to be referenced) that the CI
# stack names in `CATALOG_TENANT_ID`; a tenant-less analyst would prove
# the refusal, which `tests/query_tenant_scope.rs` already asserts.
CATALOG_TENANT_ID = "11111111-1111-4111-8111-000000000001"

POLICY_NAME = "g8-time-travel-mask"
MASKED_COLUMN = "customer"
FILTER_LIMIT = 1000  # the policy's row filter is `id <= FILTER_LIMIT`
LATE_ROWS = 10  # rows added to the source between the two loads
LATE_CUSTOMER = "g8tt_late"

# The engines this gate knows how to prove.
ENGINE_CLICKHOUSE = "clickhouse"
ENGINE_TRINO = "trino"

# The table the gate reads, on each engine. `ClickHouse` reads the
# Bronze table through the `DataLakeCatalog` database
# `clickhouse-iceberg-init` creates (`ICEBERG_QUERY_DB`); `Trino` has no
# `ClickHouse` connector and reads the SAME table through Lakekeeper's
# Iceberg REST catalog. The policy engine matches both on the table's
# canonical name, `bronze.<table>`.
ICEBERG_TABLE = f"{ICEBERG_DB}.`bronze.{BRONZE_TABLE_NAME}`"
TRINO_TABLE = f"iceberg.bronze.{BRONZE_TABLE_NAME}"

# The predicate that selects every row, per engine: `Trino` has no
# integer truth value (`WHERE 1` does not parse there), so it takes a
# boolean.
ALL_ROWS_PREDICATE = {ENGINE_CLICKHOUSE: "1", ENGINE_TRINO: "true"}


class G8TimeTravelFailure(Exception):
    """Raised for any acceptance-criterion violation or infrastructure
    failure this gate cannot proceed past."""


def parse_engine(argv: list[str]) -> str:
    """The engine this run proves.

    `--engine` on the command line beats the `G8_ENGINE` environment
    variable, which beats the `clickhouse` default — so a run with no
    argument at all is exactly the ClickHouse leg this gate always was.
    The environment variable is what CI uses (`docker compose run -e
    G8_ENGINE=trino ...`), because the compose runner's own command line
    is fixed.

    # Errors

    Raises [`G8TimeTravelFailure`] for an engine this gate cannot prove.
    """
    parser = argparse.ArgumentParser(description="G8 time-travel acceptance test")
    parser.add_argument(
        "--engine",
        choices=[ENGINE_CLICKHOUSE, ENGINE_TRINO],
        default=None,
        help="the engine to prove (default: $G8_ENGINE, else clickhouse)",
    )
    args, _unknown = parser.parse_known_args(argv)
    engine = (args.engine or os.environ.get("G8_ENGINE") or ENGINE_CLICKHOUSE).strip().lower()
    if engine not in (ENGINE_CLICKHOUSE, ENGINE_TRINO):
        raise G8TimeTravelFailure(
            f"unknown engine {engine!r}: this gate proves {ENGINE_CLICKHOUSE} or {ENGINE_TRINO}"
        )
    return engine


def table_ref(engine: str) -> str:
    """The table the gate reads, on the engine it names."""
    return TRINO_TABLE if engine == ENGINE_TRINO else ICEBERG_TABLE


def count_sql(engine: str, where: str, pin: str | None) -> str:
    """A row count, always with a WHERE (R11: a bare Iceberg count can
    overcount on delete files, `docs/plans/P5-RESULT.md`).

    `ClickHouse` writes `count()` and pins the whole statement with a
    `SETTINGS` clause at its end; `Trino` writes `count(*)` and pins the
    TABLE, so its version clause goes right after the table reference and
    before the `WHERE`.
    """
    table = table_ref(engine)
    if engine == ENGINE_TRINO:
        pin_clause = f" FOR VERSION AS OF {pin}" if pin else ""
        return f"SELECT count(*) AS n FROM {table}{pin_clause} WHERE {where}"
    settings = f" SETTINGS iceberg_snapshot_id = {pin}" if pin else ""
    return f"SELECT count() AS n FROM {table} WHERE {where}{settings}"


def rows_sql(engine: str, columns: str, where: str, pin: str | None) -> str:
    """The gate's own row read, with the version pinned.

    `ClickHouse` pins the whole statement with a `SETTINGS` clause at its
    end; `Trino` pins the table, so its version clause goes right after
    the table reference and before the `WHERE`.
    """
    table = table_ref(engine)
    if engine == ENGINE_TRINO:
        pin_clause = f" FOR VERSION AS OF {pin}" if pin else ""
        return f"SELECT {columns} FROM {table}{pin_clause} WHERE {where} ORDER BY id LIMIT 5"
    tail = "ORDER BY id LIMIT 5"
    if pin:
        tail += f" SETTINGS iceberg_snapshot_id = {pin}"
    return f"SELECT {columns} FROM {table} WHERE {where} {tail}"


def nested_pin_shapes(engine: str, pin: str) -> dict[str, str]:
    """The two shapes that hide the version pin inside a subquery and
    inside a CTE. Each must come back masked or refused, never clear
    text.

    `Trino` reads the version clause right after the table reference, and
    may only order by a column the OUTER select list carries — so the
    `Trino` shapes select the ordering column beside the masked one,
    which the `ClickHouse` shapes do not need.
    """
    table = table_ref(engine)
    if engine == ENGINE_TRINO:
        inner = f"SELECT id, {MASKED_COLUMN} FROM {table} FOR VERSION AS OF {pin}"
        return {
            "subquery": f"SELECT id, {MASKED_COLUMN} FROM ({inner}) AS s ORDER BY id LIMIT 5",
            "cte": f"WITH x AS ({inner}) SELECT id, {MASKED_COLUMN} FROM x ORDER BY id LIMIT 5",
        }
    inner = f"SELECT id, {MASKED_COLUMN} FROM {table} SETTINGS iceberg_snapshot_id = {pin}"
    return {
        "subquery": f"SELECT {MASKED_COLUMN} FROM ({inner}) AS s ORDER BY id LIMIT 5",
        "cte": f"WITH x AS ({inner}) SELECT {MASKED_COLUMN} FROM x ORDER BY id LIMIT 5",
    }


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
    """One `app_user` + `auth_identity` + `app_user_role` +
    `app_user_tenant` row, inserted directly (`POST /api/identity/users`
    creates no password credential), the same pattern
    `g8_governance_test.py` uses. Idempotent upserts."""
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
    psql(
        "INSERT INTO app_user_tenant (user_id, tenant_id) "
        f"VALUES ('{user_id}', '{CATALOG_TENANT_ID}') "
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


def run_query(session: requests.Session, sql: str, engine: str) -> requests.Response:
    """The gate's own query, on the engine this run proves. `Trino` is
    named in the request body because `POST /api/query/run` reads the
    engine from there; `Trino`'s own timeout is generous, because its
    first query pays for the Iceberg connector's metadata round trip to
    Lakekeeper."""
    timeout = 60 if engine == ENGINE_CLICKHOUSE else 180
    return session.post(
        f"{API_URL}/api/query/run", json={"sql": sql, "engine": engine}, timeout=timeout
    )


def step_wait_for_trino() -> None:
    """Waits until `Trino` can answer, before this leg's first query.

    DATA-21 S2 review BLOCKER 1: the coordinator answers HTTP while it
    is still starting, so a statement sent in that window comes back as
    a 422 ("The server is still starting up"). This gate's Trino leg
    passed its first CI run by the same luck the governance gate's
    failed run exposed — a gate that passes by timing is a flaky gate.

    The wait itself is ONE shared helper (`ops/g8/trino_ready.py`), used
    by both G8 gates, so this leg and the governance leg cannot drift
    apart (`AGENTS.md` rule 4: a duplicated guard is a finding).
    """
    try:
        trino_ready.wait_for_trino()
    except trino_ready.TrinoNotReady as exc:
        raise G8TimeTravelFailure(f"Trino never became ready: {exc}") from exc


def rows_of(session: requests.Session, sql: str, who: str, engine: str) -> list[dict]:
    resp = run_query(session, sql, engine)
    if not resp.ok:
        raise G8TimeTravelFailure(f"{who} query failed: {resp.status_code} {resp.text}\nSQL: {sql}")
    return resp.json().get("rows", [])


def count_of(session: requests.Session, where: str, pin: str | None, who: str, engine: str) -> int:
    """A row count, always with a WHERE (R11: a bare Iceberg count can
    overcount on delete files, `docs/plans/P5-RESULT.md`)."""
    rows = rows_of(session, count_sql(engine, where, pin), who, engine)
    if len(rows) != 1:
        raise G8TimeTravelFailure(f"{who} count returned {len(rows)} rows")
    return int(rows[0]["n"])


def step_assert_pinned_reads(
    admin: requests.Session, analyst: requests.Session, older: dict, engine: str
) -> None:
    pin = older["id"]
    all_rows = ALL_ROWS_PREDICATE[engine]
    older_total = count_of(admin, all_rows, pin, "admin", engine)
    current_total = count_of(admin, all_rows, None, "admin", engine)
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

    allowed = count_of(admin, f"id <= {FILTER_LIMIT}", pin, "admin", engine)
    analyst_total = count_of(analyst, all_rows, pin, "analyst", engine)
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
    admin_rows = rows_of(
        admin,
        rows_sql(engine, columns, f"id <= {FILTER_LIMIT}", pin),
        "admin",
        engine,
    )
    analyst_rows = rows_of(
        analyst, rows_sql(engine, columns, all_rows, pin), "analyst", engine
    )
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


def step_assert_nested_pins_not_clear(
    analyst: requests.Session, older: dict, engine: str
) -> None:
    """The pin written inside a subquery and inside a CTE: each must be
    masked or refused with 422, never clear text. A 401/403 is never an
    acceptable refusal (that would be the session losing authentication)."""
    pin = older["id"]
    shapes = nested_pin_shapes(engine, pin)
    for name, sql in shapes.items():
        resp = run_query(analyst, sql, engine)
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
        engine = parse_engine(sys.argv[1:])
        print(f"[g8-tt] proving time travel on the {engine} engine")
        step_wait_for_services()
        admin = login(AUTH_EMAIL, AUTH_PASSWORD, "bootstrap admin")
        step_seed_analyst_login()
        older, _current = step_load_twice(admin)
        step_author_policy(admin)
        analyst = login(ANALYST_EMAIL, ANALYST_PASSWORD, "analyst")
        if engine == ENGINE_TRINO:
            # DATA-21 S2 review BLOCKER 1: the coordinator must be ready
            # before this leg's FIRST query on it. Everything above is
            # engine-independent (the two loads, the policy), so it runs
            # while the coordinator boots.
            step_wait_for_trino()
        step_assert_pinned_reads(admin, analyst, older, engine)
        step_assert_nested_pins_not_clear(analyst, older, engine)
    except G8TimeTravelFailure as exc:
        print(f"[g8-tt] FAILED: {exc}", file=sys.stderr)
        return 1
    print("[g8-tt] PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
