#!/usr/bin/env python3
"""G9 acceptance test (file upload, DATA-9): a file is uploaded, previewed,
loaded into a raw table, loaded again with `append`, deleted, and the table is
checked to be still there, and then a pipeline is built over that uploaded
table, re-run by a re-upload of it, left alone by an upload into another
table, and traced back to the upload in lineage -- the end-to-end proof that
the upload routes, the `file_ingest_job` run, the dependency sensor and the
query route work together on a deployed stack, which their unit and route
tests, each with the other side faked, cannot show. T9 of
`docs/superpowers/plans/2026-10-02-upload-file.md`, under ADR 0014; step 10
is `DATA-19`
(`docs/superpowers/plans/2026-10-14-data-19-uploads-feed-pipelines.md`).

# What it checks, in order

1. The API answers, and (when `DAGSTER_URL` is set) the code location lists
   `file_ingest_job`. Then it signs in, as `ops/g6/g6_ingest_matrix_test.py`
   does, and makes sure the account belongs to a tenant (an upload belongs to
   the uploader's active tenant, and the bootstrap admin of a fresh database
   is in none: `POST /api/uploads` answers 400 for it; SRC-9 fourth pass).
2. It uploads `ops/fixtures/uploads/sap_report_utf16.xls`: UTF-16 tab-separated
   text with two title lines and two blank lines above its header, named
   `.xls` and sent as an Excel type, the export that motivated the feature.
   The API is not fooled by the name, and its preview detects what the
   fixture's `.expected.json` says (encoding, delimiter, header row) and shows
   its columns and rows.
3. It loads the upload with what the preview detected, `replace`, into a table
   named `g9_upload_<random suffix>`, and polls the upload to `ingested`. It
   expects the upload's own row count, then counts the table through
   `POST /api/query/run`, always under a `WHERE` (R11: an unqualified count on
   a raw Iceberg table can be wrong).
4. It loads the same upload again with `append`: the table holds twice the
   rows.
5. It reads the table's rows back: they are the file's own text, leading zeros
   intact. And every column has the type the load was told (`DATA-18`, T3.6):
   step 3 sends the column list the preview offered, as the console does, so
   `plnt` and `qty` come out `Int64` and `material` (`0000100`, a leading zero)
   `String`, as ClickHouse types them (`toTypeName`). Until `DATA-18` this step
   asserted that EVERY column was text, because the load had no way to type one;
   the assertion changed on purpose, not to get green: text everywhere is now what
   a caller that sends NO column list gets (decision 11), and the workbook, Parquet
   and typed steps below prove that case and the typed one separately.
6. `GET /api/governance/ingest-runs` holds exactly one row per load, both
   `succeeded`, with the rows loaded and no error.
7. It deletes the upload: the answer is 204, the upload is gone from the list
   and from `GET /api/uploads/{id}`, and the table keeps all its rows.
8. An Excel workbook (X4 of `docs/superpowers/plans/2026-10-07-upload-excel.md`):
   it uploads `ops/fixtures/workbooks/stock.xlsx`, expects the answer to list
   its five sheets with `Stock` as the default, previews that sheet (the
   fixed dialect, the sheet's columns and rows, dates as ISO text), loads it
   with `replace` into `<table>_xlsx`, expects the upload's record to name the
   sheet in `parseOptions`, the table to hold the sheet's three rows as text
   (a date as `2025-09-24`, a price as `1.5`), one succeeded `ingest_run`, and
   deletes the upload. The API converts the sheet and stores it beside the
   workbook; the load job is the one step 3 runs, unchanged.

9. A Parquet file (Q4 of section 11 of the same plan): it uploads
   `ops/fixtures/parquet/orders.parquet`, expects the answer to list its five
   columns with the types the file declares, previews it (the fixed dialect,
   the file's column names as the header, its rows as text: a decimal with its
   scale, a date as `2025-09-24`), loads it with `replace` into
   `<table>_parquet` without sending an encoding, delimiter, header row or
   sheet, expects the table to hold the three rows as text, one succeeded
   `ingest_run`, and deletes the upload. The API converts the file and stores
   the text beside it; the load job is the one step 3 runs, unchanged.

10. An uploaded table feeding a pipeline (`DATA-19`,
    `docs/superpowers/plans/2026-10-14-data-19-uploads-feed-pipelines.md`):
    it authors a `bronze.<table>` -> `silver.<table>_clean` pipeline whose
    `dependsOn` names `file_ingest_job` -- the console's "Run again after
    this table is re-uploaded" switch, sent here as the field it sends --
    activates it, waits for the orchestrator to build the job, triggers one
    run and waits for it to succeed, and reads the Silver table through
    `POST /api/query/run`. Then it re-uploads the same table with ONE row
    changed, expects the pipeline to run AGAIN on its own (no trigger call:
    the `authored__<id>_after` sensor fired on the new `file_ingest_job`
    SUCCESS), and expects the Silver table to show the change. Then it loads
    a file into a DIFFERENT table and expects the pipeline NOT to run again
    -- the sensor counts only the runs that loaded its own source table.
    Finally it asks the lineage route for the Silver table and expects the
    chain `upload:<id> -> bronze.<table> -> silver.<table>_clean`.

8. SEC-17: it uploads a CSV file with 1,001 columns. Its preview answers 400
   with "The file has more than 1,000 columns." and the same file read with
   another delimiter previews again; a load request for it answers 400 with the
   same sentence, before any table name is claimed. The upload is deleted.

Exit code 0 and `[g9] PASS` when every step holds; otherwise `[g9] FAILED:
<reason>` on stderr and exit code 1.

# Settings (environment)

- `LAKEHOUSE_API_URL`: the API's base URL (default: the compose service).
- `AUTH_BOOTSTRAP_EMAIL`, `AUTH_BOOTSTRAP_PASSWORD`: an account that holds
  `connector:manage` and `query:read`. Required: there is no default
  credential here.
- `ICEBERG_QUERY_DB`: the ClickHouse database through which the API reads raw
  tables (default `icecat_api`, the compose default). It must be the one the
  API itself is set to.
- `DAGSTER_URL`: the Dagster GraphQL endpoint. Optional: when set, the gate
  waits for the code location to list `file_ingest_job` before it starts.
  Step 10 needs it: without it the gate cannot prove the orchestrator built
  the authored job it authors, and it says so instead of pretending.
- `G9_TENANT_ID`: a tenant the account belongs to, sent as `X-Tenant`.
  Optional: without it the gate puts the account into the seeded tenant
  `meridian-group` (`step_ensure_tenant`) and sends that tenant's id.

Step 10 also needs the stack to have `PIPELINE_RUN_TOKEN` set (the same
value for `lakehouse-api` and the Dagster code location). Unset, the code
location loads zero authored jobs and zero chain sensors, so the gate's
first wait would fail with "the orchestrator never built the job" -- which
is the honest answer for such a stack.

It reads the fixtures from `ops/fixtures/uploads/`, `ops/fixtures/workbooks/` and
`ops/fixtures/parquet/` beside this directory, so a container that runs it needs `ops/` and not only
`ops/g9/`.

# What it leaves behind

The raw tables `g9_upload_<suffix>` (four rows), `g9_upload_<suffix>_xlsx`
(three) and `g9_upload_<suffix>_parquet` (three) with their Iceberg files and
catalog entries; the claims on their names, which are never released; four
`ingest_run` rows; audit events and query history entries. Step 10 adds the
Silver table `g9_upload_<suffix>_clean` (two rows, written by the authored
pipeline), its authored pipeline row and its version row, the corrected copy's
`ingest_run` row, and the claim on `g9_upload_<suffix>_other`. A raw table and
its claim cannot be removed through the API (deleting a raw table is outside
the plan, section 5), so an operator who wants them gone removes them by hand.
The upload itself is deleted at the end, and also when a step fails, as far as
the API lets it (an upload that is still loading cannot be deleted).
"""
from __future__ import annotations

import argparse
import gzip
import io
import json
import os
import re
import secrets
import sys
import time
import zipfile
from pathlib import Path

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures" / "uploads"
FIXTURE = "sap_report_utf16"
WORKBOOKS = Path(__file__).resolve().parents[1] / "fixtures" / "workbooks"
WORKBOOK = "stock.xlsx"
PARQUETS = Path(__file__).resolve().parents[1] / "fixtures" / "parquet"
PARQUET = "orders.parquet"
# What `orders.parquet` holds, as text (see
# `ops/fixtures/parquet/make_parquet.py`): the types the file declares, in
# column order, and the rows the conversion writes (a decimal keeps its scale).
PARQUET_COLUMNS = [
    {"name": "sku", "type": "string"},
    {"name": "name", "type": "string"},
    {"name": "qty", "type": "int64"},
    {"name": "price", "type": "decimal(10,2)"},
    {"name": "received", "type": "date"},
]
PARQUET_ROWS = [
    ["A-100", "Bolt, hex M6", "10", "1.50", "2025-09-24"],
    ["A-200", "Washer", "250", "0.05", "2025-09-25"],
    ["A-300", "Flange DN50 \u2013 steel", "4", "12.00", "2025-10-01"],
]
# What the `Stock` sheet of `stock.xlsx` holds, as text (see
# `ops/fixtures/workbooks/make_workbooks.py`), and the columns the job names
# from its header row.
WORKBOOK_SHEETS = ["Stock", "Quirks", "Offset", "Hidden notes", "Empty"]
WORKBOOK_COLUMNS = ["sku", "name", "qty", "price", "received"]
WORKBOOK_ROWS = [
    ["A-100", "Bolt, hex M6", "10", "1.5", "2025-09-24"],
    ["A-200", "Washer", "250", "0.05", "2025-09-25"],
    ["A-300", "Flange DN50 \u2013 steel", "4", "12", "2025-10-01"],
]

