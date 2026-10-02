# Upload a file into a raw table — Implementation Plan

**Status:** approved by the product owner on 2026-10-02, not started.
Written by the planner (Claude Opus) for a developer agent, under the role
split in `AGENTS.md` on `main` ("Who plans, who writes, who reviews"). The
planner writes no product code for this plan.

**Feature page (requirements and acceptance checklist):**
`docs/core/features/upload-file.md`, backlog `DATA-9`. **Decision record:**
`docs/adr/0014-file-upload.md`.

**Base commit:** `98aaa64` on `feat/connectors` (local only). **Branch:**
`feat/upload-file`, in the worktree `/home/hv/lakehouse-upload`.

**Goal, in the user's words:** add a file-upload connector, so a user can
bring data in not only through the existing connector options and wizard
but also by uploading a file.

**Why this exists.** `PRODUCT.md` lists "Upload a file from the console" as
Missing and next. Commit `f9793cd` left an unwired sketch of it on the
branch. This plan finishes that sketch, corrects what is wrong in it, and
gives it a screen.

**Why the base is `feat/connectors` and not `main`.** The uploaded table is
only useful where a raw table can be opened and queried from the console,
and where connector routes are scoped to a tenant. Both are on
`feat/connectors` and not on `main`. The consequence: no pull request can be
opened from this branch until `feat/connectors` is on `main`.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Accepted files | Delimited text only: UTF-8 or UTF-16; delimiter one of `,` `;` tab `\|`. Anything else is refused by name |
| Limits | 50 MB and 2,000,000 data rows per file. Over either: refused with the reason. Never cut short |
| Where it sits | "Upload file" button and "Uploaded files" tab on Sources; a link from the first step of "New Connector"; the flow is its own page, `/connectors/upload` |
| Who | `connector:manage` on every upload route. No new permission, no role grant |
| Existing table | A load may target a table that does not exist, or one an upload of the same tenant created. Default `replace`; `append` is a choice. Never a connector's table, never anything else |
| Types | Every column is text |
| Original file | Kept in the warehouse bucket until the upload is deleted. Deleting an upload keeps its table |
| Assistant | No tool in this version |
| Transform code from `f9793cd` | `silver_transform.py`, `gold_transform.py`, `sap_models.py` leave the branch (they stay in history at `f9793cd`). `ch_models.py` stays |
| Tenant | An upload belongs to the uploader's active tenant. Per-upload routes answer 404 outside it |
| Outcome of a load | Recorded by the job in `lake.bronze_meta.ingest_run`, read back by the API. The job does not write to Postgres |

## 2. What exists today (anchors, verified at `98aaa64`)

Line numbers drift; re-find by symbol if they do not match.

### 2.1 The unwired code from `f9793cd`

- `rust/crates/lakehouse-api/src/routes/uploads.rs` (662 lines): `create`,
  `list`, `get`, `preview`, `ingest`, `delete`, and the detection helpers
  with tests. **Not declared in `routes/mod.rs`**, so not compiled.
- `rust/crates/lakehouse-api/src/upload_store.rs` (158): `UploadStore`
  over the warehouse bucket, prefix `uploads`. **Not declared in `main.rs`.**
- `rust/crates/lakehouse-store/src/uploads.rs` (264): the `file_upload`
  repository. **Not declared in `lib.rs`** (`lib.rs:35-52`).
- `rust/migrations/0054_upload.sql`: table `file_upload`. **Applied on the
  development database** (0 rows). Do not edit it (rule: never edited once
  applied; `sqlx` would refuse to boot on the checksum).
- `dagster/dispar_orchestrate/file_ingest.py` (275): `file_ingest_job`.
  **Not registered in `definitions.py`** (`definitions.py:81-91`).
- No console code. No `POLICY_TABLE` entry.

What is wrong in it, and must not survive:

1. It does not compile as written: `state.upload_store()` does not exist;
   `launch_run_with_config` takes `&Value`, not `Option`
   (`lakehouse-dagster/src/lib.rs:1145`); `axum::extract::Multipart` needs
   axum's `multipart` feature (`rust/Cargo.toml:16` enables only `macros`);
   axum's default 2 MB body limit applies.
2. `get`, `preview`, `ingest`, `delete` and the duplicate check take any id:
   no tenant filter. The tenant recorded is the deployment's `TENANT_ID`
   constant, not the caller's.
3. Every storage and orchestrator error is returned as `err.to_string()`
   (principle 4), and several messages are in Indonesian.
4. No audit event.
5. `split_row` splits on the delimiter without honouring quotes; the job
   uses Python's `csv`. A quoted field with a comma previews as two columns
   and loads as one.
6. `file_ingest.py` imports `_install_catalog_env` and `_stamp_ingested_at`
   from `dlt_pipeline`; both moved to `adapters/sink.py` (`:128`, `:237`).
   `ops/lint/check_intra_package_imports.py` fails on it.
7. `file_ingest.py::_update_upload` writes the outcome into Postgres with
   `cfg.source_db_*`, the demo ingest's *source* database. It reaches the
   console's database only on the compose stack, where the two coincide.
