# QA round, Data (P4): drop a table made by an upload, and restore it

One backlog row opened by the QA round on staging (2026-11-02 to 11-12): `DATA-55` (a Task
under `DATA-15`), decided by the product owner on 2026-11-12 as `DEC-21`: "a table made by an
upload can be dropped from the console, with the 7-day restore". QA cases: `LH-DATA-15-06` and
`TC-1731`, both **blocked**. Unlike the other slices of this round this is a small feature,
not a bug fix, so the plan also carries the spec lines and the feature-page text (T12).

Base: `main` after #497 (`fix/qa-round-web-11`) has merged; it merged on 2026-11-13 as `8b9c297c0`, and the branch is cut from there. Every anchor below was read at
`eaebacf28` (`origin/main` on 2026-11-13). The checkout `/tmp/lhmain` stood at `be2c92910`
then, which differs from `eaebacf28` in three documents only (`CHANGELOG.md`,
`docs/core/BACKLOG.md`, `docs/core/features/pipeline-joins-and-sql.md`). Branch:
`fix/qa-round-data-p4`. One pull request for the whole slice (12 tasks), opened as a draft
by the planner as soon as T1 is pushed, so CI (x86_64) runs every test.

Two open pull requests matter:

| Pull request | What it owns | What this plan does about it |
| --- | --- | --- |
| #497 (`fix/qa-round-web-11`) | `src/services/errors.ts`, `src/services/clients/assets.ts` and its test, `src/features/catalog/asset-detail-tabs.test.tsx`, `catalog-page.test.tsx`, `src/components/patterns/page-states.tsx` | This slice is built after it merges. T9 uses its `errorFor(status, body, fallback)` and `deniedFor`. For T9 to T11, **find each anchor by the text quoted in the task, not by its line number** |
| #486 (`fix/qa-round-dashboards-p1`) | `src/features/dashboards/*` | No task edits a file under `src/features/dashboards/` |

If the code is not what a task says, stop that task and report the mismatch in the handoff;
do not improvise.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit and does not change a decision below.

## What QA tried, and what the planner found by reading the code

`LH-DATA-15-06` (and `TC-1731`, the same finding): precondition "Admin (has
`governance:write`); a test Bronze Iceberg table that may be dropped"; steps "1. Drop the
table. 2. Open its page and restore it. 3. Read it." The tester (a Platform Admin) found no
way to do step 1: the table's page has no drop action, Sources → Uploaded files → Delete
removes the file only ("The table qa_d15_large stays, with its rows."), and Query Studio
refuses `DROP TABLE` as it is read-only. So the 7-day restore could not be tested.

Findings, each read at `eaebacf28`:

- **The restore exists; only the drop is missing.** `rust/crates/lakehouse-api/src/routes/
  catalog_restore.rs` has `GET /api/catalog/{id}/restore` (`catalog:read`, `:202`) and `POST`
  (`governance:write`, `:251`). The console has the client (`src/services/clients/
  assets.ts:208-221`) and the control: `DroppedTable` in `src/features/catalog/
  asset-activity.tsx:109-158` (Activity tab, Snapshots card). Nothing in `rust/` or
  `dagster/` drops an Iceberg table; the only drop in the repository is the gate step
  `ops/g3a/g3a_test.py:475-535`, which drops as the `dlt` principal straight through the
  Iceberg REST catalog and restores through the API.
- **The window is 7 days in the deployment template.** Both warehouse inits of
  `docker-compose.yml` write `{"delete-profile": {"type": "soft", "expiration-seconds":
  604800}}` (`:1408-1419`, `:2106-2117`), and `LakekeeperAdminClient::ensure_warehouse` does
  the same for a tenant warehouse (`rust/crates/lakehouse-auth/src/openfga.rs:60`,
  `:493-512`). No finding here. A warehouse whose init never ran keeps the catalog's own
  default, a hard delete; D3 closes that for drops made from the console.
- **The catalog calls the restore makes today** (`openfga.rs`): `GET /management/v1/
  warehouse` (`find_warehouse_id`, `:357`), `GET /management/v1/warehouse/{id}/
  deleted-tabulars` (`find_deleted_table`, `:389`), `POST /management/v1/warehouse/{id}/
  deleted-tabulars/undrop` (`undrop_table`, `:461`), `POST /management/v1/warehouse/{id}/
  delete-profile` (`set_soft_delete_profile`, `:493`). All four go out as the catalog's
  `admin` principal (`AppState::lakekeeper_admin`).
- **A dropped table keeps its page.** The catalog list and the table's page are built from
  the registry rows in `ClickHouse` (`lake.bronze_meta.dataset_catalog` and `dataset_sync`,
  `routes/catalog.rs:884-915`, `routes/catalog_profile.rs:302-321`), which a drop of the
  Iceberg table does not touch. The page then shows "No Iceberg table for this asset", when
  it was dropped, until when it can be restored, and Restore. So the page that restores is
  the page that can drop (D1).
- **What says "an upload made this table".** `upload_table_claim` (`rust/migrations/
  0058_upload_tenant_mode.sql`): one row per raw table name, with the tenant whose upload
  first asked for the name. It is never released, a connector cannot take a claimed name
  (`lakehouse_store::uploads::table_claimed`), and an upload cannot claim a name a connector
  loads (`routes/uploads.rs:2624-2671`, `ensure_table_free`).
- **Two gaps in the existing restore**, fixed in T5 because the drop makes the restore
  reachable for the first time: `POST …/restore` does not ask whether the caller's roles may
  read the table (`GET` does, `catalog_restore.rs:214`); and its audit entry is written
  after the undrop, with the result thrown away (`:306`, `let _ =`), and always as a
  `user` (`:310`).

## 1. Decisions