# `ops/g6/g6_ingest_matrix_test.py` waits 150 s for a connector run to
# succeed. A two-row load does the same work (read, one Iceberg commit, one
# registration), so this is that bound with room for a cold code location: a
# bound for a slow stack, not a measurement.
LOAD_TIMEOUT_S = 240

# A freshly written Iceberg table can take a beat to be visible through
# ClickHouse's `DataLakeCatalog` engine after the run that wrote it reported
# success (`g6_ingest_matrix_test.py` polls for the same reason, for 30 s).
VISIBLE_TIMEOUT_S = 60

# DATA-19 step 10. An authored pipeline needs its job BUILT before it can
# be triggered, and the build happens on the code-location reload the
# activate call asks for -- a cold code server on a stack that has just
# booted is the slow case, so these are bounds for a slow stack, not
# measurements.
JOB_BUILT_TIMEOUT_S = 300
# One authored run: read Bronze through the job's own `DataLakeCatalog`
# database, write Silver, report the row delta.
PIPELINE_RUN_TIMEOUT_S = 420
# `GET /api/pipelines/{id}/runs` speaks the API's own vocabulary, not
# Dagster's: `lakehouse_dagster::map_run_status` renders a run's SUCCESS
# as `completed` and its FAILURE as `failed`, and every other state as
# `queued` / `running` / `cancelled` / `unknown`. The gate matches those
# words, so a run that succeeded is never mistaken for one that is still
# going.
RUN_COMPLETED = "completed"
RUN_FAILED = "failed"

# How long the gate watches a pipeline that must NOT run. Longer than a
# sensor evaluation: Dagster's daemon evaluates a run-status sensor on its
# tick, so anything shorter would prove nothing -- a sensor that fired late
# would read as "did not fire".
NO_RETRIGGER_WAIT_S = 180

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "")
QUERY_DB = os.environ.get("ICEBERG_QUERY_DB", "icecat_api")
TENANT_ID = os.environ.get("G9_TENANT_ID", "")
# The tenant that 0042_tenant_provisioning.sql seeds; the same one the g6 gates join.
GATE_TENANT_SLUG = "meridian-group"


class G9Failure(Exception):
    pass


def _wait_for(name: str, check, timeout_s: int, interval_s: float = 3.0, *, retry_failures: bool = False):
    """Poll `check` until it returns something truthy. A `G9Failure` from
    `check` ends the wait at once (the thing waited for can no longer happen),
    unless `retry_failures` says the check may fail while it is not true yet:
    a table that is not visible through ClickHouse for the first seconds."""
    deadline = time.time() + timeout_s
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            value = check()
            if value:
                print(f"[g9] ready: {name}")
                return value
        except G9Failure as exc:
            if not retry_failures:
                raise
            last_err = exc
        except Exception as exc:  # noqa: BLE001 -- report the real cause below
            last_err = exc
        time.sleep(interval_s)
    raise G9Failure(f"timed out waiting for {name!r}: {last_err}")


def _expect(resp, status: int, what: str):
    if resp.status_code != status:
        raise G9Failure(f"{what}: expected {status}, got {resp.status_code} {resp.text[:300]}")
    return resp


def _require_settings() -> None:
    missing = [name for name, value in (("AUTH_BOOTSTRAP_EMAIL", AUTH_EMAIL), ("AUTH_BOOTSTRAP_PASSWORD", AUTH_PASSWORD)) if not value]
    if missing:
        raise G9Failure(f"set {' and '.join(missing)}: the gate has no default credential")
    # QUERY_DB is interpolated into SQL below.
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", QUERY_DB):
        raise G9Failure(f"ICEBERG_QUERY_DB {QUERY_DB!r} is not a plain identifier")


def _fixture() -> tuple[bytes, dict]:
    data = FIXTURES / f"{FIXTURE}.xls"
    expected = FIXTURES / f"{FIXTURE}.expected.json"
    for path in (data, expected):
        if not path.is_file():
            raise G9Failure(f"fixture missing: {path} (this gate reads ops/fixtures/uploads/ beside it)")
    return data.read_bytes(), json.loads(expected.read_text(encoding="utf-8"))


def step_wait_for_services(api, requests) -> None:
    _wait_for("lakehouse-api", lambda: api.get(f"{API_URL}/health", timeout=5).status_code == 200, 60)
    if not DAGSTER_URL:
        print("[g9] DAGSTER_URL is not set: not checking that the code location lists file_ingest_job")
        return
    query = "{ repositoriesOrError { ... on RepositoryConnection { nodes { jobs { name } } } } }"

    def loaded() -> bool:
        resp = requests.post(DAGSTER_URL, json={"query": query}, timeout=5)
        resp.raise_for_status()
        nodes = (resp.json().get("data") or {}).get("repositoriesOrError", {}).get("nodes") or []
        return any(job.get("name") == "file_ingest_job" for node in nodes for job in node.get("jobs", []))

    _wait_for("Dagster code location (file_ingest_job loaded)", loaded, 120)


def step_login(api) -> None:
    """Duplicated from `ops/g6/g6_ingest_matrix_test.py::step_login`, as that
    gate duplicates `g3a_test.py`'s: this repository's gates share no code.
    Every `/api/*` route below needs an authenticated session, which `api` (a
    `requests.Session`) carries from here on."""
    login = api.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
    if not login.ok:
        raise G9Failure(f"login failed: {login.status_code} {login.text[:300]}")
    if login.json().get("mustChangePassword"):
        rotated = api.post(f"{API_URL}/api/auth/change-password", json={"newPassword": AUTH_PASSWORD}, timeout=10)
        if not rotated.ok:
            raise G9Failure(f"forced password rotation failed: {rotated.status_code} {rotated.text[:300]}")
        relogin = api.post(f"{API_URL}/api/auth/login", json={"email": AUTH_EMAIL, "password": AUTH_PASSWORD}, timeout=10)
        if not relogin.ok:
            raise G9Failure(f"re-login after rotation failed: {relogin.status_code} {relogin.text[:300]}")
    print("[g9] logged in")


def _me_tenant_ids(api) -> list[str]:
    me = api.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G9Failure(f"GET /api/auth/me failed: {me.status_code} {me.text[:300]}")
    return [t["id"] for t in me.json().get("tenants", [])]


def step_ensure_tenant(api) -> None:
    """Make sure the account belongs to a tenant, and send it as `X-Tenant`.

    Twin of `step_ensure_tenant` in `ops/g6/g6_ingest_matrix_test.py`, copied
    and not imported because the gates share no module (and g9's runner mounts
    only `ops/g9` and `ops/fixtures`). The bootstrap admin of a fresh database
    belongs to no tenant, and `POST /api/uploads` answers 400 ("Your account
    belongs to no tenant") for such an account: an upload belongs to the
    active tenant (`tenant_scope::resolve`). So the account joins
    `GATE_TENANT_SLUG` with `PUT /api/identity/users/{id}/tenants/{tenant_id}`
    (the unrestricted admin may add itself) and `/api/auth/me` is read again
    to confirm. With `G9_TENANT_ID` set the operator has chosen the tenant and
    nothing is changed."""
    if TENANT_ID:
        if TENANT_ID not in _me_tenant_ids(api):
            raise G9Failure(f"G9_TENANT_ID {TENANT_ID!r} is not one of the account's tenants")
        api.headers["X-Tenant"] = TENANT_ID
        return
    me = api.get(f"{API_URL}/api/auth/me", timeout=10)
    if not me.ok:
        raise G9Failure(f"GET /api/auth/me failed: {me.status_code} {me.text[:300]}")
    listed = api.get(f"{API_URL}/api/identity/tenants", timeout=10)
    if not listed.ok:
        raise G9Failure(f"list tenants failed: {listed.status_code} {listed.text[:300]}")
    tenant = next((t for t in listed.json() if t["slug"] == GATE_TENANT_SLUG), None)
    if tenant is None:
        raise G9Failure(f"tenant {GATE_TENANT_SLUG!r} does not exist (0042_tenant_provisioning.sql seeds it)")
    if tenant["id"] not in _me_tenant_ids(api):
        added = api.put(f"{API_URL}/api/identity/users/{me.json()['id']}/tenants/{tenant['id']}", timeout=10)
        if not added.ok:
            raise G9Failure(f"add the account to tenant {GATE_TENANT_SLUG!r} failed: {added.status_code} {added.text[:300]}")
        if tenant["id"] not in _me_tenant_ids(api):
            raise G9Failure(f"the account was added to tenant {GATE_TENANT_SLUG!r} but /api/auth/me does not list it")
        print(f"[g9] added the account to tenant {GATE_TENANT_SLUG!r}")
    api.headers["X-Tenant"] = tenant["id"]
    print(f"[g9] active tenant: {GATE_TENANT_SLUG}")