8. It always appends, and nothing checks whether the target table belongs
   to a connector.
9. It registers `row_count=len(rows)`, this file's rows, where a
   connector's table registers its total.
10. `parse_rows` stops silently at `MAX_ROWS`. A cut-short load that reports
    success is a fabricated success (principle 2).

Worth keeping from it: the storage-key rule (`storage_key`, `extension_of`
and their tests), encoding / delimiter / header-row detection and their
tests, `_column_names`, the padding of short rows, bytes-before-row on
create and object-before-row on delete.

### 2.2 What to reuse (rule 4: grep before writing)

- **Tenant scope:** `tenant_scope::resolve` (`tenant_scope.rs:51`). List and
  create follow `routes::connectors::list` (`connectors.rs:99`) and
  `resolve_create_tenant` (`:771`).
- **Per-id tenant guard:** `ensure_connector_in_tenants` (`connectors.rs:137`),
  `require_connector_in_tenants` (`:159`), mounted as a `route_layer` in
  `connectors_router` (`routes/mod.rs:445-536`), merged at `mod.rs:766`.
  Store side: `connectors::connector_in_tenants` (`lakehouse-store/src/
  connectors.rs:341`).
- **Policy table:** connector entries at `policy.rs:461-539`.
  `tests/route_auth.rs:838` and `:875` walk the table and the router.
- **Audit:** `connector_audit_event` (`connectors.rs:593`),
  `lakehouse_store::audit::insert` (`audit.rs:198`). Best effort: a failed
  audit write never fails the action.
- **Launching a job with config, and "one run at a time":**
  `routes::connectors::ingest_run` (`connectors.rs:2764-2825`).
- **Run status by id:** `DgClient::pipeline_run_status`
  (`lakehouse-dagster/src/lib.rs:1671`), `map_run_status` (`:2327`).
- **Reading `bronze_meta.ingest_run`:** `ingest_runs_for_connector` and
  `ingest_runs_or_empty` (`routes/governance.rs:681`, `:728`). Private
  today; make them `pub(crate)` rather than copying the query.
- **Building an S3 client from `RUSTFS_*`:** `health::probe_rustfs` and its
  `ExactMatchSecretResolver` (`health.rs:370-470`). The connector secret
  resolver refuses these refs; do not use it. Share one builder between the
  health probe and the upload store.
- **Classifying an object-store error:**
  `connector_probe::classify_object_store_error` (`connector_probe.rs:979`).
- **Does a raw table exist:** `catalog_source::iceberg_source`
  (`routes/catalog_source.rs:126`) and the registry lookup in
  `routes::catalog::bronze_upstream` (`catalog.rs:2025`).
- **Request deadline per route:** `route_timeout` (`routes/mod.rs:120`) and
  its test (`:845`).
- **The shared raw-table writer:** `adapters/sink.py`: `LoadPlan` (`:98`),
  `SinkConfig` (`:195`), `load_via_sink` (`:369`). Plain rows with `replace`
  or `append` are supported; the Kafka micro-batch is the example
  (`ingest_factory.py:523-640`).
- **Recording an outcome:** `bronze_catalog.record_ingest_run`
  (`bronze_catalog.py:625`).
- **Registering a loaded table:** `connector_catalog.register_connector_table`
  (`connector_catalog.py:44`), test `test_connector_catalog.py`.
- **Console:** `apiFetch` (`src/services/http.ts:98`; it is `fetch` plus
  `X-Tenant`, so a `FormData` body works); client style in
  `src/services/clients/connectors.ts`; binding in `src/services/index.ts`;
  the wizard shell `FormStepLayout`
  (`src/components/patterns/form-step-layout.tsx`); the Sources page
  (`src/features/connectors/connectors-page.tsx:164-266`); the type picker
  (`connector-type-picker.tsx`); the table-name rule `targetProblem`
  (`connector-ingest-panel.tsx:36-65`); `src/hooks/use-refreshable.ts`.

### 2.3 Checks that are red on the base, before any change

Measured on `98aaa64` (`docs/plans/FEAT-CONNECTORS-REPORT.md`, section 4):

- `cargo fmt --check`: one file, `rust/crates/lakehouse-api/tests/
  connector_delete_deprovision.rs`.
- `ops/lint/check_intra_package_imports.py`: `file_ingest.py:48`.
- `ops/lint/check_bare_iceberg_count.py`: `connector_catalog.py` (the lint
  reports line 18; the query is at `:68`), `test_connector_catalog.py:43`,
  `silver_transform.py:162`, `routes/catalog_governance.rs:959`.

T1, T2 and T7 make all three green. Nothing else was red.

## 3. Tasks

One task per commit, in order. Per commit run only the scoped checks for
what it touched (`AGENTS.md`, "Keep build time down"). The full
verification block runs once per slice, before its handoff.

Messages a user can read are in English and are fixed text. An underlying
error goes to `tracing`, never into a response (principle 4). Add no
`ApiError::Internal(err.to_string())`.

### T1 — Make the base's formatting and R11 lint green