- **D1. One page: the table's page in the Catalog, Activity tab, Snapshots card.** While the
  table exists the card offers **Drop table**; once it is dropped the same card shows what
  it shows today: when it was dropped, until when it can be restored, and **Restore**. Why
  this page: the restore is already there and signed (`DATA-15-AC3`, "restored from its
  page"); the page survives the drop (finding above); and it is what QA's steps name. The
  Uploaded files tab is not used: it needs `connector:manage`, the restore needs
  `governance:write`, and a file's row is not the table (several files load one table).
  To find dropped tables again, the Catalog page gets a short "Dropped tables" list that
  links to those pages and says how many days are left (T7, T11). The list does not
  restore; the page does.
- **D2. Build the drop on the catalog's soft delete; keep the restore.** One new catalog
  call: `DELETE {LAKEKEEPER_BASE_URI}/catalog/v1/{warehouse id}/namespaces/bronze/tables/
  {table}`, with no `purgeRequested` (the request the G3a gate sends, and the one
  `iceberg-catalog-rest` 0.10.1's `drop_table` sends). That the catalog path prefix is the
  warehouse id is what `ops/gold_export/gold_export_test.py:289-292` relies on in CI.
- **D3. The drop goes out as the catalog's `admin` principal, and never without the 7-day
  keep.** The admin client already makes every other call of this feature (D2's list), so
  "who drops is who undrops", and no other principal of the API is given a new use. Before
  each drop the API writes the soft delete profile again (`set_soft_delete_profile`, the
  call the compose inits make; writing the same profile twice changes nothing). If that
  call fails, nothing is dropped. With no admin client (`AppState::lakekeeper_admin` is
  `None`) nothing can be dropped. **Not verified:** that the catalog lets `admin` drop a
  table the `dlt` principal created. T8 proves it against the real catalog in CI. If T8
  shows the catalog refuses, the developer stops and reports; the planner decides (the
  fallback read is the `gold-export` token, which holds `create, modify, select` on the
  warehouse like `dlt`).
- **D4. "Made by an upload", exactly.** A table can be dropped only when all of these hold:
  1. the asset id is a Bronze dataset's (not `silver.*`, not `serving.*`) and the registry
     gives it a non-empty table name (`catalog_profile::bronze_table_name`);
  2. `upload_table_claim` holds that name for a tenant the caller belongs to
     (`uploads::table_claim` answers `TableClaim::Ours` for one of `principal.tenant_ids`;
     there is no bypass for `*:*`, as on every `/api/uploads/{id}` route);
  3. no connector's ingest spec targets the name (`connectors::any_connector_targets`);
  4. no drop of it is on record inside its window (D10);
  5. no upload is loading into it (`file_upload.status = 'ingesting'`, asked after the same
     settle the upload routes do, `uploads::settle_loading_upload`);
  6. the admin client is there (D3).
  Anything else is refused. A connector's Bronze table, a pipeline's Silver or Gold table,
  a table another tenant's upload claimed, and a table nobody claimed all fail rule 1 to 3.
- **D5. The refusals.** The catalog's tenant rule first (`catalog_tenant_refusal`), then
  the role's table access (`ensure_asset_readable`), exactly as `GET …/restore` does, so
  "not there" and "not yours to read" are the same `404 Asset not found`. A table the
  caller can see but that fails D4 rule 1 to 3 is refused with **one** sentence, whichever
  rule failed, so the answer never says whose upload made a table:

  | Case | `POST …/drop` | `GET …/drop` |
  | --- | --- | --- |
  | Outside the tenant that owns the shared catalog | `403` with the tenant gate's reason | `200 { "supported": false, "reason" }` |
  | Unknown id, or hidden by the role's table access | `404 Asset not found` | the same `404` |
  | D4 rule 1, 2 or 3 | `409` `Only a table made by a file upload can be dropped here.` | `200 { "droppable": false, "reason" }` |
  | D4 rule 4 | `409` `This table is already dropped. It can be restored from its page.` | `droppable: false` |
  | D4 rule 5 | `409` `A file is being loaded into this table, so it cannot be dropped yet.` | `droppable: false` |
  | D4 rule 6 | `503` `Tables cannot be dropped on this install: restore is not configured, so a dropped table could not come back.` | `droppable: false` |
  | The 7-day keep could not be written | `503` `The catalog could not confirm that a dropped table is kept for 7 days, so nothing was dropped. Try again shortly.` | not asked |
  | The warehouse could not be looked up | `503` `The catalog could not be reached, so nothing was dropped. Try again shortly.` | not asked |
  | The audit entry could not be written | `503` `The drop could not be written to the audit trail, so nothing was dropped.` | not asked |
  | The catalog answers 404 to the drop | `409` `There is no table to drop: the catalog holds no table of that name.` | not asked |
  | The catalog answers another 4xx | `409` `The catalog refused to drop the table. Nothing was dropped.` | not asked |
  | The catalog does not answer, or answers 5xx | `503` `The catalog did not answer, so it is not known whether the table was dropped. Open the table's page again: it says so if it was.` | not asked |

  No sentence carries catalog or engine text (principle 4): `OpenfgaError` has no body, and
  the new file passes `tests/sec11_guard.rs` with no `ALLOWED` entry.
- **D6. Permission *(proposed, not yet signed)*: `governance:write` for the drop and for
  its status read.** It is the permission the signed decision 4 of
  `docs/core/features/asset-page-depth.md` gives the restore, so everyone who is offered
  **Drop table** is also offered **Restore** on the same page, and no signed decision
  changes. The policy table already calls the restore "a governance decision, like the
  certification mark" (`policy.rs:289-293`). No new permission id. The other candidate,
  `connector:manage` (who uploads), would let a person drop a table and not restore it,
  unless the restore's gate is widened too, which changes a signed decision and cannot be
  written as one `POLICY_TABLE` row (`tests/route_auth.rs:223-266` requires each permission
  of an any-of row to pass alone). Open question 1. The permission is one string in two
  `POLICY_TABLE` rows and one `hasPermission` call, so the owner's answer is a small
  change. The list of dropped tables is `catalog:read`, like `GET …/restore`.
  Because the roles page describes each permission, the description of `governance:write`
  in `lakehouse-auth/src/permission_catalog.rs:161` gains the drop (T4).
- **D7. The row count is the counted read.** `routes::quality::count_rows(state,
  "bronze.<table>")` (`routes/quality.rs:1509`): a scan with `WHERE 1`, so the R11 guard
  holds, stopped after 30 seconds, and `None` when the table cannot be read or counted.
  `None` is sent as `null` and shown as "could not be counted", never as 0. It is the whole
  table's count, not the caller's row-filtered view: what is dropped is the whole table.
  The status read counts only when asked (`?withRowCount=true`), so opening the page costs
  no scan; the dialog asks. The drop counts once more itself, for the record and the audit
  entry; it never takes a number from the request.
- **D8. Order: audit first, then the record, then the catalog; and the same for the
  restore.**
  1. `catalog.drop` with outcome `allowed` is written. If it cannot be written, the drop is
     refused (`503`, D5) and the catalog is not asked. Fail closed.
  2. The drop is recorded (D10). If that fails, the answer is the store's `database error`
     and the catalog is not asked.
  3. The catalog is asked to drop the table.
  4. On success: `catalog.drop` with outcome `executed` is written, best effort (a failure
     is logged; the `allowed` entry already names who, which table and its row count).
  5. On failure: the record of step 2 is removed, `catalog.drop` with outcome `failed` is
     written with the reason in our own words, best effort, and the answer is D5's.
  Two entries per action, because an audit event cannot be changed after it is written
  and an `executed` written before the catalog answered could be false (principle 2).
  If step 5's removal fails too, the record says "dropped" for a table that is there; it
  is logged as an error, new loads into the name are refused until the record expires
  (at most 7 days), and no data is lost. `catalog.restore` follows the same
  order: `allowed` first or the restore is refused, then the undrop, then `executed` or
  `failed`. Each entry names the principal (id, kind from `Principal::kind_for_audit`,
  display name), `resource_kind` `catalog`, `resource_id` the asset id, and `args`
  `{ "table", "rowCount", "keptDays" }` for a drop (`rowCount` is `null` when not counted).
- **D9. A second drop is a clear `409`, not a second success.** Answering "dropped" twice
  would put a second person's name on a drop they did not make. The record's primary key
  decides between two requests sent in the same moment (D10), as `claim_table` does for a
  name.
- **D10. A record of its own: `upload_table_drop` (migration `0141`).** The catalog's
  deleted list knows no tenant, no person and no row count, and it lists every dropped
  table of the warehouse, a connector's included. The product needs three things it cannot
  give: the list of this tenant's dropped tables, who dropped each and how many rows it
  held, and a cheap answer to "is this name dropped right now" at every load. One row per
  table name (the current state; the history is the audit trail, as for `file_upload`):
  `bronze_table` (primary key), `tenant_id` (the claim's tenant, `ON DELETE SET NULL`),
  `asset_id`, `dropped_by`, `row_count` (`NULL` is "not counted"), `dropped_at`,
  `expires_at` (`dropped_at` plus 7 days, by the database clock). A row counts while
  `expires_at > now()`. A restore removes it. A new drop of the same name replaces a row
  whose window has passed.
- **D11. The name stays closed while the table can be restored *(proposed, not yet
  signed)*.** The claim is never released, so today the tenant's next upload could load
  into the dropped name; the load job would create a new table of that name, and the
  dropped one could no longer be restored (`POST …/restore` answers `409`, "a table of that
  name exists again"). So a load into a name with a drop on record inside its window is
  refused: `409` `That table was dropped and can still be restored. Restore it from its
  page in the Catalog, or choose another name.` After the window the name takes a new load
  as before. A connector still cannot take the name (the claim stands). What the catalog
  itself allows for a soft-deleted name was **not measured**; the refusal does not depend
  on it. Open question 2.
- **D12. Nothing else is deleted, marked or cascaded.** Each with its reason:
  - *The upload's file rows* stay as they are, with their table name. Deleting a file
    never touched its table; dropping a table does not touch its files. A new load of one
    of them into the dropped name is refused by D11, with the reason.
  - *The registry rows in `ClickHouse`* stay. They are what keeps the table's page, and the
    orchestrator is their single owner (`EXPECTED_SCHEMAS`); the API writes none. After the
    window the asset is still listed, and its page says the window has passed (the existing
    sentence). Open question 3.
  - *`ClickHouse` objects.* The product creates none per uploaded table: reads go through
    the one `DataLakeCatalog` database (`ICEBERG_QUERY_DB`), which lists what the catalog
    lists (`dagster/dispar_orchestrate/connector_catalog.py:102-110`). Nothing to remove.
  - *Dashboards, charts, saved queries, quality rules, pipelines, marks, grants, lineage*
    stay. Those that read the table fail, as they do today for a table that is not there,
    until it is restored. The confirmation says so, and names how many saved queries and
    dashboards the page already knows read it (`AssetDetail.dependents`, which lists only
    those the person may open). No dependency scanner is built. Open question 4.
- **D13. The list of dropped tables.** `GET /api/catalog/dropped/tables` (`catalog:read`):
  the records of the caller's tenants (`principal.tenant_ids`) inside their window, newest
  first, at most 100, minus those the caller's roles may not read (`table_access_for`).
  Each row: `assetId`, `table` (`bronze.<name>`), `droppedBy`, `rowCount`, `droppedAt`,
  `expiresAt`. The console turns `expiresAt` into "N days left". The path has two literal
  segments so it cannot shadow an asset's own `/api/catalog/{id}` (the shape
  `routes/mod.rs:729-735` already explains for `access-requests`).
- **D14. The assistant.** This is not a dashboard (BI) feature, so assistant parity does
  not apply. The assistant gets **no** tool that drops or restores a table;
  `tests/fixtures/tool_schemas.json` does not change.
- **D15. Text on screen (English, as the console).**
  - Button: **Drop table**. Dialog title: **Drop bronze.<name>?**
  - Dialog, first line, one of: **Counting the rows of bronze.<name>…** ·
    **bronze.<name> holds 1,234 rows.** (`1 row` for one) · **The rows of bronze.<name>
    could not be counted.** · the server's reason when the table can no longer be dropped.
  - Dialog, impact box: **It can be restored from this page for 7 days. After that it is
    gone for good. Nothing else is deleted: pipelines, quality rules, saved queries and
    dashboards that read this table will fail until it is restored.** and, when the page
    knows of some: **3 saved queries and dashboards that you can open read it.** (`1 saved
    query or dashboard that you can open reads it.` for one).
  - Confirm: **Drop table** (destructive), disabled until the count has answered.
  - After the drop, a toast: **Table dropped** · **It can be restored from this page
    until <date>.**
  - Catalog page: heading **Dropped tables**, line **Dropped from the console. Open a
    table to restore it before its time runs out.**; per row: the table name (a link to
    its page), **dropped by <name>**, **1,234 rows** or **rows not counted**, **6 days
    left** (`1 day left`, and **less than a day left** for 0). When the list cannot be
    read: **The list of dropped tables could not be read.**
- **D16. One task per commit, in the order of section 2.** Each commit body cites
  `LH-DATA-15-06`, `TC-1731` and `DATA-55`. Each fix site carries a short comment naming
  them (rule 13), e.g. `// DATA-55 (QA LH-DATA-15-06, TC-1731): …`. Rust first (T1 to T7,
  the migration in T1 so the rebuild it forces is paid once), then the gate (T8), the
  console (T9 to T11), the documents (T12).

## 2. Tasks

Rules for every Rust task: no `unwrap`/`expect` outside `#[cfg(test)]`; every `Result`
function has `# Errors`; product names in doc comments are backticked (`Lakekeeper`,
`ClickHouse`, `Postgres`, `Iceberg`); SQL values are bound. **Never write the text
`count(` without a `WHERE` in the same statement in any file under `rust/crates/` or
`ops/`** (comments included): `ops/lint/check_bare_iceberg_count.py` reads tests and
comments too, and `bronze.` is close by in every file this plan touches. Name the helper
`count_rows` instead.

### T1. The record of a drop: migration `0141` and its store functions (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `rust/migrations/` (highest is `0140_pipeline_inputs_and_steps.sql`);
  `rust/crates/lakehouse-store/src/uploads.rs:1-128` (module doc), `:723-732`
  (`get_by_run_id`, the pattern for a `COLS` read), `:1373-1462` (`TableClaim`,
  `table_claim`, `claim_table`), `:1568-1582` (`table_being_loaded`);
  `rust/crates/lakehouse-store/src/lib.rs:31-34` (the comment that lists migrations);
  `rust/crates/lakehouse-store/tests/uploads.rs:151-170` (`claim`, the `sqlx::test` form).
- Change:
  1. New file `rust/migrations/0141_upload_table_drop.sql`. If another pull request took
     `0141` before the branch is cut, use the next free number, and say so in the handoff.
     Its header says, in plain sentences: what the table is (D10); why the catalog's
     deleted list is not enough (no tenant, no person, no row count, every table of the
     warehouse); that it holds one row per table name, the current state, and that the
     history is the audit trail; that `row_count` `NULL` means not counted and is never 0;
     that `tenant_id` has the shape of `upload_table_claim.tenant_id` (a row whose tenant
     is gone shows to nobody and still closes the name); that a row counts while
     `expires_at > now()`. Then:
     ```sql
     CREATE TABLE upload_table_drop (
         bronze_table TEXT        PRIMARY KEY,
         tenant_id    UUID        REFERENCES tenant(id) ON DELETE SET NULL,
         asset_id     TEXT        NOT NULL,
         dropped_by   TEXT        NOT NULL,
         row_count    BIGINT,
         dropped_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
         expires_at   TIMESTAMPTZ NOT NULL
     );
     CREATE INDEX upload_table_drop_tenant_idx
         ON upload_table_drop (tenant_id, dropped_at DESC);
     ```
  2. In `uploads.rs`, after `table_being_loaded`, add (all `pub`, each with a doc comment
     and `# Errors`), and a section "# Dropped tables (`DATA-55`,
     `0141_upload_table_drop.sql`)" in the module doc that says what D10 and D11 say:
     - `pub struct TableDrop { bronze_table: String, tenant_id: Option<Uuid>, asset_id:
       String, dropped_by: String, row_count: Option<i64>, dropped_at: OffsetDateTime,
       expires_at: OffsetDateTime }` (`Debug, Clone, PartialEq, Eq, FromRow`).
     - `pub struct NewTableDrop<'a> { bronze_table: &'a str, tenant_id: Uuid, asset_id:
       &'a str, dropped_by: &'a str, row_count: Option<i64>, kept_seconds: i64 }`.
     - `pub async fn record_table_drop(pool, new: &NewTableDrop<'_>) ->
       Result<Option<TableDrop>, StoreError>`: **one statement**, so the primary key
       decides between two requests:
       ```sql
       INSERT INTO upload_table_drop AS d
           (bronze_table, tenant_id, asset_id, dropped_by, row_count, dropped_at, expires_at)
       VALUES ($1, $2, $3, $4, $5, now(), now() + ($6::bigint * interval '1 second'))
       ON CONFLICT (bronze_table) DO UPDATE
          SET tenant_id = EXCLUDED.tenant_id, asset_id = EXCLUDED.asset_id,
              dropped_by = EXCLUDED.dropped_by, row_count = EXCLUDED.row_count,
              dropped_at = EXCLUDED.dropped_at, expires_at = EXCLUDED.expires_at
        WHERE d.expires_at <= now()
       RETURNING d.bronze_table, d.tenant_id, d.asset_id, d.dropped_by, d.row_count,
                 d.dropped_at, d.expires_at
       ```
       `fetch_optional`: `None` means a drop inside its window is already on record.
     - `pub async fn dropped_table(pool, table: &str) -> Result<Option<TableDrop>,
       StoreError>`: the row `WHERE bronze_table = $1 AND expires_at > now()`.
     - `pub async fn forget_table_drop(pool, table: &str) -> Result<bool, StoreError>`:
       `DELETE … WHERE bronze_table = $1`; `true` when a row went.
     - `pub async fn list_dropped_tables(pool, tenant_ids: &[Uuid], limit: i64) ->
       Result<Vec<TableDrop>, StoreError>`: `WHERE tenant_id = ANY($1) AND expires_at >
       now() ORDER BY dropped_at DESC, bronze_table LIMIT $2`.
     - `pub async fn loading_into(pool, table: &str) -> Result<Vec<Upload>, StoreError>`:
       `SELECT {COLS} FROM file_upload WHERE bronze_table = $1 AND status = 'ingesting'`,
       written like `get_by_run_id` (`AssertSqlSafe`, the `safe:` comment).
  3. `lib.rs:32`: add `` `0141_upload_table_drop.sql`, DATA-55 `` to the list in the
     comment (this is what makes `sqlx::test` see the migration).
- Check: six tests in `rust/crates/lakehouse-store/tests/uploads.rs`, each
  `#[sqlx::test(migrations = "../../migrations")]`, using that file's `tenant` helper:
  - `a_drop_is_recorded_once_and_a_second_one_inside_the_window_is_not`: the first
    `record_table_drop` returns the row with `expires_at - dropped_at` equal to
    `kept_seconds`; the second returns `None` and the row still names the first
    `dropped_by`.
  - `a_drop_whose_window_has_passed_no_longer_counts_and_the_next_one_replaces_it`: after
    `UPDATE upload_table_drop SET expires_at = now() - interval '1 second'`,
    `dropped_table` is `None`, and a new `record_table_drop` returns the new person.
  - `forgetting_a_drop_removes_it_and_says_whether_there_was_one`: `true`, then `false`.
  - `the_dropped_list_is_the_given_tenants_unexpired_drops_newest_first`: two tenants,
    one expired row; an empty `tenant_ids` lists nothing.
  - `a_drop_whose_tenant_is_gone_is_listed_for_nobody_and_still_closes_the_name`: after
    `UPDATE upload_table_drop SET tenant_id = NULL`, the list of every tenant is empty
    and `dropped_table` still returns the row.
  - `loading_into_lists_only_the_uploads_loading_into_that_table`: one `ingesting`, one
    `ingested` into the same table, one `ingesting` into another.
  All six fail to compile today (the functions do not exist).

### T2. The admin client can drop a table (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `rust/crates/lakehouse-auth/src/openfga.rs:101-109` (`classify_status`),
  `:452-478` (`undrop_table`); `rust/crates/lakehouse-auth/tests/openfga.rs:343-360`
  (`undrop_table_sends_the_table_id_as_a_table_target`, the pattern).
- Change: after `undrop_table` add
  `pub async fn drop_table(&self, warehouse_id: &str, namespace: &str, table: &str) ->
  Result<bool, OpenfgaError>`:
  1. If `namespace` or `table` is empty or holds a character other than an ASCII letter,
     a digit or `_`, return `Err(OpenfgaError::Rejected)` and send nothing (the two go
     into a URL path; an upload's table name is `[a-z0-9_]`).
  2. Send `DELETE {base_url}/catalog/v1/{warehouse_id}/namespaces/{namespace}/tables/
     {table}` with the `Authorization` header from `auth_header()`. **No query string**
     (no `purgeRequested`). A send failure is `OpenfgaError::Transport`.
  3. Status 404 is `Ok(false)`. Any other status goes through `classify_status`; a 2xx is
     `Ok(true)`.
  The doc comment says: this is the Iceberg REST catalog's drop, sent as the admin
  principal (D3); on a warehouse whose delete profile is soft the table is kept and can
  be undropped; the caller writes the profile first; and why there is no purge.
- Check: three tests in `tests/openfga.rs`:
  - `drop_table_sends_one_delete_to_that_warehouses_catalog_path_with_no_query`: a mock
    for `DELETE /catalog/v1/wh-1/namespaces/bronze/tables/orders` answering 204 with
    `.expect(1)`; the call returns `Ok(true)`; `server.received_requests()` shows the
    request's `url.query()` is `None` and its `Authorization` header is `Bearer <token>`.
  - `drop_table_is_false_for_a_table_that_is_not_there_and_classifies_the_other_refusals`:
    404 gives `Ok(false)`; 403 gives `Err(Rejected)`; 500 gives `Err(Server)`.
  - `drop_table_sends_nothing_for_a_name_that_is_not_a_plain_identifier`: `orders/../x`
    and the empty string give `Err(Rejected)` and the server received no request.

### T3. The drop status: may this table be dropped, and how many rows does it hold (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `rust/crates/lakehouse-api/src/routes/catalog_restore.rs:47`
  (`const BRONZE_NAMESPACE`), `:202-241` (`status`, the shape to follow);
  `routes/catalog.rs:210` (`catalog_tenant_refusal`), `:3358` (`ensure_asset_readable`),
  `:3531` (`annotation_pool`), `:3561` (`validate_annotation_id`);
  `routes/catalog_profile.rs:302` (`bronze_table_name`); `routes/uploads.rs:1632`
  (`settle_loading_upload`, `pub(super)`); `routes/quality.rs:1509` (`count_rows`);
  `routes/gold.rs:497` (a boolean query value read with `== Some("true")`);
  `routes/mod.rs:28-38` (module list), `:736-760` (`catalog_access_router`);
  `policy.rs:289-293` (the restore rows), `:1380-1392` (their unit test).
- Change:
  1. `catalog_restore.rs:47`: `const BRONZE_NAMESPACE` becomes `pub(super) const`.
  2. New file `rust/crates/lakehouse-api/src/routes/catalog_drop.rs`. Module doc: what the
     three routes are, D2 to D5, D7 and D8 in a few sentences each, and that nothing the
     catalog says reaches a response. It holds the sentences of D5 as `const`s, and:
     ```rust
     enum Readiness {
         Ready { table: String, tenant_id: Uuid, admin: Arc<LakekeeperAdminClient> },
         No(&'static str),
     }
     async fn assess(state: &AppState, pool: &PgPool, principal: &Principal, id: &str)
         -> Result<Readiness, ApiError>
     ```
     `assess` applies D4 in this order and returns `No(sentence)` at the first rule that
     fails: (1) `id` starts with `silver.` or `serving.`: not an upload's table;
     `bronze_table_name(state, id).await?` (its 404 passes through); an empty name: not an
     upload's table. (2) for each id of `principal.tenant_ids`,
     `uploads::table_claim(pool, tenant, &table).await?`; the first `TableClaim::Ours`
     gives `tenant_id`; none: not an upload's table. (3)
     `connectors::any_connector_targets(pool, &table).await?` true: not an upload's
     table. (4) `uploads::dropped_table(pool, &table).await?` is `Some`: already dropped.
     (5) for each row of `uploads::loading_into(pool, &table).await?`, call
     `super::uploads::settle_loading_upload(state, pool, row).await`; if a returned row's
     `status` is still `ingesting`: a load is in progress. (6)
     `state.lakekeeper_admin.clone()` is `None`: not set up. Otherwise `Ready`.
     A comment says why `catalog_restore::target` is not reused: its "not configured"
     answer comes before the question this route must answer first.
  3. Handler `pub async fn status(State, Path(id), Query(q), Extension(principal),
     headers) -> ApiResult<ApiJson<Value>>`, with `#[derive(Deserialize)] struct
     DropQuery { #[serde(default, rename = "withRowCount")] with_row_count:
     Option<String> }`:
     - `catalog_tenant_refusal` gives a reason: `Ok` `{ "supported": false, "reason" }`.
     - `validate_annotation_id(&id)?`; `ensure_asset_readable(&state, &principal,
       &id).await?`; `let pool = annotation_pool(&state)?`.
     - `assess`: `No(reason)` gives `{ "droppable": false, "reason": reason }`.
     - `Ready`: `{ "droppable": true, "table": "bronze.<table>", "keptDays": 7 }`
       (`keptDays` is `DELETE_RETENTION_SECONDS / 86_400`). Only when
       `q.with_row_count.as_deref() == Some("true")`, add `"rowCount"`:
       `count_rows(&state, &format!("bronze.{table}")).await`, a number or `null`.
       Without the flag the key is absent.
  4. `routes/mod.rs`: `mod catalog_drop;` beside `mod catalog_restore;`, and in
     `catalog_access_router`, after the restore route:
     `.route("/api/catalog/{id}/drop", get(catalog_drop::status))`.
  5. `policy.rs`, after the restore rows: a comment (D6 in two sentences, marked proposed)
     and `("GET", "/api/catalog/{id}/drop", Policy::RequiresPermission("governance:write")),`.
     Unit test beside the restore one: `the_drop_status_needs_governance_write`.
- Check: integration tests in `rust/crates/lakehouse-api/tests/catalog_restore.rs` (its
  module doc gains a paragraph: the file now also covers the drop). New helpers there:
  `drop_app(server, with_admin)` = the existing `app` plus `CATALOG_TENANT_ID` =
  `common::CATALOG_TENANT` and `ICEBERG_QUERY_DB` = `icecat_api`; `governor(pool)` =
  `create_principal_with_permissions(pool, "catalog:read, governance:write")` put in the
  catalog's tenant with `common::in_the_catalog_tenant`; `claim(pool, tenant, table)` =
  `uploads::claim_table(pool, tenant, table, "up-1")`, asserted `true`;
  `mount_row_count(server, n)` = two mocks mounted **before** `mount_registry` (wiremock
  takes the first that matches): `POST` whose body contains `DESCRIBE TABLE` answering
  `{"meta":[],"rows":1,"data":[{"name":"id","type":"String"}]}`, and `POST` whose body
  contains `WHERE 1 SETTINGS max_execution_time` answering
  `{"meta":[],"rows":1,"data":[{"n":"<n>"}]}`.
  - `the_drop_status_names_an_uploads_table_and_counts_its_rows_only_when_asked`: claim
    `orders` for the catalog's tenant; `GET /api/catalog/orders/drop` is `200`,
    `droppable: true`, `table: "bronze.orders"`, `keptDays: 7`, and has **no** `rowCount`
    key; with `?withRowCount=true` it has `rowCount: 1234`.
  - `the_drop_status_says_null_when_the_rows_cannot_be_counted_and_never_zero`: no
    `mount_row_count`; `rowCount` is `null`.
  - `the_drop_status_refuses_every_table_that_is_not_an_upload_of_the_callers_tenant_with_one_sentence`:
    four cases, each `200`, `droppable: false`, the same `reason`: no claim (a connector's
    Bronze table); a claim of another tenant (`…000000000002`); the caller's tenant's
    claim on a name a connector also targets (the one statement of
    `tests/upload_routes.rs:2598-2604`: `UPDATE connector SET source_objects = …` on the
    seeded `conn-pg-lakehouse`, with `"target":"orders"`); the id `silver.orders`.
  - `the_drop_status_of_a_table_the_callers_role_does_not_read_is_the_unknown_assets_404`:
    narrow the caller's role as `tests/table_access.rs:152-183` does (`read_scope =
    'listed'`, one `role_read_grant` for `('finance', '*')`); `404`, `error` is
    `Asset not found`.
  - `the_drop_status_says_a_load_is_in_progress`: an upload of the tenant left
    `ingesting` into `orders` (as `seed_new_upload` and `load` of
    `tests/connector_upload_table.rs:117-134` and `:77-113` do); `droppable: false` with
    the loading sentence.
  - `without_the_admin_client_the_drop_status_says_dropping_is_not_set_up`:
    `drop_app(server, false)`.
  None passes today: the route does not exist.

### T4. The drop (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: the file of T3; `rust/crates/lakehouse-store/src/audit.rs:169-248`
  (`NewAuditEvent`, `insert`); `routes/uploads.rs:2702-2725` (`upload_audit_event`, the
  shape of an event); `rust/crates/lakehouse-auth/src/openfga.rs:357`, `:493`;
  `rust/crates/lakehouse-auth/src/permission_catalog.rs:159-162`;
  `rust/crates/lakehouse-api/tests/permission_catalog.rs:125-150` (the fixture and its
  writer); `tests/route_auth.rs:985-1008` (`a_seeded_data_engineer_is_denied_the_undrop`).
- Change, in `catalog_drop.rs`:
  1. Two helpers, `pub(super)` (T5 uses them):
     `async fn audit_first(pool, event: NewAuditEvent, refusal: &'static str) ->
     Result<(), ApiError>` (a failed insert is logged with `tracing::error!` and returned
     as `ApiError::Unavailable(refusal.to_owned())`), and
     `async fn audit_after(pool, event: NewAuditEvent)` (a failed insert is logged with
     `tracing::warn!`, nothing else). A private `fn drop_event(principal, id, table,
     rows: Option<i64>, outcome: &str, detail: Option<&str>) -> NewAuditEvent`: action
     `catalog.drop`, `principal_kind` from `principal.kind_for_audit()`, `resource_kind`
     `catalog`, `resource_id` `id`, `args` `{ "table": table, "rowCount": rows,
     "keptDays": 7 }`, the rest as `upload_audit_event` sets them.
  2. Handler `pub async fn drop_table(State, Path(id), Extension(principal), headers) ->
     ApiResult<ApiJson<Value>>`, in this order (D5, D8):
     1. `catalog_tenant_refusal` gives a reason: `Err(ApiError::PermissionDenied(reason))`.
     2. `validate_annotation_id`, `ensure_asset_readable`, `annotation_pool`.
     3. `assess`: `No(NOT_SET_UP)` is `ApiError::Unavailable`; any other `No(reason)` is
        `ApiError::Conflict`.
     4. `admin.find_warehouse_id(&state.config.lakekeeper_warehouse)`: an error or `None`
        is `Unavailable` (the "could not be reached" sentence), logged with
        `tracing::warn!(?err, …)`.
     5. `admin.set_soft_delete_profile(&warehouse)`: an error is `Unavailable` (the
        "kept for 7 days" sentence), logged.
     6. `let rows = count_rows(&state, &format!("bronze.{table}")).await;`
     7. `audit_first(pool, drop_event(…, "allowed", None), NOT_AUDITED).await?`
     8. `uploads::record_table_drop(pool, &NewTableDrop { bronze_table: &table,
        tenant_id, asset_id: &id, dropped_by: &principal.display_name, row_count: rows,
        kept_seconds })`, with `kept_seconds` from `DELETE_RETENTION_SECONDS`
        (`i64::try_from(…)`, no `as`). `None`: `audit_after(… "refused", Some("already
        dropped"))` and `ApiError::Conflict` (already dropped).
     9. `admin.drop_table(&warehouse, BRONZE_NAMESPACE, &table).await`:
        - `Ok(true)`: `audit_after(… "executed", None)`; answer `{ "ok": true, "table":
          "bronze.<table>", "rowCount": rows, "droppedAt", "expiresAt" }`, the two times
          from the record as RFC 3339 (`OffsetDateTime::format(&Rfc3339)`).
        - anything else: `uploads::forget_table_drop(pool, &table)` (a failure of it is
          `tracing::error!` with the table name, and the answer does not change);
          `audit_after(… "failed", Some(<our words>))`; then `Ok(false)` is `Conflict`
          (no table), `Err(Rejected)` is `Conflict` (refused), `Err(Transport | Server)`
          is `Unavailable` (not known). The `detail` words: `the catalog holds no such
          table`, `the catalog refused the drop`, `the catalog did not answer; the
          outcome is not known`.
  3. `routes/mod.rs`: the route of T3 becomes
     `get(catalog_drop::status).post(catalog_drop::drop_table)`.
  4. `policy.rs`: `("POST", "/api/catalog/{id}/drop",
     Policy::RequiresPermission("governance:write")),` and the unit test
     `dropping_a_table_needs_governance_write`.
  5. `permission_catalog.rs:161`: the description becomes `Certify assets, set SLAs, run
     table maintenance, approve restorations, drop a table made by a file upload and
     restore a dropped table.` Then regenerate the fixture with the command the test
     names: `cargo test -p lakehouse-api --test permission_catalog
     write_permission_catalog_fixture -- --ignored`, and commit
     `tests/fixtures/permissions.json`.
- Check, in `tests/catalog_restore.rs` (helper `mount_drop(server, template, times)`: the
  `mount_warehouse` of that file, `POST /management/v1/warehouse/wh-1/delete-profile`
  answering 204, and `DELETE /catalog/v1/wh-1/namespaces/bronze/tables/orders` answering
  `template` with `.expect(times)`; helper `deletes(server)` = how many received requests
  have method `DELETE`). In every `SELECT count(*)` of these tests keep a `WHERE`.
  - `a_permitted_person_drops_an_uploads_table_and_the_catalog_is_asked_once`: `200`;
    `table`, `rowCount: 1234`; `expiresAt` minus `droppedAt` is 604,800 seconds; one
    `DELETE`, and the delete-profile `POST` was received before it; a row of
    `upload_table_drop` for `orders` with the caller's display name and `row_count` 1234.
  - `the_drop_writes_who_dropped_which_table_and_its_row_count_to_the_audit_trail`: two
    `audit_event` rows with action `catalog.drop` and `resource_id` `orders`, outcomes
    `allowed` then `executed`, each with the caller's `principal_id`, and `args` `table`
    `orders`, `rowCount` 1234.
  - `a_second_drop_of_a_dropped_table_is_a_409_and_the_catalog_is_not_asked_again`.
  - `a_table_that_is_not_an_upload_of_the_callers_tenant_is_not_dropped_and_the_catalog_is_not_asked`:
    the cases of T3's third test; each `409` with the one sentence; `deletes` is 0; no
    `audit_event` row with action `catalog.drop`.
  - `a_table_with_a_load_in_progress_is_not_dropped`: `409`; `deletes` is 0.
  - `a_caller_whose_role_does_not_read_the_table_cannot_drop_it_and_learns_nothing`:
    `404 Asset not found`; `deletes` is 0.
  - `a_drop_is_refused_when_its_audit_entry_cannot_be_written`: before the request,
    `ALTER TABLE audit_event ADD CONSTRAINT no_drop_audit CHECK (action <>
    'catalog.drop')` (each test has its own database); `503` with the audit sentence;
    `deletes` is 0; no row in `upload_table_drop WHERE bronze_table = 'orders'`.
  - `a_drop_is_refused_when_the_seven_day_keep_cannot_be_confirmed`: the delete-profile
    `POST` answers 500; `503` with that sentence; `deletes` is 0.
  - `a_catalog_that_does_not_answer_the_drop_leaves_no_record_and_no_upstream_text`: the
    `DELETE` answers 500 with a body holding a planted marker (as `tests/
    upstream_errors.rs:24` does); `503` with the "not known" sentence; the body does not
    hold the marker; no record; the audit trail holds `allowed` then `failed`.
  - `a_catalog_that_refuses_the_drop_or_has_no_such_table_is_a_409_with_no_upstream_text`:
    403 with the marker gives the "refused" sentence; 404 gives the "no table" sentence;
    no marker, no record in either.
  - `without_the_admin_client_nothing_is_dropped`: `503` with the "not set up" sentence.
  And in `tests/route_auth.rs`, beside `a_seeded_data_engineer_is_denied_the_undrop`:
  `a_seeded_data_engineer_is_denied_the_drop_and_its_status` (`GET` and `POST
  /api/catalog/commerce_orders/drop` as the seeded Data Engineer that test signs in: `403`,
  `permission_denied: governance:write`). The table-driven loops of that file cover the
  two rows both ways with no edit.

### T5. The restore asks the role's table access, writes its audit entry first, and clears the record (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `routes/catalog_restore.rs:251-322` (`restore`): `:260`
  (`validate_annotation_id(&id)?;`), `:276-281` (`Assessment::Exists`), `:295-305` (the
  undrop), `:306-320` (the audit entry); `tests/catalog_restore.rs:137-168` (the first
  test, which counts one audit row).
- Change, in `restore`:
  1. After `validate_annotation_id(&id)?;` add
     `crate::routes::catalog::ensure_asset_readable(&state, &principal, &id).await?;`
     with the comment `status` carries at `:212-213` (GOV-7 D6).
  2. In the `Assessment::Exists` arm, before the `409`, call
     `uploads::forget_table_drop(pool, &table)` best effort (log a failure): a table that
     is there is not dropped, whatever a leftover record says (D8).
  3. Replace the audit block. Before `admin.undrop_table`: `audit_first(pool,
     restore_event(…, "allowed", None), NOT_AUDITED_RESTORE).await?` with
     `NOT_AUDITED_RESTORE` = `The restore could not be written to the audit trail, so
     nothing was restored.` If the undrop fails: `audit_after(… "failed", Some(<our
     words>))`, then the error it returns today. If it succeeds:
     `uploads::forget_table_drop(pool, &table)` best effort (log a failure), then
     `audit_after(… "executed", None)`. `restore_event` keeps today's fields (`action`
     `catalog.restore`, `resource_kind` `catalog`, `resource_id` the id, `args`
     `{ "table", "wasRestorableUntil" }`) and takes `principal_kind` from
     `principal.kind_for_audit()` instead of the literal `"user"`.
  4. The `# Errors` of `restore` gains the `404` of a hidden asset and the `503` of the
     audit entry. The module doc says the order and why (D8).
- Check, in `tests/catalog_restore.rs`:
  - Edit `a_table_dropped_inside_its_window_is_reported_restorable_and_undropped`: the
    audit assertion becomes "one row with outcome `allowed` and one with `executed`"
    (two queries, each with its `WHERE … AND outcome = …`). The commit body says why the
    count changed (D8); this strengthens the test.
  - `restoring_a_table_dropped_from_the_console_clears_its_record`: a record seeded with
    `uploads::record_table_drop`; after `POST …/restore` is `200`,
    `uploads::dropped_table(&pool, "orders")` is `None`.
  - `a_restore_is_refused_when_its_audit_entry_cannot_be_written`: the constraint of T4
    for `catalog.restore`; `503` with the sentence; the undrop mock has `.expect(0)`.
  - `a_failed_undrop_keeps_the_record_and_writes_a_failed_audit_entry`: the undrop
    answers 500; `503`; the record is still there; outcomes `allowed` then `failed`.
  - `a_caller_whose_role_does_not_read_the_table_cannot_restore_it`: a narrowed role
    with `governance:write`; `404 Asset not found`; the undrop mock has `.expect(0)`.
  The existing `a_table_past_its_window_…_is_410`, `…_no_dropped_copy_…_is_404`,
  `…_could_not_tell_…_is_503` tests pass unchanged: those answers are given before any
  audit entry is written.

### T6. A load into a dropped table's name is refused while it can be restored (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `routes/uploads.rs:303-314` (the table sentences), `:2624-2671`
  (`ensure_table_free`; the arm `TableClaim::Ours => return Ok(()),` at `:2656`), `:69-74`
  (the module doc's "Fail closed on a table name"); `tests/upload_routes.rs:2835`
  (`after_a_delete_the_claim_remains_and_a_new_upload_of_the_tenant_loads_into_the_table`,
  the neighbour), `:997-1004` (the helper that reads the claims).
- Change: a new `const TABLE_DROPPED: &str = "That table was dropped and can still be
  restored. Restore it from its page in the Catalog, or choose another name.";`. The
  `TableClaim::Ours` arm becomes: read `uploads::dropped_table(pool, table)` (an error
  goes through the same `unchecked` as its neighbours: `503`, the name is not assumed
  free); `Some(_)` is `Err(ApiError::Conflict(TABLE_DROPPED.to_owned()))`; `None` is
  `Ok(())`. A comment says why (D11) and names the QA cases. The doc comment of
  `ensure_table_free` and the module doc gain the case.
- Check, in `tests/upload_routes.rs`, after the neighbour:
  - `a_load_into_a_table_that_was_dropped_and_can_still_be_restored_is_409_and_claims_marks_and_launches_nothing`:
    the tenant's claim on `stock_raw` and a record from `uploads::record_table_drop`; a
    new upload's ingest into `stock_raw` is `409` with `TABLE_DROPPED`; the upload's
    status is still `uploaded`; `Dagster` received no request.
  - `once_the_restore_window_has_passed_the_name_takes_a_new_load`: the same, then
    `UPDATE upload_table_drop SET expires_at = now() - interval '1 second' WHERE
    bronze_table = 'stock_raw'`; the ingest is launched as in the neighbour test.
  The first fails today (the load is launched).

### T7. The list of dropped tables (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `routes/catalog.rs:3200` (`table_access_for`), `:3242` (`asset_visible`, the
  use of `allows_key`); the files of T3 and T4.
- Change, in `catalog_drop.rs`: `pub async fn list(State, Extension(principal), headers)
  -> ApiResult<ApiJson<Value>>`: a tenant refusal is `Ok` `{ "supported": false,
  "reason", "tables": [] }`; else `annotation_pool`, `table_access_for`,
  `uploads::list_dropped_tables(pool, &principal.tenant_ids, 100)`, keep the rows where
  `access.is_all() || access.allows_key(&format!("bronze.{}", row.bronze_table))`, and
  answer `{ "tables": [ { "assetId", "table": "bronze.<name>", "droppedBy", "rowCount",
  "droppedAt", "expiresAt" } ] }` (D13). Route in `catalog_access_router`:
  `.route("/api/catalog/dropped/tables", get(catalog_drop::list))`, with a comment that
  the two literal segments keep it clear of `/api/catalog/{id}`. `policy.rs`:
  `("GET", "/api/catalog/dropped/tables", Policy::RequiresPermission("catalog:read")),`
  and the unit test `the_dropped_list_needs_catalog_read`.
- Check, in `tests/catalog_restore.rs`:
  - `the_dropped_list_shows_the_callers_tenants_unexpired_tables_they_may_read_with_when_each_expires`:
    four records (the caller's tenant; the caller's tenant, expired; another tenant; the
    caller's tenant, for a table the narrowed caller's role does not read). A caller
    with all tables sees the first and the fourth; the narrowed one sees only the first;
    each row has `assetId`, `table`, `droppedBy`, `rowCount`, and an `expiresAt` in the
    future.
  - `a_table_is_listed_after_its_drop_and_gone_from_the_list_after_its_restore`: the
    drop of T4, then the list holds `orders`; then (with `mount_missing_table`,
    `mount_dropped` and the undrop mock of that file) the restore, then the list is
    empty. Note in the test's doc comment: the asset itself stays in `GET /api/catalog`
    by design (D12); what a drop removes is the Iceberg table.

### T8. The gate drops and restores an uploaded table against the real catalog (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors: `ops/g9/upload_test.py:268` (`_wait_for`), `:291` (`_expect`), `:399-403`
  (`count_rows`), `:834-841` (`_settle`), `:1107-1111` (`_upload_one`), `:1206-1251`
  (`step_long_table_name`, the pattern of a self-contained step), `:1460`
  (`_delete_upload`), `:1699-1700` (`main`: `step_long_table_name`, `step_folders`).
- Change: a new `step_drop_and_restore(api, state: dict) -> None`, called in `main`
  right after `step_long_table_name(api, state)`. Its docstring names `DATA-55`,
  `LH-DATA-15-06` and `TC-1731` and says what it proves: that the API's catalog identity
  may drop a table an upload made, that the table is kept and comes back, and that the
  name is closed in between. Steps, each failing with a `G9Failure` that says what was
  expected and what came:
  1. `table = f"g9_drop_{secrets.token_hex(4)}"`, `slug = table.replace("_", "-")`;
     upload `id,name\n1,a\n2,b\n` with `_upload_one`; ingest it with the body
     `step_long_table_name` uses (`replace`); `_settle` says `ingested` with 2 rows; wait
     until `count_rows(api, table) == 2`.
  2. Wait (`retry_failures=True`) until `GET /api/catalog/{slug}/drop?withRowCount=true`
     is `200` with `droppable` true; then `table == f"bronze.{table}"` and
     `rowCount == 2`.
  3. `POST /api/catalog/{slug}/drop` is `200` with `rowCount == 2` and an `expiresAt`.
  4. A second `POST` is `409`.
  5. `GET /api/catalog/dropped/tables` holds a row whose `assetId` is `slug`.
  6. A new ingest of the same upload into `table` is `409` and its `error` is the
     sentence of T6 (a module constant, with a comment naming `TABLE_DROPPED`).
  7. Wait until `GET /api/catalog/{slug}/restore` says `restorable` true; `POST` it:
     `200`.
  8. The list of step 5 no longer holds `slug`; wait until
     `GET /api/lakehouse/tables/bronze/{table}` is `200`; wait up to 120 seconds
     (`retry_failures=True`) until `count_rows(api, table) == 2`.
  9. `_delete_upload(api, upload_id, state)`; print one `[g9] drop and restore: …` line.
  The row count goes through the file's own `count_rows` (it carries `WHERE 1`).
- Check: `python3 -m py_compile ops/g9/upload_test.py` locally; the proof is the
  `g9-upload` job of CI on the pull request (it runs whenever Rust changed). In the
  handoff: *not verified locally, CI*. If step 3 fails with the "refused" sentence, stop
  and report (D3); do not change the identity.

### T9. The console's contract and client for the three routes (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors (find by text; #497 moves the lines): `src/services/contracts/assets.ts`, the
  type `AssetRestoreStatus` and the two members `getRestoreStatus`, `restoreTable` of
  `interface AssetService`; `src/services/clients/assets.ts`, `function errorFor` (after
  #497 it takes `(status, body, fallback)` and calls `deniedFor` first) and the methods
  `getRestoreStatus`, `restoreTable`; `src/services/clients/assets.test.ts` (`stubFetch`,
  `json`).
- Change:
  1. Contracts, after `AssetRestoreStatus`, each with a doc comment naming its route:
     ```ts
     export type AssetDropStatus =
       | { supported: false; reason: string }
       | { supported?: undefined; droppable: false; reason: string }
       | { supported?: undefined; droppable: true; table: string; keptDays: number; rowCount?: number | null }
     export type AssetDropResult = { table: string; rowCount: number | null; droppedAt: string; expiresAt: string }
     export type DroppedTableEntry = {
       assetId: string; table: string; droppedBy: string
       rowCount: number | null; droppedAt: string; expiresAt: string
     }
     ```
     The comment on `rowCount` says: absent when not asked for, `null` when the rows
     could not be counted, never 0 for "unknown". In `AssetService`, after
     `restoreTable`: `getDropStatus(id: string, options?: { withRowCount?: boolean },
     signal?: AbortSignal): Promise<AssetDropStatus>`, `dropTable(id: string, signal?:
     AbortSignal): Promise<AssetDropResult>`, `listDroppedTables(signal?: AbortSignal):
     Promise<DroppedTableEntry[]>`.
  2. Client, after `restoreTable`, all through `apiFetch` and `errorFor`:
     `getDropStatus` reads `/api/catalog/${encodeURIComponent(id)}/drop`, with
     `?withRowCount=true` only when asked (fallback `Failed to check whether the table
     can be dropped`); `dropTable` posts to the same path (fallback `Failed to drop the
     table`) and returns the body; `listDroppedTables` reads
     `/api/catalog/dropped/tables` (fallback `Failed to list the dropped tables`) and
     returns `Array.isArray(json?.tables) ? json.tables : []` (so `supported: false`,
     which carries an empty list, is an empty list).
- Check, in `assets.test.ts`, a `describe("clickhouseAssetService: dropping a table")`:
  - "asks for the drop status without a count, and with one only when asked" (the two
    URLs);
  - "posts the drop and returns what the API says was dropped";
  - "throws the API's sentence as an invalid_request on a 409";
  - "throws permission_denied with the permission named on a 403" (body
    `permission_denied: governance:write`; `err.message` is `You don't have access. This
    needs the governance:write permission.`);
  - "reads the dropped tables, and an answer without a list as none".

### T10. Drop table on the table's page, with a confirmation that names the table and its rows (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors (by text): `src/components/patterns/confirm-action-dialog.tsx` (the props, and
  the confirm button's `disabled={confirming}`); `src/features/catalog/
  asset-activity.tsx`: `function DroppedTable`, `export function AssetActivity`, and the
  `SectionCard` with `title="Snapshots"` and its `action={…CountToggle…}`;
  `src/features/connectors/uploads-panel.tsx`, the `ConfirmActionDialog` whose title is
  `Delete ${pendingDelete.originalFilename}?` (the pattern: an inline `role="alert"`
  line for the action's error); `src/lib/notify.ts` (`notifySuccess`);
  `src/features/catalog/asset-detail-tabs.test.tsx`: `function stubApi`,
  `describe("Activity tab: a dropped table"`.
- Change:
  1. `confirm-action-dialog.tsx`: a new optional prop `confirmDisabled?: boolean`
     (default `false`); the confirm button is `disabled={confirming || confirmDisabled}`.
     Nothing else changes; every existing use is unaffected.
  2. New file `src/features/catalog/asset-drop.tsx` (`"use client"` first), exporting
     `DropTableAction({ asset, onDropped }: { asset: AssetDetail; onDropped: () => void
     })`:
     - `useService((s) => assetService.getDropStatus(asset.id, undefined, s),
       [asset.id])`. It renders **nothing** unless the status is a success with
       `droppable === true` (loading, an error, `supported: false` and `droppable:
       false` all render nothing).
     - Then a `Button` (`size="sm"`, `variant="outline"`) **Drop table** that opens a
       dialog component mounted only while open. The dialog reads
       `getDropStatus(asset.id, { withRowCount: true })` with its own `useService`, and
       renders `ConfirmActionDialog` with the texts of D15: `title` from the first
       status's `table`; `description` by the second read's state (loading; a number;
       `null` or absent; `droppable: false` gives its `reason`; an error gives its
       message); `impact` with `keptDays` and, when `asset.dependents.length > 0`, the
       dependents sentence; `confirmLabel="Drop table"`; `destructive`;
       `confirmDisabled` unless the second read is a success with `droppable === true`;
       `confirming` while the drop is pending; as children, the drop's error message in
       a `role="alert"` paragraph.
     - The drop is `useServiceAction((signal) => assetService.dropTable(asset.id,
       signal))`. On success: `notifySuccess("Table dropped", \`It can be restored from
       this page until ${formatDate(result.expiresAt)}.\`)`, close, `onDropped()`. On a
       refusal the dialog stays open with the sentence.
     - Numbers through `formatNumber` (`@/lib/format`). Imports from `@/services` only.
  3. `asset-activity.tsx`, in `AssetActivity`: `const { hasPermission } = useAuth()`;
     `const canDrop = hasPermission("governance:write") && iceberg.status === "success"
     && iceberg.data !== null && isIcebergCandidate(a) && Boolean(a.tableName)`. The
     Snapshots card's `action` becomes: the `CountToggle` when it showed before, and
     `<DropTableAction asset={a} onDropped={iceberg.reload} />` when `canDrop`, side by
     side in a `div` with `className="flex items-center gap-2"`; `undefined` when
     neither shows. A comment names the QA cases and says the permission check hides
     the control and the request (D6). `DroppedTable` is not changed: after
     `iceberg.reload()` the card shows it, with Restore.
- Check:
  - New `src/features/catalog/asset-drop.test.tsx` (render inside `AuthProvider`, stub
    `fetch` as `asset-detail-tabs.test.tsx` does, including `/api/auth/me`):
    - "names the table and its counted rows before the drop can be confirmed": the
      dialog shows `Drop bronze.demo_orders?` and `bronze.demo_orders holds 1,234 rows.`;
      the confirm button is disabled while the count read is held open and enabled
      after.
    - "says the rows could not be counted, and never shows 0" (`rowCount: null`).
    - "offers nothing for a table that was not made by an upload" (`droppable: false`).
    - "says how many saved queries and dashboards that the reader can open read the
      table" (two `dependents`).
    - "sends the drop once, says until when the table can be restored, and reloads the
      table" (one `POST`; `onDropped` called once; the toast's description).
    - "shows the server's sentence when the drop is refused, and stays open" (a `409`).
  - In `asset-detail-tabs.test.tsx`: `stubApi` gains an option `drop` (what `GET …/drop`
    answers; the default stays the file's "not stubbed" 404), and two tests in a new
    `describe("Activity tab: dropping a table (DATA-55)")`: "offers Drop table on the
    Snapshots card for a table an upload made" and "offers no Drop table, and asks the
    API nothing about it, without governance:write" (no request whose path ends in
    `/drop`).

### T11. The Catalog lists the dropped tables and the days each has left (LH-DATA-15-06, TC-1731; DATA-55)

- Anchors (by text): `src/features/catalog/catalog-page.tsx`, the `<PageHeader
  title="Catalog"` element; `src/lib/dashboard-organise.ts`, `export function daysLeft`
  (whole days left, 0 once due); `src/components/patterns/section-card.tsx`;
  `src/features/catalog/asset-activity.tsx`, the link form
  `/data/assets/${encodeURIComponent(…)}`.
- Change:
  1. New file `src/features/catalog/dropped-tables.tsx`, exporting
     `DroppedTablesNote({ now = Date.now() }: { now?: number })`:
     `useService((s) => assetService.listDroppedTables(s), [])`. Loading, or a success
     with no rows: render nothing. An error: one muted line, **The list of dropped
     tables could not be read.** Otherwise a `SectionCard size="sm"` with the heading
     and line of D15 and one row per table: a `Link` to
     `/data/assets/${encodeURIComponent(entry.assetId)}` showing `entry.table`,
     `dropped by ${entry.droppedBy}`, the rows (`formatNumber`, or `rows not counted`
     for `null`), and the days left from `daysLeft(entry.expiresAt, now)` (rule 4: no
     second day arithmetic).
  2. `catalog-page.tsx`: `<DroppedTablesNote />` directly after the `PageHeader`.
- Check:
  - New `src/features/catalog/dropped-tables.test.tsx` (`fetch` stubbed, `now` passed):
    "lists each dropped table with who dropped it, its rows and the days it has left,
    and links to its page"; "says the rows were not counted rather than 0"; "says less
    than a day is left on the last day"; "shows nothing when no table is dropped";
    "says so when the list could not be read".
  - Run `src/features/catalog/catalog-page.test.tsx` unchanged: its stub answers
    `/api/catalog/dropped/tables` with an object that has no `tables`, which T9 reads as
    none, so the page must look as before. If one of its tests fails, report which and
    why; do not edit the stub to hide it.

### T12. The documents: the spec lines, the feature page, the changelog (DATA-55)

Edit only what is listed. Every new decision is written with *(proposed, not yet
signed)* in its Signed cell; no existing Signed cell and no existing acceptance row is
edited. A line is wrapped like the lines around it.

- `docs/core/specs/data-15.md`:
  - In "Target specs", after the `| Restore |` row (`:28`), add:
    ```text
    | Drop | Nothing in the console drops a table: deleting an uploaded file keeps its table, and Query Studio is read-only (QA `LH-DATA-15-06`, `TC-1731`, 2026-11-06) | A table made by a file upload is dropped from its page, with the 7-day restore (product owner 2026-11-12, `DEC-21`). Tables made by a connector or a pipeline are not dropped from the console. Who may drop *(proposed: `governance:write`, the permission that restores)* |
    ```
  - At the end of "Benchmark" (`:33`), add a paragraph:
    ```text
    Dropping and undropping: the comparison's "Undrop a table" row says Databricks has it and Snowflake has it in SQL only (`reference/competitive-comparison.md`, Data matrix). That document gives no number for how long either keeps a dropped table, so the 7 days are the product owner's number (`DEC-21`), not a competitor's.
    ```
  - At the end of "Acceptance checklist" (after `:42`), add:
    ```text
    - `DATA-15-AC10` to `DATA-15-AC16` A table made by an upload is dropped from its page and restored there within 7 days; every other table is refused (`DATA-55`; the rows are on the feature page)
    ```
- `docs/core/features/asset-page-depth.md`:
  - "What the user can do when this is done", after item 5 (`:37`):
    ```text
    6. With the governance permission *(proposed, not yet signed)*: drop a table
       that a file upload made, from the table's page, after a confirmation that
       names the table and its row count, and restore it from the same page
       within 7 days (`DATA-55`, `DEC-21`).
    ```
  - "Not included", one more bullet:
    ```text
    - Dropping a table made by a connector or a pipeline (`DEC-21`).
    ```
  - "Asking the assistant", one more sentence: `The assistant has no tool that drops or
    restores a table (`DATA-55`).`
  - "Decisions": below the table, a new paragraph `**Added 2026-11-12 (`DATA-55`).** Row
    7 is the product owner's decision `DEC-21`; rows 8 to 13 are proposed and not yet
    signed.` and these rows appended to the table:
    ```text
    | 7 | Which tables can be dropped from the console. | A table made by a file upload, with the 7-day restore. Tables made by a connector or a pipeline are out of scope. | 2026-11-12 (`DEC-21`) |
    | 8 | Who may drop. | `governance:write`, the permission that restores (decision 4), so whoever can drop can also restore. The person must belong to the tenant whose upload made the table, and their roles must be allowed to read it. | *(proposed, not yet signed)* |
    | 9 | Where. | The table's page, Activity tab, Snapshots card: Drop table while the table exists, Restore once it is dropped. The Catalog lists the dropped tables with the days each has left; the list links to the page and does not restore. | *(proposed, not yet signed)* |
    | 10 | What the confirmation says. | The table's name and its row count, counted when the dialog opens. When the rows cannot be counted the dialog says so; it never shows 0 for an unknown count. It also says the table can be restored for 7 days and that what reads it will fail until then. | *(proposed, not yet signed)* |
    | 11 | What a drop leaves alone. | Everything but the table: its uploaded files, its entry in the Catalog, and the pipelines, quality rules, saved queries, dashboards, marks and grants that name it. Those that read it fail until it is restored. | *(proposed, not yet signed)* |
    | 12 | The table's name during the 7 days. | Closed: a new load into it is refused with a sentence that says to restore the table or choose another name. After the 7 days the name takes a new load. | *(proposed, not yet signed)* |
    | 13 | The audit trail. | A drop and a restore are each written before they happen (`allowed`) and again when they end (`executed` or `failed`), with who, which table and, for a drop, its row count. If the first entry cannot be written, nothing is dropped or restored. | *(proposed, not yet signed)* |
    ```
  - "Limits to tell a customer", at the end:
    ```text
    - Only a table that a file upload made can be dropped from the console, and only by someone of the tenant whose upload made it. A connector's table and a pipeline's table cannot (`DATA-55`, `DEC-21`).
    - A dropped table can be restored for 7 days from its page. After that it cannot come back. Whether the catalog then removes its files from storage was not measured.
    - Dropping a table deletes nothing else. Its uploaded files stay, and so does its entry in the Catalog, also after the 7 days. Pipelines, quality rules, saved queries and dashboards that read it fail until it is restored.
    - While a dropped table can still be restored, a file cannot be loaded into its name.
    - A table with a load in progress cannot be dropped until the load ends.
    - The row count in the confirmation is counted at that moment and can take up to 30 seconds on a large table; when it cannot be counted the dialog says so.
    - Dropping needs the restore to be configured on the install (the catalog's admin token). Without it nothing can be dropped.
    ```
  - "Spec rows", one more row:
    ```text
    | Drop: a table made by an upload, from its page | **Met in code** (`DATA-55`; proven against the real catalog by the G9 gate) |
    ```
    Write this row only if the `g9-upload` job is green on the pull request; otherwise
    write `**Built, not proven against the catalog**` and say so in the handoff.
  - "Acceptance checklist", after `DATA-15-AC9`, with an empty Result cell each:
    ```text
    | `DATA-15-AC10` | As `governance:write`, open the page of a table an upload made, Activity tab, and press Drop table | A dialog names the table and its row count (or says the rows could not be counted) and says it can be restored for 7 days | |
    | `DATA-15-AC11` | Confirm the drop | The page says the table was dropped and until when it can be restored; the Catalog lists it under Dropped tables with the days left; the audit trail names who dropped which table and its row count | |
    | `DATA-15-AC12` | Press Restore on the same page | The table is back with the same row count and can be queried; it is gone from Dropped tables | |
    | `DATA-15-AC13` | Open the page of a table a connector made, and of a Silver table | No Drop table; calling the drop route answers "Only a table made by a file upload can be dropped here." | |
    | `DATA-15-AC14` | Without `governance:write`, open an uploaded table's page and call the drop route | No Drop table; 403; nothing changes | |
    | `DATA-15-AC15` | While the table is dropped, load a file into its name | Refused with the sentence that says to restore it or choose another name | |
    | `DATA-15-AC16` | Start a load into an uploaded table and try to drop it while it loads | Refused: a file is being loaded into this table | |
    ```
- `docs/core/features/upload-file.md`, in the limits list, after the bullet that begins
  "The original file is kept until someone deletes the upload" (`:257-258`):
  ```text
  - Deleting an upload never drops its table. A table an upload made is dropped
    from the table's page in the Catalog (`DATA-55`, `asset-page-depth.md`), and
    while it can still be restored a file cannot be loaded into its name.
  ```
- `CHANGELOG.md`, under `## [Unreleased]`, `### Added`, one entry in the style of the
  entries there: a table made by a file upload can be dropped from its page in the
  Catalog and restored there for 7 days; the confirmation names the table and its row
  count; who may (`governance:write`, proposed); what is refused; that nothing else is
  deleted and the name is closed for 7 days; the three routes; migration `0141`;
  backlog `DATA-55`, QA `LH-DATA-15-06`, `TC-1731`.
- Do not edit `docs/core/BACKLOG.md`, `PRODUCT.md`, `HANDOFF.md`, `AGENTS.md` or
  `docs/CODE-STANDARD.md` (the reviewer does what is needed, after merge).

## 3. Verification

The common rules of this QA round apply (per commit, only what the commit touched; never
`cargo clean`; the two repository lints always), with the change of machine in section
3a: on that machine the touched integration test files are run, not left to CI. CI on
the pull request (x86_64) is the final word: a check that is green there and red here is
green, and the reverse is red.

Per commit:

- Rust, from `rust/`: `cargo fmt --check`, then `cargo clippy -p <crate> --all-targets --
  -D warnings` for each crate the commit touched, then the tests of the task. Counts are
  the file's own tests at `eaebacf28` plus this plan's; each `lakehouse-api` integration
  binary reports one more (section 3a). #497 or another merge may have moved a base
  count: quote what you see.

  | Task | Crates for clippy | Tests |
  | --- | --- | --- |
  | T1 | `lakehouse-store` | `cargo test -p lakehouse-store --test uploads` (65 + 6 = 71) |
  | T2 | `lakehouse-auth` | `cargo test -p lakehouse-auth --test openfga` (16 + 3 = 19) |
  | T3 | `lakehouse-api` | `cargo test -p lakehouse-api --lib the_drop_status_needs_governance_write`; `… --lib routes::route_policy_tests`; `cargo test -p lakehouse-api --test catalog_restore` (8 + 6 = 14); `… --test sec11_guard` (4); `… --test route_auth` (93) |
  | T4 | `lakehouse-auth`, `lakehouse-api` | `… --lib dropping_a_table_needs_governance_write`; `… --lib routes::route_policy_tests`; `… --test catalog_restore` (14 + 11 = 25); `… --test permission_catalog` (3); `… --test sec11_guard`; `… --test route_auth` (93 + 1 = 94) |
  | T5 | `lakehouse-api` | `… --test catalog_restore` (25 + 4 = 29); `… --test sec11_guard`; `… --test route_auth` |
  | T6 | `lakehouse-api` | `… --test upload_routes` (175 + 2 = 177); `… --test sec11_guard`; `… --test route_auth` |
  | T7 | `lakehouse-api` | `… --lib the_dropped_list_needs_catalog_read`; `… --lib routes::route_policy_tests`; `… --test catalog_restore` (29 + 2 = 31); `… --test sec11_guard`; `… --test route_auth` |

- Python (T8): `python3 -m py_compile ops/g9/upload_test.py`. Nothing under `dagster/`
  changes, so no pytest runs.
- TypeScript: `bun install --frozen-lockfile` once, then per commit `bun run typecheck`,
  `bun run lint`, and `bun test` on the touched test files:

  | Task | Test files |
  | --- | --- |
  | T9 | `src/services/clients/assets.test.ts` |
  | T10 | `src/features/catalog/asset-drop.test.tsx`, `src/features/catalog/asset-detail-tabs.test.tsx`, and one file that uses the dialog unchanged: `src/features/connectors/uploads-panel.test.tsx` |
  | T11 | `src/features/catalog/dropped-tables.test.tsx`, `src/features/catalog/catalog-page.test.tsx` |

  Do not run prettier.
- Always: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py`.
- No compose file changes, so no `docker compose` check.

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-store -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-store --test uploads
cargo test -p lakehouse-auth --test openfga
cargo test -p lakehouse-api --lib routes::route_policy_tests
cargo test -p lakehouse-api --lib policy::
cargo test -p lakehouse-api --test catalog_restore
cargo test -p lakehouse-api --test upload_routes
cargo test -p lakehouse-api --test permission_catalog
cargo test -p lakehouse-api --test sec11_guard
cargo test -p lakehouse-api --test route_auth
cd .. && python3 -m py_compile ops/g9/upload_test.py
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

The full `cargo test --workspace` and the `g9-upload` gate are CI's on the pull request;
list both in the handoff as *not verified locally, CI*. The handoff quotes each command
with its counts, names every test added or changed so it can be found in the CI log, and
says *not verified* for anything not run (rule 7).

What the planner read and did not run. If one of these answers differently, report the
answer and stop that task; the plan is corrected, not the assertion.

- That the catalog lets its `admin` principal drop a table the `dlt` principal created
  (D3). T8 is the proof. What is measured today: `dlt` drops its own table through the
  same request and the API restores it (`ops/g3a/g3a_test.py:475-535`, in CI).
- That the catalog's path prefix is the warehouse id for a `DELETE` as it is for the
  table load in `ops/gold_export/gold_export_test.py:289-292`.
- What the catalog allows for the name of a soft-deleted table (D11), and whether it
  removes a table's files from storage when the 7 days end (T12 says "not measured").
- How long `ClickHouse` needs to see a restored table again (T8 step 8 waits up to 120
  seconds; if that is not enough, report the time seen, do not shorten the check).
- That `INSERT … AS d … ON CONFLICT … DO UPDATE … WHERE d.expires_at <= now() RETURNING`
  returns no row when the `WHERE` is false (T1's first test is the proof).
- That `count_rows` reads an uploaded table's rows through `ICEBERG_QUERY_DB` in the
  gate's stack (T8 step 2), and that the test stand-ins of T3 answer it as written (the
  shapes are those of `tests/quality_on_load.rs:46-78`).
- That adding `ensure_asset_readable` to `POST …/restore` changes no existing test (the
  callers there hold `*:*`).
- That `audit_event` takes the `ALTER TABLE … ADD CONSTRAINT` the two "audit entry cannot
  be written" tests use (T4, T5). If it does not, report it; make the insert fail another
  way only with the planner's word.
- That the 30 seconds of the row count plus the catalog calls stay inside the 60-second
  request bound (`routes/mod.rs:197`). If a drop of a very large table times out, the
  count's bound is what to lower, in a follow-up, not the request bound.
- That the maintenance job leaves a dropped table's files alone: it visits only the
  tables the catalog lists (`dagster/dispar_orchestrate/maintenance.py:342-362`).
- That the catalog's `admin` token, read once at start and valid 30 days (ADR 0011's
  known gap), is still valid when a person drops: an expired one makes the catalog refuse
  (`409`, nothing dropped), which is honest but not explained on screen.

## 3a. Change of machine

- The developer works on the ARM64 build machine, in the checkout
  `/home/rismana/rantai/lakehouse-agents-1`.
- Before the first cargo command of a shell:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1 CARGO_BUILD_JOBS=6`.
- Plain `cargo` (there is no `cargo-locked` there).
- Per Rust commit: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets -- -D
  warnings`, the named unit tests as `cargo test -p <crate> --lib <name>`, and each
  touched integration test file as `cargo test -p lakehouse-api --test <file>` (Docker
  runs there).
- ALWAYS also `cargo test -p lakehouse-api --test sec11_guard` and `--test route_auth`
  when `lakehouse-api/src` is touched.
- Never `cargo test --workspace`, never a bare `cargo test -p lakehouse-api <filter>`,
  never `cargo clean`.
- Each integration test binary of `lakehouse-api` also runs one test from
  `src/tls_proxy.rs`, so its count is one more than the file's own tests.
- Before the first TypeScript check: `bun install --frozen-lockfile`.
- A migration edit forces a rebuild (`sqlx::test` embeds migrations).
- CI on the pull request (x86_64) is the final word.
- Work only under `/home/rismana/rantai/`. Start no container you do not remove.

## 4. Out of scope

- **Dropping a connector's or a pipeline's table** (`DEC-21` says so). The drop route
  refuses them; nothing else changes for them. The restore keeps restoring any Bronze
  table, as signed.
- **A new permission id.** None is added (D6). If the owner asks for one, it is
  registered in `lakehouse-auth/src/permission_catalog.rs` (and its fixture
  `tests/fixtures/permissions.json`), granted to a role by a migration, named in the
  `POLICY_TABLE` rows, and asserted in `tests/route_auth.rs`: its own small change.
- **A tool for the assistant** that drops or restores a table (D14).
- **Removing or hiding the Catalog entry of a table whose 7 days have passed.** The
  registry rows are the orchestrator's; the entry stays and its page says the window has
  passed. Open question 3.
- **A mark on the Uploaded files tab** that a file's table is dropped, and a pointer
  from its Delete dialog to the table's page. The tab needs `connector:manage` and the
  drop `governance:write`, so a pointer could send a person to a control they do not
  have; and `uploads-panel.tsx` stays untouched in this round.
- **Blocking a drop while a pipeline, a dashboard or a rule reads the table**, and any
  count of pipelines or rules in the confirmation: no store function answers "what
  reads this table" for those today, and none is built here (D12). Open question 4.
- **Who dropped it and how many rows it held, on the table's page itself** (the page
  shows the catalog's dates, as today; the Catalog's list shows the person and the
  rows).
- **Closing the last race** between a drop and a load started in the same instant: both
  checks can pass before either write lands. The result is a table that exists again
  and a record that says "dropped" until it expires (at most 7 days): loads into the
  name are refused meanwhile, and no data is lost. A call of the restore route clears
  the record (T5's `Exists` arm), though the page no longer offers the button. A lock
  shared by the two routes is its own change.
- **A dedicated catalog principal for the drop** (compose, the token issuer, a real
  `up`). The admin client is used (D3). Open question 6.
- **Checking the list of dropped tables against the catalog.** The list is the record;
  the page is the catalog. They differ only after a double failure (D8).
- Everything else in the QA round that is not on row `DATA-55`.

### Open questions for the owner

None of these blocks the plan.

1. **Who may drop a table made by an upload?** The plan builds `governance:write`, the
   permission the signed decision gives the restore, so whoever can drop can restore on
   the same page and nothing signed changes (D6). Recommendation: sign it. The
   alternative is `connector:manage` (the person who uploads): then the uploader can
   drop their own table, but either cannot restore it, or the restore's permission is
   widened for uploaded tables, which changes signed decision 4 of the feature page.
2. **May a file be loaded into a dropped table's name during the 7 days?** The plan
   refuses it, because a new table of that name makes the dropped one impossible to
   restore (D11). Recommendation: keep the refusal.
3. **Should a table whose 7 days have passed leave the Catalog?** Today its entry stays
   (also for a table dropped outside the console), and its page says the window has
   passed. Removing it means the API or the orchestrator deletes registry rows.
   Recommendation: its own backlog item.
4. **Should a drop be refused, or only warned about, while something reads the table?**
   The plan warns: the dialog says what will fail and counts the saved queries and
   dashboards the page already knows (D12). Deleting a connector is blocked while
   pipelines depend on it; doing the same here needs a "what reads this table" answer
   for pipelines and rules. Recommendation: warn now, block in a follow-up if wanted.
5. **Two audit entries per drop and per restore** (`allowed`, then `executed` or
   `failed`), so the trail is never wrong and nothing happens unaudited (D8). The Audit
   page shows both. Recommendation: accept; one entry is possible only if a drop may
   happen without one when the database fails at that moment.
6. **Is the catalog's admin identity the right one to drop tables?** The API already
   undrops with it. A principal of its own, limited to dropping, is cleaner and is a
   compose and token-issuer change. Recommendation: keep the admin identity for now.
7. **The description of `governance:write` on the roles page** now says it drops a
   table made by a file upload (T4). It follows question 1.

## 5. Handoff

## 6. Review