def run_query(api, sql: str) -> dict:
    resp = api.post(f"{API_URL}/api/query/run", json={"sql": sql}, timeout=60)
    return _expect(resp, 200, f"query {sql!r}").json()


def count_rows(api, table: str) -> int:
    """Never a bare count of a raw Iceberg table: `WHERE 1` makes ClickHouse
    scan, so equality deletes are subtracted (R11, docs/plans/P5-RESULT.md)."""
    body = run_query(api, f"SELECT count() AS n FROM {QUERY_DB}.`bronze.{table}` WHERE 1")
    return int(body["rows"][0]["n"])


def step_upload_and_preview(api, state: dict) -> tuple[str, dict, dict]:
    raw, expected = _fixture()
    resp = api.post(
        f"{API_URL}/api/uploads",
        # The misleading name and type are the point: see the module docstring.
        files={"file": (f"{FIXTURE}.xls", raw, "application/vnd.ms-excel")},
        timeout=60,
    )
    upload = _expect(resp, 201, "POST /api/uploads").json()
    upload_id = upload["id"]
    state["upload_id"] = upload_id
    if upload.get("status") != "uploaded" or upload.get("sizeBytes") != len(raw):
        raise G9Failure(f"the new upload should be `uploaded` with {len(raw)} bytes, got {upload}")
    if upload.get("duplicateOf"):
        print(f"[g9] note: this tenant already held these bytes (upload {upload['duplicateOf'].get('id')})")
    print(f"[g9] uploaded {FIXTURE}.xls as {upload_id} ({len(raw)} bytes)")

    preview = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}/preview", timeout=30), 200, "preview").json()
    detected = preview.get("detected") or {}
    want = {"encoding": expected["encoding"], "delimiter": expected["delimiter"], "headerRow": expected["headerRow"]}
    if detected != want:
        raise G9Failure(f"the preview detected {detected}, the fixture says {want}")
    if preview.get("columns") != expected["columns"] or preview.get("rows") != expected["rows"]:
        raise G9Failure(
            f"the preview shows columns {preview.get('columns')} and rows {preview.get('rows')}, "
            f"the fixture says {expected['columns']} and {expected['rows']}"
        )
    if preview.get("truncated"):
        raise G9Failure("a six-line file should not be previewed as truncated")
    print(f"[g9] preview: {detected}, columns {preview['columns']}, {len(preview['rows'])} rows")

    listed = _expect(api.get(f"{API_URL}/api/uploads", timeout=30), 200, "GET /api/uploads").json()
    if upload_id not in [u.get("id") for u in listed]:
        raise G9Failure("the new upload is not in this tenant's list")
    # What the console does with the preview: the guessed list goes back with Load (DATA-18).
    specs = preview.get("columnSpecs") or []
    if [c["source"] for c in specs] != expected["columns"]:
        raise G9Failure(f"the preview offers columns {[c.get('source') for c in specs]}, the fixture says {expected['columns']}")
    columns = [
        {k: c[k] for k in ("source", "name", "type", "dateFormat", "timezone") if k in c and c[k] is not None} for c in specs
    ]
    state["column_types"] = {c["name"]: c["type"] for c in columns}
    print(f"[g9] guessed types: {state['column_types']}")
    return upload_id, {**preview["using"], "columns": columns, "onBadValue": "stop"}, expected


def step_load(api, upload_id: str, table: str, mode: str, using: dict, file_rows: int, table_rows: int) -> None:
    resp = api.post(
        f"{API_URL}/api/uploads/{upload_id}/ingest",
        json={
            "bronzeTable": table,
            "mode": mode,
            "encoding": using["encoding"],
            "delimiter": using["delimiter"],
            "headerRow": using["headerRow"],
            # A workbook is loaded one sheet at a time; the API reads UTF-8 and
            # commas for it whatever `encoding` and `delimiter` say.
            **({"sheet": using["sheet"]} if using.get("sheet") else {}),
            # DATA-18: the column list, when the caller has one (the console always sends it).
            **({"columns": using["columns"], "onBadValue": using.get("onBadValue", "stop")} if using.get("columns") else {}),
        },
        timeout=60,
    )
    launched = _expect(resp, 200, f"POST .../ingest ({mode})").json()
    if not launched.get("runId") or launched["upload"].get("status") != "ingesting":
        raise G9Failure(f"an accepted load should return a run id and an `ingesting` upload, got {launched}")
    print(f"[g9] {mode}: launched run {launched['runId']}")

    def settled():
        upload = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "GET /api/uploads/{id}").json()
        print(f"[g9] upload {upload_id} status={upload.get('status')}")
        if upload.get("status") == "failed":
            raise G9Failure(f"the {mode} load failed: {upload.get('error')!r}")
        return upload if upload.get("status") == "ingested" else None

    upload = _wait_for(f"upload {upload_id} ingested", settled, LOAD_TIMEOUT_S)
    problems = []
    if upload.get("rows") != file_rows:
        problems.append(f"rows {upload.get('rows')!r}, expected {file_rows} (this load's own rows)")
    if upload.get("bronzeTable") != table:
        problems.append(f"bronzeTable {upload.get('bronzeTable')!r}, expected {table!r}")
    if upload.get("loadMode") != mode:
        problems.append(f"loadMode {upload.get('loadMode')!r}, expected {mode!r}")
    if upload.get("assetId") != table.replace("_", "-"):
        problems.append(f"assetId {upload.get('assetId')!r}, expected {table.replace('_', '-')!r}")
    if problems:
        raise G9Failure(f"the {mode} load settled with the wrong record: {'; '.join(problems)}")

    counted: list[int] = []

    def holds() -> bool:
        counted.append(count_rows(api, table))
        return counted[-1] == table_rows

    try:
        _wait_for(
            f"{QUERY_DB}.`bronze.{table}` holds {table_rows} rows", holds, VISIBLE_TIMEOUT_S, retry_failures=True
        )
    except G9Failure as exc:
        raise G9Failure(f"{exc}; the last count was {counted[-1] if counted else 'never read'}") from exc
    print(f"[g9] {mode}: the table holds {table_rows} rows (counted under a WHERE)")


# What ClickHouse calls the type each column type is stored as (`toTypeName`).
CLICKHOUSE_TYPE = {
    "text": "String",
    "integer": "Int64",
    "decimal": "Decimal(38, 9)",
    "boolean": "Bool",
    "date": "Date32",
    "timestamp": "DateTime64",
    "float": "Float64",
}


def step_rows_and_types(api, table: str, expected: dict, column_types: dict) -> None:
    columns = ["plnt", "material", "description", "qty"]
    body = run_query(api, f"SELECT {', '.join(columns)} FROM {QUERY_DB}.`bronze.{table}` WHERE 1 ORDER BY material, plnt")
    # Compared as text: `plnt` and `qty` are integers now, `material` keeps its leading zeros.
    got = [[str(row[c]) for c in columns] for row in body["rows"]]
    want = sorted(expected["rows"], key=lambda r: (r[1], r[0]))
    # The table holds the file twice after the append: compare the distinct rows.
    distinct = [list(r) for r in sorted({tuple(r) for r in got}, key=lambda r: (r[1], r[0]))]
    if distinct != want:
        raise G9Failure(f"the table holds {distinct}, the file says {want}")

    types = run_query(
        api,
        "SELECT "
        + ", ".join(f"toTypeName({c}) AS t_{c}" for c in columns)
        + f" FROM {QUERY_DB}.`bronze.{table}` WHERE 1 LIMIT 1",
    )["rows"][0]
    wrong = {
        c: (types[f"t_{c}"], CLICKHOUSE_TYPE[column_types[c]])
        for c in columns
        if CLICKHOUSE_TYPE[column_types[c]] not in types[f"t_{c}"]
    }
    if wrong:
        raise G9Failure(f"these columns are not the type the load was told (got, wanted): {wrong}")
    if column_types["material"] != "text" or "String" not in types["t_material"]:
        raise G9Failure("a column with leading zeros (`0000100`) must stay text, whatever else is typed")
    print(f"[g9] the rows are the file's own values and each column has the type it was told: {column_types}")


def step_one_ingest_run_per_load(api, upload_id: str, table: str, file_rows: int) -> None:
    resp = api.get(f"{API_URL}/api/governance/ingest-runs", params={"connectorId": f"upload:{upload_id}"}, timeout=30)
    runs = _expect(resp, 200, "GET /api/governance/ingest-runs").json()
    if len(runs) != 2:
        raise G9Failure(f"two loads should have left exactly two ingest_run rows, found {len(runs)}: {runs}")
    for run in runs:
        wrong = {
            key: run.get(key)
            for key, want in (("job", "file_ingest_job"), ("object", table), ("status", "succeeded"), ("rows", file_rows), ("error", ""))
            if run.get(key) != want
        }
        if wrong:
            raise G9Failure(f"an ingest_run row differs from what a successful load records: {wrong}")
    print("[g9] one ingest_run row per load, both succeeded with the rows loaded")


