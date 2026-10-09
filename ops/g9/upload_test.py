#!/usr/bin/env python3
"""G9 acceptance test (file upload, DATA-9): a file is uploaded, previewed,
loaded into a raw table, loaded again with `append`, deleted, and the table is
checked to be still there -- the end-to-end proof that the upload routes, the
`file_ingest_job` run and the query route work together on a deployed stack,
which their unit and route tests, each with the other side faked, cannot show.
T9 of `docs/superpowers/plans/2026-10-02-upload-file.md`, under ADR 0014.

# What it checks, in order

1. The API answers, and (when `DAGSTER_URL` is set) the code location lists
   `file_ingest_job`. Then it signs in, as `ops/g6/g6_ingest_matrix_test.py`
   does.
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
   intact. And every column is text, as ClickHouse types it (`toTypeName`):
   `file_ingest.py` declares them so because dlt would otherwise type what it
   recognises.
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
- `G9_TENANT_ID`: a tenant the account belongs to, sent as `X-Tenant`.
  Optional: without it the API uses the account's first tenant.

It reads the fixtures from `ops/fixtures/uploads/`, `ops/fixtures/workbooks/` and
`ops/fixtures/parquet/` beside this directory, so a container that runs it needs `ops/` and not only
`ops/g9/`.

# What it leaves behind

The raw tables `g9_upload_<suffix>` (four rows), `g9_upload_<suffix>_xlsx`
(three) and `g9_upload_<suffix>_parquet` (three) with their Iceberg files and
catalog entries; the claims on their names, which are never released; four
`ingest_run` rows; audit events and query history entries. A raw table and
its claim cannot be removed through the API (deleting a raw table is outside
the plan, section 5), so an operator who wants them gone removes them by hand.
The upload itself is deleted at the end, and also when a step fails, as far as
the API lets it (an upload that is still loading cannot be deleted).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import secrets
import sys
import time
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

API_URL = os.environ.get("LAKEHOUSE_API_URL", "http://lakehouse-api:8080")
DAGSTER_URL = os.environ.get("DAGSTER_URL", "")
AUTH_EMAIL = os.environ.get("AUTH_BOOTSTRAP_EMAIL", "")
AUTH_PASSWORD = os.environ.get("AUTH_BOOTSTRAP_PASSWORD", "")
QUERY_DB = os.environ.get("ICEBERG_QUERY_DB", "icecat_api")
TENANT_ID = os.environ.get("G9_TENANT_ID", "")


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
    if TENANT_ID:
        api.headers["X-Tenant"] = TENANT_ID
    print("[g9] logged in")


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
    return upload_id, preview["using"], expected


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


def step_rows_and_types(api, table: str, expected: dict) -> None:
    columns = ["plnt", "material", "description", "qty"]
    body = run_query(api, f"SELECT {', '.join(columns)} FROM {QUERY_DB}.`bronze.{table}` WHERE 1 ORDER BY material, plnt")
    got = [[row[c] for c in columns] for row in body["rows"]]
    want = sorted(expected["rows"], key=lambda r: (r[1], r[0]))
    # The table holds the file twice after the append: compare the distinct rows,
    # in the file's own text (`0000100`, not `100`).
    distinct = [list(r) for r in sorted({tuple(r) for r in got}, key=lambda r: (r[1], r[0]))]
    if distinct != want:
        raise G9Failure(f"the table holds {distinct}, the file says {want}")

    types = run_query(
        api,
        "SELECT "
        + ", ".join(f"toTypeName({c}) AS t_{c}" for c in columns)
        + f" FROM {QUERY_DB}.`bronze.{table}` WHERE 1 LIMIT 1",
    )["rows"][0]
    not_text = {c: types[f"t_{c}"] for c in columns if "String" not in types[f"t_{c}"]}
    if not_text:
        raise G9Failure(f"every column should be text, these are not: {not_text}")
    print(f"[g9] the rows are the file's own text and every column is text ({types['t_plnt']})")


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
        upload_id, using, expected = step_upload_and_preview(api, state)
        file_rows = len(expected["rows"])
        print(f"[g9] table for this run: {table}")
        step_load(api, upload_id, table, "replace", using, file_rows, table_rows=file_rows)
        step_load(api, upload_id, table, "append", using, file_rows, table_rows=2 * file_rows)
        step_rows_and_types(api, table, expected)
        step_one_ingest_run_per_load(api, upload_id, table, file_rows)
        step_delete_keeps_the_table(api, upload_id, table, 2 * file_rows, state)
        step_workbook(api, table, state)
        step_parquet(api, table, state)
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