- Format `tests/connector_delete_deprovision.rs` (`cargo fmt`; no other
  file should change).
- `connector_catalog.py`: count with a `WHERE` (`WHERE 1`, as
  `ops/g3a/g3a_test.py` does) and reword whatever the lint matches at line
  18. Update `test_connector_catalog.py` to the new statement.
- `routes/catalog_governance.rs:959`: the test's sample query gets a
  `WHERE`.
- Do not touch the lint or add an allowlist entry (rule 2).
- **Accept:** `cargo fmt --check` passes; `check_bare_iceberg_count.py`
  reports only `silver_transform.py` (removed in T2); the two touched test
  suites pass.

### T2 — Remove the unregistered transformation modules

- Delete `dagster/dispar_orchestrate/silver_transform.py`,
  `gold_transform.py`, `sap_models.py`. Check nothing imports them
  (`connector_catalog.py` mentions `silver_transform.py` in a comment;
  reword it). Keep `ch_models.py`.
- **Accept:** `check_bare_iceberg_count.py` passes;
  `python -m pytest dispar_orchestrate -q` passes with the same count as
  before this task.

### T3 — Store and schema

- Migration `0055_upload_tenant_mode.sql`, with a why-header: add
  `tenant_id UUID REFERENCES tenant(id) ON DELETE SET NULL` (the same shape
  `0042` gave `connector`), `load_mode TEXT CHECK (load_mode IN ('replace',
  'append'))`, `row_count BIGINT`; drop the free-text `tenant` column and
  its index; index `(tenant_id, created_at DESC)` and `(bronze_table)`.
  The numbers `0054` and `0055` are provisional: `DATA-1` plans to use
  `0054` on `main`. Say so in the header; do not renumber now.
- `lakehouse-store`: declare `pub mod uploads;`. Functions, values bound,
  `# Errors` on each:
  `insert` (takes `tenant_id`), `list(tenant_id, limit)`, `get`,
  `upload_in_tenants(id, tenant_ids)`, `find_by_sha256(tenant_id, sha, 
  exclude_id)`, `mark_ingesting(id, parse_options, bronze_table, load_mode,
  run_id)`, `mark_finished(id, error, row_count)`, `delete`,
  `table_created_by_upload(tenant_id, table)` (an `ingested` row of that
  tenant names the table), `table_being_loaded(table, exclude_id)` (another
  row is `ingesting` into it).
- `connectors.rs` (store): `any_connector_targets(table)`, true when a
  connector's `source_objects` holds that `target`.
- The serialized `Upload` drops `storageKey` and `tenant` and gains
  `loadMode`, `rows`.
- Tests in `rust/crates/lakehouse-store/tests/uploads.rs` with
  `sqlx::test`.
- **Accept:** tests cover: list and duplicate lookup see only their own
  tenant; `upload_in_tenants` is false for another tenant and for an
  unknown id; the state transitions; `mark_finished` with and without an
  error; `table_created_by_upload` true only for an ingested row of that
  tenant; `any_connector_targets` true and false.

### T4 — Storage for the files

- One builder for an S3 client over the warehouse bucket from `RUSTFS_*`,
  through the exact-match resolver, used by both `health::probe_rustfs` and
  the upload store. No behaviour change for the health probe.
- Declare `mod upload_store;`. `UploadStore::connect(&Config)`; `put`,
  `head_bytes`, `delete` as they are. A missing object on delete is success.
- Errors become two fixed messages: "Upload storage is not configured." and
  "Upload storage is unavailable (<classify_object_store_error>)." (both
  503). The detail is logged.
- **Accept:** unit tests for the error mapping and for "unset refs means
  not configured, and nothing is dialled"; the health probe's tests still
  pass unchanged.

### T5 — Sniffing and parsing, as pure functions

New module `upload_parse.rs` holding what `uploads.rs` has today plus:

- `sniff(head) -> Kind`: `DelimitedText`, `Workbook` (zip `PK\x03\x04` or
  OLE `D0 CF 11 E0`), `Parquet` (`PAR1`), `OtherBinary` (PDF, gzip, or NUL
  bytes in text that is not UTF-16).
- `split_records(text, delimiter)`: quote-aware. `"` quotes a field, `""`
  is a literal quote, the delimiter and line breaks are allowed inside
  quotes. Replaces `split_row` everywhere. No new crate.
- A preview built from a truncated head drops its last, possibly partial,
  record.
- Fixture files under `ops/fixtures/uploads/`, each with the expected
  columns and rows beside it as JSON: quoted comma and line break (UTF-8,
  comma); semicolon; pipe; UTF-16 LE with BOM, tab, report lines above the
  header; duplicate and blank header cells; a short row and a long row.
  T7's Python tests read the same files.
- **Accept:** every fixture parses to its expected JSON; the existing
  detection tests still pass; each `Kind` is recognised from its magic
  bytes.

### T6 — The routes

`routes/uploads.rs`, declared in `routes/mod.rs`, mounted by a new
`uploads_router(&state)` beside `connectors_router`. Every per-id route
sits behind a `require_upload_in_tenants` layer shaped like the connector
one.