def step_delete_keeps_the_table(api, upload_id: str, table: str, table_rows: int, state: dict) -> None:
    _expect(api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 204, "DELETE /api/uploads/{id}")
    state["deleted"] = True
    _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 404, "GET of a deleted upload")
    listed = _expect(api.get(f"{API_URL}/api/uploads", timeout=30), 200, "GET /api/uploads").json()
    if upload_id in [u.get("id") for u in listed]:
        raise G9Failure("the deleted upload is still listed")
    left = count_rows(api, table)
    if left != table_rows:
        raise G9Failure(f"deleting the upload should keep the table's {table_rows} rows, it holds {left}")
    print(f"[g9] the upload is gone and the table keeps its {left} rows")


# SEC-17: the sentence for the column cap, as `routes/uploads.rs` words it
# (`JOB_TOO_MANY_COLUMNS`) and `file_ingest.py` records it (`TOO_MANY_COLUMNS`).
TOO_MANY_COLUMNS = "The file has more than 1,000 columns."


def step_column_cap(api) -> None:
    """A file of 1,001 columns is refused at the preview and at the load request,
    with the one sentence, and a delimiter that reads it as fewer columns
    previews again (a wrong guess must not lock a good file out). The 400 at the
    load request comes before any claim, mark or launch, so nothing is left
    behind but the upload, which is deleted."""
    header = ",".join(f"c{n}" for n in range(1_001))
    resp = api.post(
        f"{API_URL}/api/uploads",
        files={"file": ("g9_wide.csv", f"{header}\n1\n".encode(), "text/csv")},
        timeout=60,
    )
    upload_id = _expect(resp, 201, "POST /api/uploads (1,001 columns)").json()["id"]
    try:
        refused = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}/preview", timeout=30), 400, "preview of 1,001 columns")
        if refused.json().get("error") != TOO_MANY_COLUMNS:
            raise G9Failure(f"the preview's 400 should say {TOO_MANY_COLUMNS!r}, it said {refused.text[:200]}")
        again = _expect(
            api.get(f"{API_URL}/api/uploads/{upload_id}/preview", params={"delimiter": ";"}, timeout=30),
            200,
            "preview of the same file with another delimiter",
        ).json()
        if len(again.get("columns", [])) != 1:
            raise G9Failure(f"with ';' the file is one column, the preview shows {again.get('columns')}")
        body = {"bronzeTable": f"g9_wide_{secrets.token_hex(4)}", "encoding": "utf-8", "delimiter": ",", "headerRow": 0}
        ingest = _expect(api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json=body, timeout=30), 400, "ingest of 1,001 columns")
        if ingest.json().get("error") != TOO_MANY_COLUMNS:
            raise G9Failure(f"the ingest's 400 should say {TOO_MANY_COLUMNS!r}, it said {ingest.text[:200]}")
        print("[g9] 1,001 columns: refused at the preview and at the load request, one column under ';'")
    finally:
        # Never loaded, so the delete is allowed; a failed delete must not hide
        # the failure that matters, so it is reported and not raised.
        gone = api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30)
        if gone.status_code != 204:
            print(f"[g9] cleanup: DELETE of the wide upload answered {gone.status_code}", file=sys.stderr)


def step_workbook(api, base_table: str, state: dict) -> None:
    path = WORKBOOKS / WORKBOOK
    if not path.is_file():
        raise G9Failure(f"fixture missing: {path} (this gate reads ops/fixtures/workbooks/ beside it)")
    raw = path.read_bytes()
    state["upload_id"], state["deleted"] = None, False
    resp = api.post(
        f"{API_URL}/api/uploads",
        files={"file": (WORKBOOK, raw, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")},
        timeout=60,
    )
    upload = _expect(resp, 201, "POST /api/uploads (workbook)").json()
    upload_id = upload["id"]
    state["upload_id"] = upload_id
    workbook = upload.get("workbook") or {}
    if [sheet.get("name") for sheet in workbook.get("sheets", [])] != WORKBOOK_SHEETS or workbook.get("defaultSheet") != "Stock":
        raise G9Failure(f"the workbook's sheets should be {WORKBOOK_SHEETS} with Stock the default, got {workbook}")
    print(f"[g9] workbook: uploaded {WORKBOOK} as {upload_id}, sheets {WORKBOOK_SHEETS}")

    preview = _expect(
        api.get(f"{API_URL}/api/uploads/{upload_id}/preview", params={"sheet": "Stock"}, timeout=60), 200, "workbook preview"
    ).json()
    if preview.get("columns") != WORKBOOK_COLUMNS or preview.get("rows") != WORKBOOK_ROWS:
        raise G9Failure(f"the workbook preview shows {preview.get('columns')} and {preview.get('rows')}")
    if (preview.get("workbook") or {}).get("sheet") != "Stock":
        raise G9Failure(f"the preview should say it read the Stock sheet, got {preview.get('workbook')}")
    using = {**preview["using"], "sheet": "Stock"}

    table = f"{base_table}_xlsx"
    step_load(api, upload_id, table, "replace", using, len(WORKBOOK_ROWS), table_rows=len(WORKBOOK_ROWS))
    loaded = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "GET /api/uploads/{id}").json()
    if (loaded.get("parseOptions") or {}).get("sheet") != "Stock":
        raise G9Failure(f"the load should record the sheet it read, parseOptions is {loaded.get('parseOptions')}")

    body = run_query(
        api, f"SELECT {', '.join(WORKBOOK_COLUMNS)} FROM {QUERY_DB}.`bronze.{table}` WHERE 1 ORDER BY sku"
    )
    got = [[row[c] for c in WORKBOOK_COLUMNS] for row in body["rows"]]
    if got != WORKBOOK_ROWS:
        raise G9Failure(f"the table holds {got}, the sheet says {WORKBOOK_ROWS}")
    types = run_query(
        api,
        "SELECT " + ", ".join(f"toTypeName({c}) AS t_{c}" for c in WORKBOOK_COLUMNS)
        + f" FROM {QUERY_DB}.`bronze.{table}` WHERE 1 LIMIT 1",
    )["rows"][0]
    not_text = {c: types[f"t_{c}"] for c in WORKBOOK_COLUMNS if "String" not in types[f"t_{c}"]}
    if not_text:
        raise G9Failure(f"every column of a workbook load should be text, these are not: {not_text}")
    print("[g9] workbook: the sheet's rows are in the table, dates and numbers as text")

    resp = api.get(f"{API_URL}/api/governance/ingest-runs", params={"connectorId": f"upload:{upload_id}"}, timeout=30)
    runs = _expect(resp, 200, "GET /api/governance/ingest-runs (workbook)").json()
    if len(runs) != 1 or runs[0].get("status") != "succeeded" or runs[0].get("rows") != len(WORKBOOK_ROWS):
        raise G9Failure(f"the workbook load should have left one succeeded ingest_run row, found {runs}")

    _expect(api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 204, "DELETE the workbook upload")
    state["deleted"] = True
    _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 404, "GET of the deleted workbook upload")
    print("[g9] workbook: PASS (the upload is gone, the table stays)")


def step_parquet(api, base_table: str, state: dict) -> None:
    path = PARQUETS / PARQUET
    if not path.is_file():
        raise G9Failure(f"fixture missing: {path} (this gate reads ops/fixtures/parquet/ beside it)")
    raw = path.read_bytes()
    names = [c["name"] for c in PARQUET_COLUMNS]
    state["upload_id"], state["deleted"] = None, False
    resp = api.post(
        f"{API_URL}/api/uploads",
        files={"file": (PARQUET, raw, "application/octet-stream")},
        timeout=60,
    )
    upload = _expect(resp, 201, "POST /api/uploads (parquet)").json()
    upload_id = upload["id"]
    state["upload_id"] = upload_id
    parquet = upload.get("parquet") or {}
    if parquet.get("columns") != PARQUET_COLUMNS or parquet.get("rows") != len(PARQUET_ROWS):
        raise G9Failure(f"the Parquet file's footer should list {PARQUET_COLUMNS} and {len(PARQUET_ROWS)} rows, got {parquet}")
    print(f"[g9] parquet: uploaded {PARQUET} as {upload_id}, columns {names}")

    preview = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}/preview", timeout=60), 200, "parquet preview").json()
    if preview.get("columns") != names or preview.get("rows") != PARQUET_ROWS:
        raise G9Failure(f"the Parquet preview shows {preview.get('columns')} and {preview.get('rows')}")
    if (preview.get("parquet") or {}).get("columns") != PARQUET_COLUMNS:
        raise G9Failure(f"the preview should carry the declared types, got {preview.get('parquet')}")

    table = f"{base_table}_parquet"
    # No dialect, header row or sheet: none of them is read for a Parquet file.
    resp = api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json={"bronzeTable": table, "mode": "replace"}, timeout=60)
    launched = _expect(resp, 200, "POST .../ingest (parquet)").json()
    if not launched.get("runId") or launched["upload"].get("status") != "ingesting":
        raise G9Failure(f"an accepted load should return a run id and an `ingesting` upload, got {launched}")
    print(f"[g9] parquet: launched run {launched['runId']}")

    def settled():
        current = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "GET /api/uploads/{id}").json()
        print(f"[g9] upload {upload_id} status={current.get('status')}")
        if current.get("status") == "failed":
            raise G9Failure(f"the Parquet load failed: {current.get('error')!r}")
        return current if current.get("status") == "ingested" else None

    loaded = _wait_for(f"upload {upload_id} ingested", settled, LOAD_TIMEOUT_S)
    if loaded.get("rows") != len(PARQUET_ROWS) or loaded.get("bronzeTable") != table:
        raise G9Failure(f"the Parquet load settled with the wrong record: rows {loaded.get('rows')!r}, table {loaded.get('bronzeTable')!r}")
    if loaded.get("parseOptions") != {"encoding": "utf-8", "delimiter": ",", "headerRow": 0}:
        raise G9Failure(f"a Parquet load records the fixed dialect and no sheet, parseOptions is {loaded.get('parseOptions')}")

    body = run_query(api, f"SELECT {', '.join(names)} FROM {QUERY_DB}.`bronze.{table}` WHERE 1 ORDER BY sku")
    got = [[row[c] for c in names] for row in body["rows"]]
    if got != PARQUET_ROWS:
        raise G9Failure(f"the table holds {got}, the file says {PARQUET_ROWS}")
    types = run_query(
        api,
        "SELECT " + ", ".join(f"toTypeName({c}) AS t_{c}" for c in names)
        + f" FROM {QUERY_DB}.`bronze.{table}` WHERE 1 LIMIT 1",
    )["rows"][0]
    not_text = {c: types[f"t_{c}"] for c in names if "String" not in types[f"t_{c}"]}
    if not_text:
        raise G9Failure(f"every column of a Parquet load should be text, these are not: {not_text}")
    print("[g9] parquet: the file's rows are in the table, every column as text")

    resp = api.get(f"{API_URL}/api/governance/ingest-runs", params={"connectorId": f"upload:{upload_id}"}, timeout=30)
    runs = _expect(resp, 200, "GET /api/governance/ingest-runs (parquet)").json()
    if len(runs) != 1 or runs[0].get("status") != "succeeded" or runs[0].get("rows") != len(PARQUET_ROWS):
        raise G9Failure(f"the Parquet load should have left one succeeded ingest_run row, found {runs}")

    _expect(api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 204, "DELETE the Parquet upload")
    state["deleted"] = True
    _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 404, "GET of the deleted Parquet upload")
    print("[g9] parquet: PASS (the upload is gone, the table stays)")


