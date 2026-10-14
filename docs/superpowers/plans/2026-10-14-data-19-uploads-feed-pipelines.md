# `DATA-19`: an uploaded table feeds a pipeline, re-runs it on re-upload, and shows in lineage

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `caf5784b` (after `BLD-3`, PR #155, and `DATA-10`,
PR #156). Branch: `feat/data-19-uploads-feed-pipelines`. One PR slice.
Spec: `docs/core/specs/data-19.md`. Feature page:
`docs/core/features/uploads-feed-pipelines.md`.

## 1. What is already true, and what is missing

Verified by reading `main` at `caf5784b`:

- **Already works since `BLD-3`:** an upload lands in a Bronze Iceberg table
  (`file_ingest.py`, `bronze_table_name`), and a console pipeline reads a
  Bronze source through its own `DataLakeCatalog` database. The console's
  "Source table" is a free-text field (`pipeline-create-page.tsx`, the
  `Source zone` / `Source table` fields), so a user who types the upload's
  table name already gets a working pipeline. Spec row 1 ("selectable as a
  pipeline source") is met for typing, not for picking.
- **Missing 1, re-upload trigger.** A pipeline can name Dagster jobs in
  `dependsOn` (`authored_pipelines::validate_depends_on`, `dagster_jobs`), and
  its `authored__<id>_after` sensor fires after ANY success of that job
  (`authored_factory.build_authored_dependency_sensor`). So `dependsOn:
  ["file_ingest_job"]` would re-run every such pipeline after every upload
  of any table. The console has no `dependsOn` control at all.
- **Missing 2, lineage.** `routes/lineage.rs` draws connector → Bronze from
  the ingest spec and pipeline source → target, but nothing for an upload.
  The record exists: `upload_table_claim (bronze_table, tenant_id,
  upload_id)` (migration `0058`), which is never released, even after the
  upload is deleted.
- **Correction to the feature page.** Its AC4 ("deleting the upload is
  refused while a pipeline reads it") is wrong: `DELETE /api/uploads/{id}`
  removes the file and the upload row, never the table
  (`routes/uploads.rs::delete` doc). The pipeline keeps working. The planner
  replaces AC4 in this PR (task T6).

## 2. Decisions (planner)

- **D1. Re-upload trigger = `dependsOn: ["file_ingest_job"]`, filtered to
  the pipeline's own source table.** When a pipeline's `dependsOn` names
  `file_ingest_job`, its sensor counts only those `file_ingest_job` SUCCESS
  runs whose run config `ops.ingest_uploaded_file.config.bronze_table_name`
  equals the pipeline's `sourceTable` (and its `sourceZone` is `bronze`). A
  run for another table never triggers it. Read the config from the run
  (`DagsterRun.run_config`); do not add tags unless reading the config is
  impossible, and say which in the handoff.
  Other upstreams (`ingest_job`, authored pipelines) keep today's behaviour
  exactly; `ingest_job`'s coarse "any connector" firing is a separate gap,
  recorded below, not fixed here.
- **D2. The API keeps `file_ingest_job` honest.** On create, update and
  restore, `dependsOn` containing `file_ingest_job` is refused with 400
  `"file_ingest_job can only be an upstream of a pipeline whose source is a
  bronze table"` unless `sourceZone == "bronze"`. One validator, called from
  the three places `validate_target_zone` is called (`BLD-3`).
- **D3. Console: one switch, shown only for a Bronze source.** In the
  pipeline form's source step, a checkbox "Run again after this table is
  re-uploaded", visible only when `sourceZone` is `bronze`, default ON for a
  new pipeline. ON adds `file_ingest_job` to `dependsOn`, OFF removes it;
  every other `dependsOn` entry is kept as it was. The edit page shows the
  stored state. No other `dependsOn` UI.
- **D4. "Selectable" (spec row 1):** the "Source table" field gets the
  zone's table names as suggestions (a `datalist`, typing still allowed),
  read from the catalog list the console already calls through `@/services`.
  If no existing client returns table names per zone, keep the free-text
  field and say so in the handoff; do not add an API route for this.
- **D5. Lineage edge upload → Bronze table**, kind `"upload"`, evidence
  `upload_table_claim`, from every claim row in the caller's tenant (same
  tenant rule as connectors in `lineage.rs`). The node for the upload is
  `upload:<upload_id>` with the upload's file name as its label when the
  `file_upload` row still exists, and `"deleted upload"` when it does not.
  Never a guessed name.
- **D6. No assistant change.** `DATA-19` is not a BI feature. The
  assistant's `create_pipeline` already accepts a Bronze source; it has no
  `dependsOn` field today and does not get one here (AI team's call,
  `AI-16`).

## 3. Anchors (`main` at `caf5784b`)

- `dagster/dispar_orchestrate/authored_factory.py`:
  `build_authored_dependency_sensor` (the per-upstream `get_runs` walk),
  `_upstream_job_selector`.
- `dagster/dispar_orchestrate/file_ingest.py`: `FileIngestParams`,
  `bronze_table_name`, the op name `ingest_uploaded_file`.
- `rust/crates/lakehouse-api/src/routes/authored_pipelines.rs`:
  `validate_depends_on`, `validate_target_zone`, `update`, `restore_version`;
  `routes/pipelines.rs::create`.
- `rust/crates/lakehouse-api/src/routes/lineage.rs`: `Graph::edge`, the
  connector edges (`source_objects`), the module's edge table.
- `rust/migrations/0058_upload_tenant_mode.sql`: `upload_table_claim`.
- `src/features/pipelines/pipeline-create-page.tsx`: source step fields.
- `ops/g9/upload_test.py` and the `g9-upload` CI job.

## 4. Tasks (one commit each, in order; Rust tasks together)

- **T1. D2 in the API** with tests for create, update and restore (refused
  for a `silver` source, accepted for `bronze`). Scoped Rust checks.
- **T2. D5 lineage edge** with tests: claim → edge; deleted upload →
  `"deleted upload"` label; another tenant's claim → no edge. Update the
  module doc's edge table. Scoped Rust checks.
- **T3. D1 in the sensor** with unit tests in `test_authored_factory.py`
  (no network): a `file_ingest_job` success for the source table counts; one
  for another table does not; an authored or `ingest_job` upstream behaves as
  before. `pytest` for that file, the two `ops/lint` scripts.
- **T4. D3 and D4 in the console** with tests next to the page. Typecheck,
  lint, and the frontend tests of `src/features/pipelines/` only (one whole
  `bun test src` process is OOM-killed on this machine).
- **T5. The gate proves it.** Extend `ops/g9/upload_test.py`: upload a file
  into table T, create a Bronze-to-Silver pipeline over T with the switch ON,
  load it once and wait for the pipeline's run; re-upload T with one row
  changed; the pipeline runs again and the Silver table shows the change;
  upload a file into another table U; the pipeline does NOT run again; the
  lineage route shows `upload:<id> → bronze.T → silver.<out>`. Run the
  `g9-upload` CI job's sequence locally from a clean project and quote the
  gate's output and wall time. If the `g9-upload` job's `timeout-minutes`
  must rise, raise it by the measured time plus 5 minutes and say so.
- **T6. Docs.** `CHANGELOG.md`; the module docs above. Do NOT edit
  `docs/core/` (the planner does).

## 5. Out of scope

- `ingest_job` (connectors) firing every dependent on any connector run.
  Recorded by the planner as its own backlog item.
- A `dependsOn` editor for anything but the one switch.
- The assistant (D6).

## 6. Verify, do not assume

1. Does a `file_ingest_job` run's `run_config` reach the sensor through
   `context.instance.get_runs(...)` with `bronze_table_name` readable? Quote
   what the sensor sees.
2. Does the existing ALL-upstreams rule still hold when one upstream is
   `file_ingest_job` filtered by table and another is an authored pipeline?
3. Which catalog client (if any) gives table names per zone for D4?
4. What does the lineage route return for a claim whose `tenant_id` is NULL
   (a claim from before `0058`)? Follow the connectors' rule and say which.

## 7. Handoff

(Developer appends here, task by task.)

### T1 — D2 in the API (`1c9dc5b6`)

`validate_file_ingest_upstream(source_zone, depends_on)` in
`routes/authored_pipelines.rs`, called from `update`, `restore_version`
and `routes::pipelines::create` next to `validate_target_zone`. It reuses
`routes::uploads::FILE_INGEST_JOB` (the one definition of the job name)
rather than repeating the literal.

| Check | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean |
| `cargo test -p lakehouse-api --lib -- file_ingest` | 5 passed, 0 failed, 1655 filtered out |

Tests: `validate_file_ingest_upstream_refuses_a_non_bronze_source_and_accepts_bronze`
(the pure rule, both directions, `None` skipped, the message pinned and
asserted to name `FILE_INGEST_JOB`);
`create_with_a_file_ingest_upstream_on_a_silver_source_is_refused` and
`create_with_a_file_ingest_upstream_on_a_bronze_source_passes_the_guard`
(the accepted direction is proven by the guard PASSING: the route then
reaches `pool(&state)` and answers 503, not 400 — the two differ in one
field of the body);
`update_put_refuses_a_file_ingest_upstream_on_a_silver_source` and
`restore_of_a_file_ingest_upstream_version_returns_400` (real
`sqlx::test` Postgres; the restore seeds its pre-refusal version through
the store, since the routes now refuse that body).

### T2 — D5 lineage edge (`1bfc5a70`)

`uploads::list_claims(pool, tenant_id: Option<Uuid>)` in
`lakehouse-store` (one `LEFT JOIN file_upload`, returns
`ClaimedTable { bronze_table, upload_id, original_filename }`), consumed by
`routes/lineage.rs::add_claim_rows` — edge `upload:<id>` -> `bronze:<table>`,
kind `"upload"`, evidence `"upload table claim (upload_table_claim)"`. Label
is `original_filename` while the row exists, `"deleted upload"` when it does
not (a blank name is the same case). A claim table this build cannot read
costs a `coverage` note, not a failed request.

| Check | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -- -D warnings` | clean |
| `cargo test -p lakehouse-api --lib -- lineage` | 9 passed, 0 failed, 1653 filtered out |
| `cargo test -p lakehouse-store --test uploads -- list_claims` | 1 passed, 0 failed, 36 filtered out |

The plan's three cases: claim → edge
(`an_upload_draws_an_edge_into_the_bronze_table_it_claimed`), deleted
upload → `"deleted upload"` label and still drawn
(`an_upload_that_was_deleted_is_labelled_as_such_and_still_drawn`), and
another tenant's claim → no edge — the last is where the tenant filter
lives, so it is pinned on the store function
(`list_claims_shows_only_the_caller_s_tenant_and_names_the_upload_when_it_is_there`).

**Section 6, question 4 (answer).** A claim whose `tenant_id` is NULL is
drawn for an UNSCOPED caller (`tenant_id: None` from
`tenant_scope::resolve`, the `*:*` principal with no `X-Tenant`), which
shows every connector and every authored pipeline today, and for NO
tenant-scoped caller: `Some(t)` is `tenant_id = $1`, SQL `NULL = $1` is
never true. That is `ConnectorFilter`'s rule verbatim, and it fails
closed — a claim whose tenant is gone is nobody's, never everybody's.
Asserted in the store test above.

### T3 — D1 in the sensor (`c44edc3e`, `28942870`)

`authored_factory._run_config_value` + `_latest_run_for_table` +
`FILE_INGEST_OP_CONFIG` + `FILE_INGEST_SCAN_LIMIT`. Only the
`file_ingest_job` entry of the freshness walk is filtered; every other
upstream keeps `limit=1` and the unfiltered rule byte for byte. A
stored row whose source is not `bronze` but names `file_ingest_job`
(pre-dating the API's D2 refusal) matches nothing and ends the tick with
a reason that names the source.

| Check | Result |
| --- | --- |
| `python -m pytest dispar_orchestrate/test_authored_factory.py -q` | 63 passed, 0 failed |
| `python3 ops/lint/check_intra_package_imports.py` | OK: all intra-package imports resolve |
| `python3 ops/lint/check_bare_iceberg_count.py` | R11 guard: no bare count() against a Bronze Iceberg table |

**Section 6, question 1 (answer).** Yes — `bronze_table_name` is
readable on the run, so NO tag was added. Proven on a real run, not a
stub: `execute_in_process` against a temp instance with
`routes::uploads::run_config`'s launch shape verbatim, then
`instance.get_runs(RunsFilter(job_name="file_ingest_job"), limit=1)` —
the very call the sensor makes. The stored `run_config` is a `dict` and
resolves to

```json
{"ops": {"ingest_uploaded_file": {"config": {
  "bronze_table_name": "stock_raw", "delimiter": ",", "encoding": "utf-8",
  "header_row": 0, "load_mode": "replace",
  "storage_key": "uploads/t/up-1.csv", "upload_id": "up-1"}}}}
```

so the key is the OP name (`ingest_uploaded_file`), not the job name,
and every launch key survives resolution. Pinned by
`a_real_runs_config_resolves_to_the_table_the_load_wrote`.

**Section 6, question 2 (answer).** Yes, the ALL rule is unchanged.
The loop shape is identical: each upstream contributes either its
newest SUCCESS run id or ends the tick with its own `SkipReason`, and
one round asks for one run keyed on the sorted ids. Only WHICH run of
the filtered upstream is eligible changes. Pinned both ways by
`the_all_upstreams_rule_holds_with_a_filtered_and_an_authored_upstream`
(fires with `authored-deps:ours,upstream-a-1`; with the authored
upstream behind, the tick ends naming `pl-up-a` and asks for nothing).

### T4 — D3 and D4 in the console (`d2241ef0`)

`src/features/pipelines/pipeline-reupload-trigger.ts` (pure: the switch
is shown for a Bronze source only, ON for a new pipeline, and ON/OFF
touches ONLY `file_ingest_job`); `pipeline-create-page.tsx` renders it
in the source step and sends the chain on every save, so switching it
off clears a stored upstream. `dependsOn` is declared on
`CreatePipelineInput`, `UpdatePipelineInput` and `PipelineDetail` — it
was already on the wire (`authored_detail` returns the pipeline row);
the clients post their input unchanged, so no client code changed.

| Check | Result |
| --- | --- |
| `bun run typecheck` | clean |
| `bun run lint` | 0 errors, 6 warnings (all pre-existing, none in the files this task touched) |
| `bun test src/features/pipelines/` | 49 passed, 0 failed across 5 files |

**Section 6, question 3 (answer).** `lakehouseService`, not the data
catalog: `listNamespaces()` then `listTables(zone)` —
`/api/lakehouse/namespaces` and `/api/lakehouse/tables?namespace=…`
(`src/services/clients/lakehouse.ts`, WS2 §4). `/api/catalog` was the
other candidate and is NOT usable for this: its Bronze rows carry a
deployment-chosen namespace (`NAMESPACE_PRIMER`, `catalog.rs:1172`),
and its Silver/Gold `name` is `prettify(name)` (a display title), not
the table name. `listTables` answers for an Iceberg namespace, so
today the suggestions cover `bronze` — which is the whole of DATA-19's
"pick an uploaded table" case — and a ClickHouse zone has no
suggestions and stays free text. No API route was added. The form
checks the zone against the namespaces the same service lists before
asking for tables, so a `silver` zone never fires a listing that would
answer 404.

## 8. Review

(Reviewer.)