| Route | Behaviour |
| --- | --- |
| `POST /api/uploads` | One multipart part named `file`. Tenant from `tenant_scope::resolve`; a caller in no tenant gets 400. Refuse: no part, empty, over 50 MB ("The file is larger than the 50 MB limit."), `Workbook` ("This looks like an Excel workbook. Save the sheet as CSV and upload that file."), `Parquet`, `OtherBinary`. Store bytes, then the row. 201 with the upload, plus `duplicateOf` when the same tenant already holds the same bytes |
| `GET /api/uploads` | The active tenant's uploads, newest first, at most 100. No tenant: empty list |
| `GET /api/uploads/{id}` | One upload |
| `GET /api/uploads/{id}/preview` | `?encoding=&delimiter=&headerRow=` override detection. Returns `detected`, `using`, `columns`, at most 20 `rows`, `truncated` |
| `POST /api/uploads/{id}/ingest` | Body `{ bronzeTable, mode?, encoding, delimiter, headerRow }`; `mode` defaults to `replace`. Validates all five. 409 when this upload is already loading, when another upload is loading into that table, or when the table is not free (below). Launches `file_ingest_job`, then marks the row `ingesting`. Returns `{ upload, runId }` |
| `DELETE /api/uploads/{id}` | 409 while loading. Deletes the object, then the row. Never the table |

- **Is the table free?** Refuse with 409 when a connector targets it
  (`any_connector_targets`), or when it exists (registry row or
  `iceberg_source`) and `table_created_by_upload` is false. If the check
  itself fails, answer 503; never assume the name is free.
- **Reconciling a load.** A row in `ingesting` is settled when it is read
  (`list`, `get`): ask `pipeline_run_status(run_id)`. While the run is
  queued or running, leave it. When it has ended, take the newest
  `bronze_meta.ingest_run` row for `upload:<id>` that ended after the row's
  `updated_at`: `succeeded` gives `ingested` with its `rows`; anything else
  gives `failed` with its `error`. No such row: `failed`, "The load stopped
  before it recorded a result." If the orchestrator cannot be reached,
  return the row unchanged.
- `Upload` in a response: `id`, `originalFilename`, `sizeBytes`, `status`,
  `uploadedBy`, `createdAt`, `updatedAt`, and when set `parseOptions`,
  `bronzeTable`, `loadMode`, `rows`, `runId`, `error`, `assetId` (the
  catalog slug, the table name with `_` as `-`, only when `ingested`).
- `POLICY_TABLE`: six entries, all `connector:manage`.
- Body limit: `DefaultBodyLimit::max` of the cap plus 1 MiB, on the POST
  route only. `route_timeout`: 300 s for `/api/uploads`, with the reason at
  the constant (a bound for a slow link, not a measurement); extend the
  existing test.
- Enable axum's `multipart` feature. List every crate the lock file gains
  in the handoff.
- Audit: `upload.create`, `upload.ingest`, `upload.delete`, resource kind
  `upload`. Args carry the file name, size, table and mode. Never a row of
  the file.
- **Accept:** `tests/route_auth.rs` passes; a Data Engineer is not refused
  and an Analyst is, on `POST /api/uploads`. Router-level tests (pattern:
  `tests/connector_probe_history_route.rs`) for: another tenant gets 404 on
  each of the four per-id routes and does not see the upload in the list;
  `duplicateOf` is not reported across tenants; a 3 MB body is accepted and
  a body over the cap refused with the fixed text; a workbook's magic bytes
  are refused; an invalid table name, mode, encoding and delimiter are each
  400; a table a connector targets is 409; a second ingest while loading is
  409; delete while loading is 409; no response body contains the text of
  an injected storage or orchestrator error.

### T7 — The load job

Rewrite `dagster/dispar_orchestrate/file_ingest.py`.

- Run config: `ops.ingest_uploaded_file.config` with `upload_id`,
  `storage_key`, `bronze_table_name`, `load_mode`, `encoding`, `delimiter`,
  `header_row`.
- Read the object with the sink's credentials. Refuse a key that does not
  start with `uploads/` or contains `..`.
- Parse with `csv.reader` in the dialect of ADR 0014, decision 3. Keep
  `_column_names` and the padding of short rows.
- Fail, writing nothing, with one of these exact reasons: "The stored file
  could not be read." / "The header row is past the end of the file." /
  "The file has no rows below the header row." / "The file has more than
  2,000,000 rows."
- Write through `load_via_sink(rows, table, SinkConfig…, LoadPlan(mode))`.
  No Iceberg code of its own. A failure there: "The load into the table
  failed." The run log has the detail.
- Register the table through one helper shared with
  `connector_catalog.register_connector_table` (total under a `WHERE`,
  columns from `DESCRIBE`, author `upload`). A failure there: "The table
  was loaded but could not be registered in the catalog."
- Record exactly one `record_ingest_run` row per run:
  `connector_id=f"upload:{upload_id}"`, `job="file_ingest_job"`,
  `object_name=<table>`, `rows` from the sink (never `len(rows)` when the
  sink reports `None`), `status`, `error` = the reason above or empty.
  Re-raise after recording a failure so the run is red.