def _dagster_lists_job(requests, job_name: str) -> bool:
    """Whether the code location's job list holds `job_name`."""
    query = "{ repositoriesOrError { ... on RepositoryConnection { nodes { jobs { name } } } } }"
    resp = requests.post(DAGSTER_URL, json={"query": query}, timeout=10)
    resp.raise_for_status()
    nodes = (resp.json().get("data") or {}).get("repositoriesOrError", {}).get("nodes") or []
    return any(job.get("name") == job_name for node in nodes for job in node.get("jobs", []))


# DATA-18 (T3.6): columns with types. Invented rows; `007` is the postcode-like value that must stay text.
TYPED_CSV = (
    "id,price,when_at,active,code,qty\n"
    "1,1.50,2026-10-16,yes,007,5\n"
    "2,12,2026-10-17,no,008,6\n"
    "3,0.25,2026-10-18,yes,009,7\n"
)
# The same file with one value that is not an integer.
TYPED_CSV_BAD = TYPED_CSV.replace(",007,5\n", ",007,five\n", 1)
TYPED_GUESS = {"id": "integer", "price": "decimal", "when_at": "date", "active": "boolean", "code": "text", "qty": "integer"}
BAD_VALUES_REASON = "Some values do not fit their column's type."


def _upload_text(api, name: str, text: str, state: dict) -> str:
    resp = api.post(f"{API_URL}/api/uploads", files={"file": (name, text.encode(), "text/csv")}, timeout=60)
    upload_id = _expect(resp, 201, f"POST /api/uploads ({name})").json()["id"]
    state["upload_id"], state["deleted"] = upload_id, False
    return upload_id


def _settle(api, upload_id: str, what: str) -> dict:
    """Wait for a load to leave `ingesting`; returns the settled upload (ingested OR failed)."""

    def done():
        upload = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "GET /api/uploads/{id}").json()
        return upload if upload.get("status") in ("ingested", "failed") else None

    return _wait_for(f"{what}: upload {upload_id} settled", done, LOAD_TIMEOUT_S)


def _typed_body(table: str, columns: list, on_bad: str, mode: str = "replace") -> dict:
    return {
        "bronzeTable": table,
        "mode": mode,
        "encoding": "utf-8",
        "delimiter": ",",
        "headerRow": 0,
        "columns": columns,
        "onBadValue": on_bad,
    }


def _types_of(api, table: str, columns: list[str]) -> dict:
    row = run_query(
        api,
        "SELECT "
        + ", ".join(f"toTypeName({c}) AS t_{c}" for c in columns)
        + f" FROM {QUERY_DB}.`bronze.{table}` WHERE 1 LIMIT 1",
    )["rows"][0]
    return {c: row[f"t_{c}"] for c in columns}


def step_typed_columns(api, base_table: str, state: dict) -> None:
    """The column types end to end (DATA-18 AC1, AC5, AC6, AC7, AC9 and decision 9).

    1. The preview guesses `id` and `qty` integers, `price` a decimal, `when_at` an
       ISO date, `active` a boolean, and leaves `code` (`007`) text.
    2. The check lists the one value of a column made Integer that does not fit.
    3. A typed load: the table's types are those told (Int64, Decimal(38, 9), Date32,
       Bool, String), a sum and a date-range filter work on them, and `007` is intact.
    4. A load told to STOP at a value that does not fit fails with the fixed reason,
       the report, and the table unchanged.
    5. The same told to load it as EMPTY succeeds with the report, and the cell is NULL.
    6. A REPLACE that changes the types (every column text, no list: decision 11) works
       and the table's columns are text afterwards: the case dlt and Iceberg refuse
       unless the table is dropped between the normalize and the load.
    """
    table = f"{base_table}_typed"
    columns_names = list(TYPED_GUESS)
    upload_id = _upload_text(api, "typed.csv", TYPED_CSV, state)
    preview = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}/preview", timeout=30), 200, "typed preview").json()
    guessed = {c["name"]: c["type"] for c in preview.get("columnSpecs", [])}
    if guessed != TYPED_GUESS:
        raise G9Failure(f"the preview should guess {TYPED_GUESS}, it guessed {guessed}")
    specs = [
        {k: c[k] for k in ("source", "name", "type", "dateFormat", "timezone") if k in c and c[k] is not None}
        for c in preview["columnSpecs"]
    ]
    print(f"[g9] typed: guessed {guessed}")

    as_integer = [dict(c, type="integer") if c["name"] == "code" else c for c in specs]
    as_integer = [{k: v for k, v in c.items() if k not in ("dateFormat", "timezone") or c["type"] in ("date", "timestamp")} for c in as_integer]
    checked = _expect(
        api.post(f"{API_URL}/api/uploads/{upload_id}/check", json={"columns": as_integer}, timeout=30), 200, "check"
    ).json()
    if checked.get("count") != 0:
        raise G9Failure(f"`007`, `008` and `009` fit Integer, the check should list nothing: {checked}")
    bad_check = [dict(c, type="date", dateFormat="iso") if c["name"] == "qty" else c for c in specs]
    refused = _expect(
        api.post(f"{API_URL}/api/uploads/{upload_id}/check", json={"columns": bad_check}, timeout=30), 200, "check (dates)"
    ).json()
    if refused.get("count") != 3 or refused["rows"][0]["column"] != "qty":
        raise G9Failure(f"qty read as dates should fail three values, got {refused}")

    resp = api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json=_typed_body(table, specs, "stop"), timeout=60)
    _expect(resp, 200, "typed ingest")
    done = _settle(api, upload_id, "typed load")
    if done["status"] != "ingested" or done.get("rows") != 3 or done.get("badValues"):
        raise G9Failure(f"the typed load should be ingested with 3 rows and nothing that did not fit: {done}")
    _wait_for(f"{table} holds 3 rows", lambda: count_rows(api, table) == 3, VISIBLE_TIMEOUT_S, retry_failures=True)
    types = _types_of(api, table, columns_names)
    for name, kind in TYPED_GUESS.items():
        if CLICKHOUSE_TYPE[kind] not in types[name]:
            raise G9Failure(f"column {name} should be {CLICKHOUSE_TYPE[kind]}, ClickHouse says {types[name]}")
    totals = run_query(
        api,
        f"SELECT toString(sum(price)) AS total, toString(count()) AS late FROM {QUERY_DB}.`bronze.{table}` "
        "WHERE when_at >= toDate('2026-10-17')",
    )["rows"][0]
    if totals["late"] != "2":
        raise G9Failure(f"two rows are on or after 2026-10-17, the date filter says {totals}")
    codes = sorted(r["code"] for r in run_query(api, f"SELECT code FROM {QUERY_DB}.`bronze.{table}` WHERE 1")["rows"])
    if codes != ["007", "008", "009"]:
        raise G9Failure(f"the leading zeros must survive as text, got {codes}")
    asset = api.get(f"{API_URL}/api/catalog/{table.replace('_', '-')}", timeout=30)
    if asset.ok and isinstance(asset.json().get("columns"), list) and all(isinstance(c, dict) for c in asset.json()["columns"]):
        in_catalog = {c.get("name"): str(c.get("type")) for c in asset.json()["columns"]}
        wrong = {n: in_catalog.get(n) for n, k in TYPED_GUESS.items() if n in in_catalog and CLICKHOUSE_TYPE[k] not in in_catalog[n]}
        if wrong:
            raise G9Failure(f"the Catalog shows other types than the table holds: {wrong}")
    else:
        print(f"[g9] typed: the Catalog answered {asset.status_code} or in a shape this gate does not read; types asserted through the query route only")
    print(f"[g9] typed: {table} holds {types}, a date filter and the leading zeros work")
    _delete_upload(api, upload_id, state)

    bad_id = _upload_text(api, "typed_bad.csv", TYPED_CSV_BAD, state)
    resp = api.post(f"{API_URL}/api/uploads/{bad_id}/ingest", json=_typed_body(table, specs, "stop"), timeout=60)
    _expect(resp, 200, "typed ingest (stop)")
    stopped = _settle(api, bad_id, "the load told to stop")
    report = stopped.get("badValues") or {}
    if stopped["status"] != "failed" or stopped.get("error") != BAD_VALUES_REASON or report.get("count") != 1:
        raise G9Failure(f"a load told to stop should fail with {BAD_VALUES_REASON!r} and one bad value: {stopped}")
    if report["rows"][0] != {"row": 1, "column": "qty", "value": "five"}:
        raise G9Failure(f"the report should name row 1, qty, 'five': {report}")
    if count_rows(api, table) != 3 or CLICKHOUSE_TYPE["integer"] not in _types_of(api, table, ["qty"])["qty"]:
        raise G9Failure("a load told to stop must leave the table as it was")
    print("[g9] typed: told to stop, the load failed with the report and the table was left alone")

    resp = api.post(f"{API_URL}/api/uploads/{bad_id}/ingest", json=_typed_body(table, specs, "null"), timeout=60)
    _expect(resp, 200, "typed ingest (empty)")
    emptied = _settle(api, bad_id, "the load told to load as empty")
    if emptied["status"] != "ingested" or (emptied.get("badValues") or {}).get("count") != 1:
        raise G9Failure(f"told to load it as empty, the load should succeed with the report: {emptied}")
    nulls = run_query(api, f"SELECT toString(count()) AS n FROM {QUERY_DB}.`bronze.{table}` WHERE qty IS NULL")["rows"][0]["n"]
    if nulls != "1":
        raise G9Failure(f"exactly one qty should be empty, got {nulls}")
    print("[g9] typed: told to load as empty, the cell is NULL and the report says so")

    text_only = {k: v for k, v in _typed_body(table, specs, "stop").items() if k not in ("columns", "onBadValue")}
    resp = api.post(f"{API_URL}/api/uploads/{bad_id}/ingest", json=text_only, timeout=60)
    _expect(resp, 200, "text ingest over a typed table")
    retyped = _settle(api, bad_id, "the replace that changes the types")
    if retyped["status"] != "ingested" or retyped.get("badValues"):
        raise G9Failure(f"a replace with no column list should load as text and report nothing: {retyped}")
    _wait_for(
        f"{table} is text again",
        lambda: all("String" in t for t in _types_of(api, table, columns_names).values()),
        VISIBLE_TIMEOUT_S,
        retry_failures=True,
    )
    print("[g9] typed: a replace with no column list changed every column to text (decision 9, 11)")
    _delete_upload(api, bad_id, state)