- No `psycopg2`. Module docstring, `from __future__ import annotations`.
- Register `file_ingest_job` in `definitions.py`.
- From the review of slice A (section 9):
  - `SHOULD-FIX A1`: `ch_models.py` holds a SQL-model runner (`Model`,
    `run_model`, `run_models`, `_split_target`, `ALLOWED_SCHEMAS`) that has
    had no caller since T2. Put the three helpers still in use
    (`ch_target`, `ensure_catalog_database`, `ch_exec`) beside the shared
    registration helper and delete the rest of the file.
  - `SHOULD-FIX A2`: `connector_catalog.py`'s docstring still says raw
    tables are append-only. Since load modes (`0941ce4`) a run replaces by
    default. Say what is true when the helper is reshaped.
- Tests, no network, fakes injected as in `test_connector_catalog.py`:
  every fixture of T5 parses to the same JSON; each failure reason; the row
  cap fails and writes nothing; the mode reaches `LoadPlan`; one outcome
  row on success and on failure; the key guard.
- **Accept:** `check_intra_package_imports.py` passes; the new tests pass;
  the job appears in `definitions.py`.

### T8 — A connector may not take an uploaded table

- `routes::connectors::ingest_spec_put` (`connectors.rs:2654`): 409 when a
  `sourceObjects[].target` is a table an upload created.
- **Accept:** a route test for the refusal and one that an ordinary target
  still saves.

### T9 — Acceptance gate

- `ops/g9/upload_test.py`, in the shape of `ops/g6/g6_ingest_matrix_test.py`:
  sign in, upload a fixture, check the preview, load, poll to `ingested`,
  count the rows through `POST /api/query/run` with a `WHERE`, load again
  with `append` and count again, delete the upload, check the table is
  still there. Exit non-zero on any mismatch.
- **Accept:** `python3 -m py_compile` and a `--help` run. Running it needs a
  deployed stack, which the developer does not have; say so in the handoff.

### T10 — Console: contract and client

- `src/services/contracts/uploads.ts`: `Upload`, `UploadStatus`,
  `UploadPreview`, `UploadParseOptions`, `IngestUploadInput`,
  `interface UploadService` (`list`, `get`, `create`, `preview`, `ingest`,
  `remove`). camelCase mirrors the routes.
- `src/services/clients/uploads.ts` through `apiFetch`; `create` sends
  `FormData` and sets no `Content-Type`. Errors as `ServiceError`, as the
  connector client does. Bind `uploadService` in `src/services/index.ts`.
- `src/lib/uploads.ts`, pure: `MAX_UPLOAD_BYTES`, `suggestTableName(file
  name)`, `tableNameProblem` (the same rule as `targetProblem`; do not
  write a second regex if the first can be imported), `statusLabel`,
  `delimiterLabel`. Tests beside it with `node:test`
  (`CODE-STANDARD.md` §4.6).
- **Accept:** `bun run typecheck`, `bun run lint`, the new tests.

### T11 — Console: the screens

- **Sources page.** Header actions: "Upload file" (outline, to
  `/connectors/upload`) before "New Connector". Two tabs under the header,
  kept in the URL (`?tab=uploads`): "Connectors" (what is there today,
  unchanged) and "Uploaded files".
- **Uploaded files.** A table: File, Size, Status, Table (a link to
  `/data/assets/<assetId>` when loaded), Rows ("Not measured" when absent),
  Uploaded by, Uploaded. Row actions: "Load" for `uploaded` and `failed`
  (opens the upload page on that upload), "Delete" behind a confirmation
  that says the table stays, disabled while loading. Refreshes every 5 s
  while a row is loading. Empty state with the upload button.
- **Upload page** `/connectors/upload`, `FormStepLayout`:
  1. **File.** A drop area and a file button. Shows name and size. A file
     over 50 MB is refused here, naming the limit, and nothing is sent.
     "Next" sends the file; a refusal from the server is shown in place.
     A repeated file shows the notice with the earlier upload's name and
     date. With `?id=`, this step is skipped.
  2. **Check.** Encoding (UTF-8, UTF-16), Delimiter (Comma, Semicolon, Tab,
     Pipe), Header row (shown from 1). Each says what was detected. Below,
     the columns and the first 20 rows; a line saying so when the file is
     longer. Changing a control reloads the preview.
  3. **Table.** The table name, suggested from the file name, with the
     naming rule. When the name is a table an earlier upload of this tenant
     created: "Replace its rows" (selected) or "Add to its rows". A line
     saying every column is stored as text.
  4. **Review.** A summary; the submit button is "Load".
  After "Load": the status, refreshed every 2 s while loading. Loaded: the
  row count, "Open in Data Explorer", "Upload another file". Failed: the
  reason, "Change settings" (back to step 2), "Try again".
- **New Connector, first step.** Above the type cards, one link: "Have a
  file instead? Upload a CSV or TSV" to `/connectors/upload`.
- Components in `src/features/connectors/`, `"use client"` first, imports
  from `@/services` only. Component tests with `bun:test`, as the
  connector tests are.
- **Accept:** tests for: the oversize refusal sends nothing; a server
  refusal is shown; the preview shows detected and chosen values and
  reloads on change; the mode choice appears only for an uploaded table;
  each of Loading, Loaded, Failed; the list's "Not measured"; delete asks
  first. `bun run typecheck && bun run lint && bun run test` green.

### T12 — Documents that change with the code

- `CHANGELOG.md` `[Unreleased]`: the feature, and the removal of the three
  modules.
- `docs/OPERATIONS.md`: files are kept under `uploads/` in the warehouse
  bucket and grow it; the load needs the orchestrator profile and the
  `RUSTFS_*` refs; a reverse proxy in front of the console must allow a
  50 MB request body.
- `docs/ARCHITECTURE.md`: one paragraph on the upload path.
- `docs/FEATURE_COVERAGE.md`: a row.
- `README.md`: nothing new to configure; add one line under "Status /
  Known limitations" with the limits of the feature page.
- **Accept:** each file names only settings and routes that exist.

## 4. Slices

| Slice | Tasks | Why separate |
| --- | --- | --- |
| A | T1–T2 | Hygiene of the base. Could merge on its own |
| B | T3–T6, T8 | The API. Rust is confined to this slice and T1 |
| C | T7, T9 | The job and the gate. No Rust |
| D | T10–T12 | Console and documents. No Rust, no Python |

The developer appends a handoff per slice. The planner reviews each before
the next starts.

## 5. Out of scope (do not build)

- Workbooks, JSON, Parquet, compressed files.
- Direct-to-storage upload, resumable upload, files over 50 MB.
- Type inference; any Silver or Gold step.
- An assistant tool.
- Deleting or renaming a raw table.
- Making pipelines read raw tables (`authored_factory.py` belongs to
  another team).
- A tenant filter on `GET /api/governance/ingest-runs`. It has none today;
  that is a finding for its own change, not this plan.
- Renumbering migrations.

## 6. Things the developer must verify, not assume

- That `load_via_sink` with plain rows and `LoadPlan("replace")` needs no
  change in `adapters/sink.py`. If it does, stop and report.
- That enabling `multipart` adds only the crates it should; list them.
- That the `0055` migration applies on a database that has `0054` with no
  rows, and on a fresh one (`sqlx::test` does the second).
- That no test depended on the three modules T2 removes.
- Not verifiable by the developer, and to be written as *not verified*: a
  real load end to end; a 50 MB upload through the console's `/api` proxy;
  the gate. They need a deployed stack (section 9).

## 7. Working on this machine