def _delete_upload(api, upload_id: str, state: dict) -> None:
    _expect(api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 204, "DELETE upload")
    state["deleted"] = True


# DATA-17 (slice 4): JSON, JSON Lines, Avro, gzip and zip. The files are the shared fixtures of
# ops/fixtures/uploads/formats/ (the API's preview tests and the job's tests read them too).
FORMATS_DIR = FIXTURES / "formats"
NEW_FORMATS = [
    # file, layout the preview reports, the dialect the load is told (None: JSON and Avro have none)
    ("orders.json", {"format": "json", "compression": "none"}, None),
    ("orders.jsonl", {"format": "jsonl", "compression": "none"}, None),
    ("orders.avro", {"format": "avro", "compression": "none"}, None),
    ("orders.csv.gz", {"format": "delimited", "compression": "gzip"}, "dialect"),
    ("three.zip", {"format": "delimited", "compression": "zip"}, "dialect"),
]
UNPACKED_SENTENCE = "The file is larger than the upload limit once unpacked."


def step_new_formats(api, base_table: str, state: dict) -> None:
    """One small file per format loads and reads back (DATA-17 AC6, AC7), and a gzip and a zip
    bomb are refused and stored nowhere (decision 2): no column list, so every column is text
    and a nested value is JSON text (decision 11)."""
    for index, (name, layout, dialect) in enumerate(NEW_FORMATS):
        expected = json.loads((FORMATS_DIR / f"{name}.expected.json").read_text(encoding="utf-8"))
        names = expected["columns"]
        raw = (FORMATS_DIR / name).read_bytes()
        state["upload_id"], state["deleted"] = None, False
        resp = api.post(f"{API_URL}/api/uploads", files={"file": (name, raw, "application/octet-stream")}, timeout=60)
        upload_id = _expect(resp, 201, f"POST /api/uploads ({name})").json()["id"]
        state["upload_id"] = upload_id
        preview = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}/preview", timeout=60), 200, f"{name} preview").json()
        if preview.get("layout") != layout or preview.get("columns") != names:
            raise G9Failure(f"{name}: the preview should say {layout} and list {names}, got {preview.get('layout')} and {preview.get('columns')}")
        if name == "three.zip":
            files = [f["name"] for f in preview["archive"]["files"]]
            if files != expected["entries"] or not all(f["matchesFirst"] for f in preview["archive"]["files"]):
                raise G9Failure(f"{name}: the preview should list {expected['entries']} with matching columns, got {preview.get('archive')}")
        if name == "orders.avro":
            kinds = [c["type"] for c in preview["columnSpecs"]]
            if kinds != expected["declared"]:
                raise G9Failure(f"{name}: the schema's types are the starting guess {expected['declared']}, got {kinds}")

        table = f"{base_table}_f{index}"
        body = {"bronzeTable": table, "mode": "replace"}
        if dialect:
            using = preview["using"]
            body.update(encoding=using["encoding"], delimiter=using["delimiter"], headerRow=using["headerRow"])
        launched = _expect(api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json=body, timeout=60), 200, f"POST .../ingest ({name})").json()
        if not launched.get("runId"):
            raise G9Failure(f"{name}: an accepted load should return a run id, got {launched}")

        def settled():
            current = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "GET /api/uploads/{id}").json()
            if current.get("status") == "failed":
                raise G9Failure(f"{name}: the load failed: {current.get('error')!r}")
            return current if current.get("status") == "ingested" else None

        loaded = _wait_for(f"{name} ingested", settled, LOAD_TIMEOUT_S)
        if loaded.get("rows") != len(expected["rows"]) or loaded.get("bronzeTable") != table:
            raise G9Failure(f"{name}: the load settled with rows {loaded.get('rows')!r} in {loaded.get('bronzeTable')!r}, expected {len(expected['rows'])} in {table!r}")
        _wait_for(f"{table} holds its rows", lambda: count_rows(api, table) == len(expected["rows"]), VISIBLE_TIMEOUT_S, retry_failures=True)
        # PR #186 review BLOCKER 4: `/api/query/run` answers every cell as text and renders a
        # NULL as "" (`stringify_cell`), so a value read through it cannot tell a missing key
        # (decision 11: null) from an empty string. Each column is asked `isNull` beside its
        # value, and a cell is None only where ClickHouse says it is NULL. Nothing is loosened:
        # a table that held "" where the fixture says null still fails here.
        flags = ", ".join(f"isNull({c}) AS is_null_{i}" for i, c in enumerate(names))
        got = run_query(api, f"SELECT {', '.join(names)}, {flags} FROM {QUERY_DB}.`bronze.{table}` WHERE 1")["rows"]
        got_rows = sorted(
            ([None if r[f"is_null_{i}"] == "1" else r[c] for i, c in enumerate(names)] for r in got),
            key=lambda row: str(row[0]),
        )
        want_rows = sorted(expected["rows"], key=lambda row: str(row[0]))
        types = _types_of(api, table, names)
        if got_rows != want_rows:
            raise G9Failure(f"{name}: the table holds {got_rows}, the fixture says {want_rows}; the columns are {types}")
        print(f"[g9] formats: {name} columns are {types}")
        print(f"[g9] formats: {name} loaded {len(want_rows)} rows into {table}")
        _expect(api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 204, f"DELETE {name}")
        state["deleted"] = True

    # Bombs: a few KiB of gzip or zip that unpack past the 50 MB cap are refused, and nothing is kept.
    before = len(_expect(api.get(f"{API_URL}/api/uploads", timeout=30), 200, "GET /api/uploads").json())
    big = b"id,name\n" + b"a" * (51 * 1024 * 1024)
    zipped = io.BytesIO()
    with zipfile.ZipFile(zipped, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("a.csv", big)
    for name, raw in (("bomb.csv.gz", gzip.compress(big, compresslevel=9)), ("bomb.zip", zipped.getvalue())):
        if len(raw) > 1024 * 1024:
            raise G9Failure(f"{name}: the setup should be a small file, it is {len(raw)} bytes")
        resp = api.post(f"{API_URL}/api/uploads", files={"file": (name, raw, "application/octet-stream")}, timeout=60)
        refused = _expect(resp, 400, f"POST /api/uploads ({name})").json()
        if refused.get("error") != UNPACKED_SENTENCE:
            raise G9Failure(f"{name}: a bomb should be refused with {UNPACKED_SENTENCE!r}, got {refused}")
    after = len(_expect(api.get(f"{API_URL}/api/uploads", timeout=30), 200, "GET /api/uploads").json())
    if after != before:
        raise G9Failure(f"a refused bomb must leave no upload behind, the list went from {before} to {after}")
    print("[g9] formats: a gzip bomb and a zip bomb were refused and nothing was stored")


def step_create_pipeline_over_uploaded_table(api, requests, table: str, silver_table: str) -> str:
    """Author a `bronze.<table>` -> `silver.<silver_table>` pipeline whose
    `dependsOn` names `file_ingest_job`, exactly the body the console's
    "Run again after this table is re-uploaded" switch sends
    (`src/features/pipelines/pipeline-create-page.tsx`, `DATA-19` D3).

    `schedule` is `manual` on purpose: every run this gate cares about is
    either triggered explicitly or launched by the sensor, and a cron would
    add runs nobody asked for and the "did not run again" step could no
    longer tell them apart."""
    resp = api.post(
        f"{API_URL}/api/pipelines",
        json={
            "name": f"g9_pipeline_{secrets.token_hex(3)}",
            "kind": "batch",
            "sourceZone": "bronze",
            "sourceTable": table,
            "incrementalColumn": "",
            "transforms": [],
            "targetZone": "silver",
            "targetTable": silver_table,
            "schedule": "manual",
            "dependsOn": ["file_ingest_job"],
        },
        timeout=60,
    )
    created = _expect(resp, 201, "POST /api/pipelines").json()
    pipeline_id = created.get("id")
    if not pipeline_id or created.get("source") != f"bronze.{table}":
        raise G9Failure(f"the created pipeline should name bronze.{table}, got {created}")
    print(f"[g9] pipeline {pipeline_id}: bronze.{table} -> silver.{silver_table}, depends on file_ingest_job")

    # An authored pipeline is a `draft` until it is activated, and only a
    # `ready` one is built into a job with a sensor.
    activated = _expect(
        api.post(f"{API_URL}/api/pipelines/{pipeline_id}/status", json={"status": "ready"}, timeout=60),
        200,
        "POST /api/pipelines/{id}/status",
    ).json()
    if activated.get("status") != "ready":
        raise G9Failure(f"the pipeline should be ready after activation, got {activated.get('status')!r}")

    if not DAGSTER_URL:
        raise G9Failure(
            "step 10 needs DAGSTER_URL: without the orchestrator there is no job to trigger "
            "and no sensor to fire, so this step would prove nothing"
        )
    job_name = f"authored__{pipeline_id.replace('-', '_')}"
    _wait_for(f"the orchestrator built {job_name}", lambda: _dagster_lists_job(requests, job_name), JOB_BUILT_TIMEOUT_S)
    print(f"[g9] the orchestrator built {job_name}")
    return pipeline_id


def _pipeline_runs(api, pipeline_id: str) -> list[dict]:
    body = _expect(
        api.get(f"{API_URL}/api/pipelines/{pipeline_id}/runs", timeout=60), 200, "GET /api/pipelines/{id}/runs"
    ).json()
    if body.get("unavailable"):
        raise G9Failure(f"the orchestrator could not be asked for runs: {body['unavailable']}")
    return body.get("runs", [])


def _succeeded(api, pipeline_id: str) -> list[dict]:
    return [r for r in _pipeline_runs(api, pipeline_id) if r.get("status") == RUN_COMPLETED]


def step_first_pipeline_run(api, pipeline_id: str, silver_table: str, table_rows: int) -> None:
    """Trigger one run and wait for it, then read the Silver table it
    wrote. The trigger is what makes the later "it ran again on its own"
    step meaningful: without a prior run there is no `start_time` for the
    sensor's freshness rule to beat."""
    triggered = _expect(
        api.post(f"{API_URL}/api/pipelines/{pipeline_id}/trigger", json={}, timeout=60),
        200,
        "POST /api/pipelines/{id}/trigger",
    ).json()
    print(f"[g9] pipeline: triggered run {triggered.get('runId') or triggered.get('id')}")

    def succeeded() -> bool:
        runs = _pipeline_runs(api, pipeline_id)
        failed = [r for r in runs if r.get("status") == RUN_FAILED]
        if failed:
            raise G9Failure(f"the authored run failed: {failed[0]}")
        return [r for r in runs if r.get("status") == RUN_COMPLETED]

    try:
        _wait_for("the authored pipeline run succeeded", succeeded, PIPELINE_RUN_TIMEOUT_S)
    except G9Failure as exc:
        # The statuses are what a reader needs: "completed" is the API's
        # word for a successful run (`map_run_status`), and a run stuck in
        # `running` says the wait, not this gate, was the problem.
        statuses = [r.get("status") for r in _pipeline_runs(api, pipeline_id)]
        raise G9Failure(f"{exc}; the pipeline's runs were {statuses}") from exc

    counted: list[int] = []

    def holds() -> bool:
        counted.append(count_silver_rows(api, silver_table))
        return counted[-1] == table_rows

    try:
        _wait_for(f"silver.{silver_table} holds {table_rows} rows", holds, VISIBLE_TIMEOUT_S, retry_failures=True)
    except G9Failure as exc:
        raise G9Failure(f"{exc}; the last count was {counted[-1] if counted else 'never read'}") from exc
    print(f"[g9] pipeline: silver.{silver_table} holds {table_rows} rows, read from its Bronze source")


def count_silver_rows(api, silver_table: str) -> int:
    """Every count in this gate runs under a `WHERE` (R11). A Silver table
    is a `ClickHouse` `MergeTree`, but the rule costs nothing and the
    Bronze ones beside it depend on it."""
    body = run_query(api, f"SELECT count() AS n FROM silver.`{silver_table}` WHERE 1")
    return int(body["rows"][0]["n"])


def step_reupload_reruns_the_pipeline(api, table: str, silver_table: str, pipeline_id: str, using: dict, expected: dict, state: dict) -> str:
    """Re-upload the SAME table with one row changed and expect the
    pipeline to run again by itself.

    The changed file is built from the fixture itself (same encoding,
    delimiter and header row, one cell changed), so the second load is the
    first one with different data -- exactly what a user re-uploading a
    corrected export does."""
    upload_id = upload_changed_copy(api, expected, state)
    print(f"[g9] re-upload: {upload_id} into {table} (one row changed)")
    step_load(api, upload_id, table, "replace", using, len(expected["rows"]), table_rows=len(expected["rows"]))

    before = len(_succeeded(api, pipeline_id))

    def reran() -> bool:
        return len(_succeeded(api, pipeline_id)) > before

    _wait_for("the pipeline ran again after the re-upload", reran, PIPELINE_RUN_TIMEOUT_S)
    after = _succeeded(api, pipeline_id)
    print(f"[g9] the pipeline ran again without being triggered ({before} -> {len(after)} successful runs)")

    # And the change is in the Silver table: the fixture's own row, with
    # the quantity the changed file carries.
    qty = CHANGED_QTY
    columns = ["plnt", "material", "description", "qty"]

    def shows_change() -> bool:
        body = run_query(
            api,
            f"SELECT {', '.join(columns)} FROM silver.`{silver_table}` WHERE 1 ORDER BY material",
        )
        got = [[row[c] for c in columns] for row in body["rows"]]
        return any(row[3] == qty for row in got)

    try:
        _wait_for(f"silver.{silver_table} shows the changed row ({qty})", shows_change, VISIBLE_TIMEOUT_S, retry_failures=True)
    except G9Failure as exc:
        body = run_query(api, f"SELECT {', '.join(columns)} FROM silver.`{silver_table}` WHERE 1 ORDER BY material")
        raise G9Failure(f"{exc}; the table holds {body['rows']}") from exc
    print(f"[g9] silver.{silver_table} shows the re-uploaded row")
    _delete_upload(api, upload_id, state)
    return upload_id


# The quantity the changed copy of `sap_report_utf16.xls` carries in the
# row the gate asserts on. The original says `4`; the changed copy says
# `9001`, which no other fixture and no earlier step mentions.
CHANGED_QTY = "9001"
# The last data row of the fixture, as tab-separated cells.
_CHANGED_ROW = ["8250", "0000200", "PP BAND", CHANGED_QTY]


def changed_fixture_bytes(expected: dict) -> bytes:
    """`sap_report_utf16.xls` with its last data row's quantity changed.

    Built from the fixture's OWN expected rows, so the header, the title
    lines, the encoding, the BOM and the CRLF endings stay exactly as the
    preview detected them: the only difference a re-upload introduces here
    is the cell the gate asserts on. Re-encoded as UTF-16 LE with the BOM
    the fixture starts with, which is what makes the API's preview detect
    `utf-16` again."""
    raw, _expected = _fixture()
    text = raw.decode("utf-16")
    lines = text.split("\r\n")
    header = expected["columns"]
    # The header cells are the fixture's own, lower-cased by the loader;
    # the changed row is matched on its position in the file (the last
    # non-blank record), not on a guess about its content.
    for index in range(len(lines) - 1, -1, -1):
        cells = lines[index].split("\t")
        if len(cells) == len(header) and cells[0] == expected["rows"][-1][0] and cells[1] == expected["rows"][-1][1]:
            lines[index] = "\t".join(_CHANGED_ROW)
            return b"\xff\xfe" + "\r\n".join(lines).encode("utf-16-le")
    raise G9Failure("the fixture's last data row could not be found, so it cannot be changed")


def upload_changed_copy(api, expected: dict, state: dict) -> str:
    """Upload the changed copy of the fixture as a new upload of its own
    (the same bytes would be refused as a duplicate)."""
    raw = changed_fixture_bytes(expected)
    resp = api.post(
        f"{API_URL}/api/uploads",
        files={"file": (f"{FIXTURE}_corrected.xls", raw, "application/vnd.ms-excel")},
        timeout=60,
    )
    upload = _expect(resp, 201, "POST /api/uploads (corrected copy)").json()
    state["upload_id"] = upload["id"]
    state["deleted"] = False
    return upload["id"]


def _delete_upload(api, upload_id: str, state: dict) -> None:
    _expect(api.delete(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 204, "DELETE /api/uploads/{id}")
    if state.get("upload_id") == upload_id:
        state["deleted"] = True


def step_other_table_does_not_rerun(api, table: str, pipeline_id: str, state: dict) -> None:
    """Load a file into a DIFFERENT table and expect no run.

    This is the step that distinguishes "the pipeline runs after an
    upload" from "the pipeline runs after a re-upload of ITS OWN table":
    without the per-table filter in the sensor, this load would fire the
    chain just as the re-upload did."""
    other = f"{table}_other"
    path = PARQUETS / PARQUET
    if not path.is_file():
        raise G9Failure(f"fixture missing: {path}")
    raw = path.read_bytes()
    resp = api.post(
        f"{API_URL}/api/uploads",
        files={"file": (PARQUET, raw, "application/octet-stream")},
        timeout=60,
    )
    upload_id = _expect(resp, 201, "POST /api/uploads (other table)").json()["id"]
    state["upload_id"], state["deleted"] = upload_id, False
    launched = _expect(
        api.post(f"{API_URL}/api/uploads/{upload_id}/ingest", json={"bronzeTable": other, "mode": "replace"}, timeout=60),
        200,
        f"POST .../ingest (into {other})",
    ).json()
    if not launched.get("runId"):
        raise G9Failure(f"the other table's load should have launched, got {launched}")

    def settled() -> bool:
        current = _expect(api.get(f"{API_URL}/api/uploads/{upload_id}", timeout=30), 200, "GET /api/uploads/{id}").json()
        if current.get("status") == "failed":
            raise G9Failure(f"the {other} load failed: {current.get('error')!r}")
        return current.get("status") == "ingested"

    _wait_for(f"upload {upload_id} into {other} ingested", settled, LOAD_TIMEOUT_S)

    before = len(_succeeded(api, pipeline_id))
    print(f"[g9] other table: {other} loaded; watching {NO_RETRIGGER_WAIT_S}s for a run that must not come")
    time.sleep(NO_RETRIGGER_WAIT_S)
    after = len(_succeeded(api, pipeline_id))
    if after != before:
        raise G9Failure(
            f"loading {other} ran the pipeline ({before} -> {after} successful runs); "
            "only a re-upload of the pipeline's own source table may trigger it"
        )
    print(f"[g9] other table: the pipeline did not run again ({after} successful runs, unchanged)")
    _delete_upload(api, upload_id, state)


def step_lineage_shows_the_upload(api, upload_id: str, table: str, silver_table: str) -> None:
    """`GET /api/governance/lineage` for the Silver table must show the
    upload that made its Bronze source: `upload:<id>` -> `bronze.<table>`
    -> `silver.<silver_table>`. The first edge is DATA-19's own
    (`upload_table_claim`); the second is the authored pipeline's."""
    resp = api.get(f"{API_URL}/api/governance/lineage", params={"focus": f"silver.{silver_table}"}, timeout=60)
    body = _expect(resp, 200, "GET /api/governance/lineage").json()
    wanted = [
        (f"upload:{upload_id}", f"bronze:{table}", "upload"),
        (f"bronze:{table}", f"table:silver.{silver_table}", "pipeline"),
    ]
    missing = [
        f"{f} -> {t} [{k}]"
        for f, t, k in wanted
        if not any(
            edge.get("from") == f and edge.get("to") == t and edge.get("kind") == k
            for edge in body.get("edges", [])
        )
    ]
    if missing:
        drawn = sorted(
            f"{edge.get('from')} -> {edge.get('to')} [{edge.get('kind')}]"
            for edge in body.get("edges", [])
        )
        raise G9Failure(f"the lineage of silver.{silver_table} is missing {missing}; it draws {drawn}")
    node = next((n for n in body.get("nodes", []) if n.get("id") == f"upload:{upload_id}"), None)
    if node is None:
        raise G9Failure(f"the lineage has no node for upload:{upload_id}: {body.get('nodes')}")
    print(f"[g9] lineage: upload:{upload_id} -> bronze.{table} -> silver.{silver_table} ({node.get('label')!r})")


def step_feeds_a_pipeline(
    api, requests, table: str, first_upload_id: str, using: dict, expected: dict, state: dict
) -> None:
    """DATA-19, the whole feature in five assertions, in the order they
    can each fail informatively:

    1. a Bronze table an upload made is a pipeline's source;
    2. the pipeline runs once and its Silver table holds the rows;
    3. re-uploading the source table runs it AGAIN, with no trigger call,
       and the changed row reaches Silver;
    4. loading a DIFFERENT table does NOT run it;
    5. lineage draws `upload -> bronze.<table> -> silver.<out>`.

    `first_upload_id` is the upload the first load was made from, so the
    lineage step names the upload the table really came from -- the claim
    records the FIRST upload to ask for a table name, and the corrected
    copy of step 3 keeps that claim (a claim is never taken away)."""
    silver_table = f"{table}_clean"
    pipeline_id = step_create_pipeline_over_uploaded_table(api, requests, table, silver_table)
    # The table holds the `replace` load AND the `append` of it that step
    # 4 made, so the pipeline copies twice the file's rows. The re-upload
    # below replaces them again, which is the point: a re-upload that
    # changes nothing would not show anything downstream either.
    step_first_pipeline_run(api, pipeline_id, silver_table, 2 * len(expected["rows"]))
    step_reupload_reruns_the_pipeline(api, table, silver_table, pipeline_id, using, expected, state)
    step_other_table_does_not_rerun(api, table, pipeline_id, state)
    step_lineage_shows_the_upload(api, first_upload_id, table, silver_table)
    print("[g9] pipeline: PASS (uploaded table -> pipeline, re-upload re-ran it, another upload did not)")


def main() -> int:
    argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    ).parse_args()
    # Imported after `--help` so that asking for help needs only the standard library.
    import requests

    api = requests.Session()
    table = f"g9_upload_{secrets.token_hex(4)}"
    state: dict = {"upload_id": None, "deleted": False}
    try:
        _require_settings()
        step_wait_for_services(api, requests)
        step_login(api)
        step_ensure_tenant(api)
        upload_id, using, expected = step_upload_and_preview(api, state)
        file_rows = len(expected["rows"])
        print(f"[g9] table for this run: {table}")
        step_load(api, upload_id, table, "replace", using, file_rows, table_rows=file_rows)
        step_load(api, upload_id, table, "append", using, file_rows, table_rows=2 * file_rows)
        step_rows_and_types(api, table, expected, state["column_types"])
        step_one_ingest_run_per_load(api, upload_id, table, file_rows)
        step_delete_keeps_the_table(api, upload_id, table, 2 * file_rows, state)
        step_column_cap(api)
        step_workbook(api, table, state)
        step_parquet(api, table, state)
        step_typed_columns(api, table, state)
        step_new_formats(api, table, state)
        step_feeds_a_pipeline(api, requests, table, upload_id, using, expected, state)
    except (G9Failure, requests.RequestException) as exc:
        reason = str(exc) if isinstance(exc, G9Failure) else f"{type(exc).__name__}: {exc}"
        print(f"[g9] FAILED: {reason}", file=sys.stderr)
        _clean_up_upload(api, state)
        return 1
    print("[g9] PASS")
    return 0


def _clean_up_upload(api, state: dict) -> None:
    """Best effort after a failed run: take the upload (its object and row) back
    out, so a failed gate does not leave a file in the bucket and a row in the
    list. The table and its claim stay, as after a pass. An upload that is
    still loading answers 409 and stays."""
    if not state["upload_id"] or state["deleted"]:
        return
    try:
        resp = api.delete(f"{API_URL}/api/uploads/{state['upload_id']}", timeout=30)
        print(f"[g9] cleanup: DELETE upload {state['upload_id']} answered {resp.status_code}", file=sys.stderr)
    except Exception as exc:  # noqa: BLE001 -- the failure that matters is the one already reported
        print(f"[g9] cleanup: upload {state['upload_id']} was not deleted ({type(exc).__name__})", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