- Work only in `/home/hv/lakehouse-upload`. Never edit, check out or build
  in `/home/hv/lakehouse` (the product owner's checkout) or
  `/home/hv/lakehouse-uiux`. Never push. Never run `docker compose` against
  the running stack, and never rebuild or restart its containers.
- Rust: `export CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
  CARGO_BUILD_JOBS=4`. Check `df -h /` before a build; stop and report
  under 15 GB free. No `cargo clean`.
- Console: `bun install --frozen-lockfile` once in the worktree. No
  prettier; match the file's style by hand.
- Orchestrator tests: a throwaway container of the code-location image,
  the worktree's `dagster/` mounted read-only:
  `docker run --rm --entrypoint sh -e PYTHONDONTWRITEBYTECODE=1 -v
  /home/hv/lakehouse-upload/dagster:/work/dagster:ro -w /work/dagster
  lakehouse-dagster-code-location:latest -c "pip install -q --target
  /tmp/pt pytest && PYTHONPATH=/tmp/pt python -m pytest dispar_orchestrate
  -q -p no:cacheprovider"`. A test that needs a writable tree copies what
  it needs to `/tmp` inside the container.
- Commits: one per task, Conventional Commits, English, the task id in the
  subject (`feat(uploads): T3 store and schema`). End each message with the
  developer model's own `Co-Authored-By` line, so the history shows who
  wrote the code. `git status` before every commit; nothing generated.

## 8. Handoff (developer appends one entry per slice)

Commits; the exact commands run with their counts; anything skipped or
*not verified* with the reason; every place the plan was wrong against the
code.

### Slice A — T1, T2 (developer, 2026-10-02)

Branch `feat/upload-file`, from `98aaa64` plus the plan commit `41ce2ad`.
Nothing was pushed. No file outside `/home/hv/lakehouse-upload` was edited
(build output went to the shared `CARGO_TARGET_DIR`, logs and scratch copies
to the session scratchpad); no `docker compose` was run, only throwaway
`docker run --rm` containers of the existing code-location image. No
`Cargo.lock` change.

**Commits**

- `87646eb` fix(lint): T1 format one test file and count raw tables under a WHERE
- `46642fa` chore(orchestrator): T2 remove the unregistered transformation modules

T1 (4 files, +21/-5): `cargo fmt` on `tests/connector_delete_deprovision.rs`
(the only file `cargo fmt --check` refused, one hunk; `git diff --stat`
after `cargo fmt` showed that file alone); `WHERE 1` on the Bronze total in
`connector_catalog.py` plus a comment at the statement saying why (R11,
`docs/plans/P5-RESULT.md`), and the docstring's "row count" before
`dataset_sync.total` reworded to "row total"; `test_connector_catalog.py`
asserts the new statement; `WHERE 1` on the sample query in
`routes/catalog_governance.rs` (the test only checks which table a query
reads). The lint, its allowlist and every threshold are untouched.

T2 (5 files, +6/-446): `git rm` of `silver_transform.py`,
`gold_transform.py`, `sap_models.py` (441 lines); the three comments that
named `silver_transform.py` (one in `connector_catalog.py`, two in
`ch_models.py`) now state the reason themselves. `ch_models.py` stays.

**Commands run, with counts**

Per commit, scoped (T1):

- `cd rust && cargo fmt --check` on the base: failed, one hunk, in
  `connector_delete_deprovision.rs:119`. After `cargo fmt` and the
  `catalog_governance.rs` edit: exit 0.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings`: exit 0.
- `cargo test -p lakehouse-api --bin lakehouse-api catalog_governance`:
  18 passed, 0 failed, 1070 filtered out (includes
  `reads_any_needs_the_table_in_the_query_not_just_in_its_text`).
- `cargo test -p lakehouse-api --test connector_delete_deprovision`:
  6 passed, 0 failed.
- `pytest dispar_orchestrate/test_connector_catalog.py -v` in the
  code-location image: 5 passed. The same file against the pre-T1
  `connector_catalog.py` (a scratch copy, the worktree untouched): 1 failed,
  4 passed, so the new assertion is not vacuous.
- `python3 ops/lint/check_bare_iceberg_count.py` after T1: reports only
  `silver_transform.py:162`, as the plan's acceptance says.

Per commit, scoped (T2):

- Before deleting (a repo-wide `grep`, skipping `node_modules`, `target`,
  `.next`, `.git`): nothing imports the three modules, and no job or op of
  theirs (`silver_transform_job`, `gold_transform_job`, `sap_transform_job`,
  `transform_bronze_to_silver`, `build_gold_marts`, `build_sap_models`) is
  named outside the three modules, in `dagster/`, `ops/`, `rust/`, `src/`,
  `docker-compose.yml` or `docs/`; `definitions.py` registers none; no test
  file covers them. The only other mentions are the comments listed under
  "Where the plan was wrong", the plan and ADR 0014.
  `transform_test_vectors.json` belongs to `authored_transforms`, not to
  them.
- Orchestrator suite, image command from the plan, before T2 (after T1,
  which adds no test): `402 passed, 30 subtests passed in 11.72s`;
  `--collect-only`: 402 tests. After T2: `402 passed, 30 subtests passed in
  11.79s`; `--collect-only`: 402 tests, and the sorted test ids are
  identical before and after (`diff` empty).
- `check_bare_iceberg_count.py` after T2: exit 0, "no bare count() against a
  Bronze Iceberg table found".
- `import dispar_orchestrate.definitions` in a throwaway container with
  `--network none`: loads; the eight fixed jobs are `agent_run_job`,
  `alerts_run_job`, `bronze_ingest_job`, `bronze_maintenance_job`,
  `capacity_snapshot_job`, `gold_export_job`, `ingest_job`,
  `replication_slot_check_job` (no authored jobs: the run tokens are unset
  in that container).

Full verification, once, in the foreground, on the final product commit
`46642fa` (the handoff commit changes only this file):

- `cd rust && cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0.
- `cargo test --workspace`: exit 0. 76 `test result:` lines (62 test
  binaries and 14 doc-test targets): **3087 passed, 0 failed, 8 ignored**.
  The 8 ignored are `#[ignore]`s already on the base (one each in the
  `lakehouse-api` lib and bin targets, from `routes/ai/registry.rs`; two in
  `tests/parity.rs`; three in `lakehouse-iceberg/tests/g1_lakekeeper.rs`;
  one doc test in `lakehouse-auth`). Their reasons, in the source, are a
  live stack or server, a `DATABASE_URL`, or a fixture rewrite. They were
  not run, and this slice added none (`git diff 98aaa64 HEAD -- rust` has
  no `ignore`).
- `python3 ops/lint/check_intra_package_imports.py`: **exit 1, red, as
  expected**. Output: `file_ingest.py:48` imports `_install_catalog_env` and
  `_stamp_ingested_at` from `dispar_orchestrate.dlt_pipeline`, which does not
  define them. Measured on the untouched `98aaa64` too (a scratch
  `git archive`): the same two lines. T7 rewrites that file; I did not touch
  it.
- `python3 ops/lint/check_bare_iceberg_count.py`: exit 0.
- `python3 ops/lint/check_compose_init_readiness.py`: exit 0, no output.
- Orchestrator pytest (the image command from the brief):
  `402 passed, 30 subtests passed in 12.45s`.

**Not verified, or not run**

- No TypeScript or compose file is touched, so `bun run typecheck|lint|test`
  and `docker compose --profile '*' config --quiet` were not run.
- `gitleaks` (`CODE-STANDARD.md` §9) was not run: it is not installed on this
  machine. I read the diff: it adds no secret, host, port or client name.
- `check_intra_package_imports.py` stays red until T7 (above).
- The cargo runs used the warm shared `CARGO_TARGET_DIR`, not a cold build
  (no `cargo clean`, per the rules), so "from a fresh build" in rule 7 holds
  here only in the sense that cargo recompiled what had changed.

**Where the plan was wrong against the code**

1. T1, `connector_catalog.py`: the plan pairs the lint's line-18 hit with
   "the query is at `:68`". The lint never saw the query: its table name
   arrives through the `table` variable, outside the lint's statement
   window, so it passed over line 68 before and after. The line-18 hit was
   the docstring alone: its prose "row count (`dataset_sync.total`)" put
   `count` and an opening parenthesis, the lint's `count(` pattern, within
   reach of the word `DataLakeCatalog`. Rewording that prose is what clears
   the lint (I checked this by running the lint between the two edits). The
   `WHERE 1` on the real statement is a correctness fix the lint cannot
   enforce, so the comment at the statement says it is kept by hand.
2. T2: the plan names one comment (`connector_catalog.py`). Three files name
   the removed modules in comments: `connector_catalog.py:35` and
   `ch_models.py:12` and `:28` (all reworded), and **`file_ingest.py:33`**
   (`sap_models.py`, in the "Everything lands as text" paragraph of its
   docstring). I left the last one because slice A is not to touch
   `file_ingest.py`; it now points at a file that no longer exists until T7
   replaces the docstring. **T7 must drop that sentence.**
3. T2 and ADR 0014 say `ch_models.py` stays because `connector_catalog.py`
   uses it. That holds for `ch_exec`, `ensure_catalog_database` and
   `ch_target` only. The runner (`Model`, `run_models`, and what it calls:
   `run_model`, `_split_target`, `ALLOWED_SCHEMAS`) had its only callers in
   the removed modules and has none now, and the module docstring still
   introduces the file as the "SQL-model runner" for Silver and Gold. I kept
   the file as the plan says and changed no code in it; whether to prune it
   is the planner's call.
4. The handoff model named in the brief
   (`2026-10-02-gold-publish-per-mart.md`) is not in this worktree, so this
   entry follows the list in section 8's own intro.

## 9. Review (planner appends findings per slice), then the trial

Findings are tagged `BLOCKER` or `SHOULD-FIX`. The planner re-runs the
verification; it does not take the handoff's word.

After slice D has no open `BLOCKER`, and with the product owner's go-ahead:
the planner builds the API and code-location images from this branch,
tells the other sessions on this machine, deploys them to the development
stack, runs `ops/g9/upload_test.py`, and hands the feature page's checklist
to the product owner. Nothing is merged into `feat/connectors` before that
checklist has been run.

### Slice A — T1, T2 (reviewer, 2026-10-02)

Reviewed `87646eb` and `46642fa` against T1 and T2.

**Findings: no `BLOCKER`. Two `SHOULD-FIX`, both moved into T7.**

- `SHOULD-FIX A1`: `dagster/dispar_orchestrate/ch_models.py` now carries a
  SQL-model runner with no caller, and its docstring still introduces the
  file as that runner. T2 was right not to prune it unasked. T7 does.
- `SHOULD-FIX A2`: `connector_catalog.py`'s module docstring says "Bronze is
  append-only". That stopped being true with load modes; T1 left the
  sentence alone, correctly, since it was not in scope. T7 rewrites it.

What was checked against the plan:

- `cargo fmt` changed one file, one hunk, as T1 said it would.
- The real count in `connector_catalog.py` carries `WHERE 1`, and the
  comment says why the lint cannot enforce it there. The docstring edit is
  what cleared the lint; the developer measured that between the two edits.
- The test asserts the new statement and fails against the old module.
- The fixture in `routes/catalog_governance.rs` keeps its meaning (which
  table a query reads).
- No lint, allowlist or threshold was changed (`git show --stat`: four
  files in T1, five in T2).
- Nothing imports the three removed modules; the orchestrator suite has the
  same 402 tests before and after.
- All three commits end with the developer model's `Co-Authored-By` line.

Verification re-run by the reviewer on `05427c2`, warm shared
`CARGO_TARGET_DIR`:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  pass.
- `cargo test -p lakehouse-api --bin lakehouse-api catalog_governance` — 18
  passed. `cargo test -p lakehouse-api --test connector_delete_deprovision`
  — 6 passed.
- `python3 ops/lint/check_bare_iceberg_count.py` — pass.
- `python3 ops/lint/check_compose_init_readiness.py` — pass.
- `python3 ops/lint/check_intra_package_imports.py` — fails on
  `file_ingest.py:48`, as on the base. T7 clears it.
- `python -m pytest dispar_orchestrate -q` — 402 passed, 30 subtests.

Not re-run here: `cargo test --workspace` (the developer's run: 3,087
passed, 0 failed, 8 ignored). The reviewer runs it once on the final commit
before the trial.

**Slice B starts from `feat/upload-file` at this review's commit.**
