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

Base `d3a2c42f`; branch `fix/qa-round-data-p4`; one commit per task, pushed after
each. Migration number used: **`0141`** (`rust/migrations/0141_upload_table_drop.sql`);
`ls rust/migrations | tail -3` showed `0138`–`0140` before it, so `0141` was free.

### Commits

| Task | Commit | Title |
| --- | --- | --- |
| T1 | `ed9e0012` | feat(store): the record of a dropped table (T1) |
| T2 | `7d22410e` | feat(auth): the admin client can drop a table (T2) |
| T3 | `d27f89d1` | feat(api): the drop status says whether a table may be dropped (T3) |
| T4 | `4d037b7a` | feat(api): drop a table made by an upload (T4) |
| T5 | `2ece7b0d` | feat(api): the restore asks table access, audits first and clears the record (T5) |
| T6 | `4fb64cc2` | feat(api): a load into a dropped table's name is refused while it can be restored (T6) |
| T7 | `947baca7` | feat(api): the list of dropped tables (T7) |
| T8 | `911aed0f` | test(g9): the gate drops and restores an uploaded table (T8) |
| T9 | `4306cc3c` | feat(console): the contract and client for the drop routes (T9) |
| T10 | `9dad6930` | feat(console): Drop table on the table's page (T10) |
| T11 | `13cf7592` | feat(console): the Catalog lists the dropped tables (T11) |
| T12 | `a4c751e0` | docs: the drop, in the spec, the feature page and the changelog (T12) |

### Tests added or changed, by name

- `rust/crates/lakehouse-store/tests/uploads.rs` (+6):
  `a_drop_is_recorded_once_and_a_second_one_inside_the_window_is_not`,
  `a_drop_whose_window_has_passed_no_longer_counts_and_the_next_one_replaces_it`,
  `forgetting_a_drop_removes_it_and_says_whether_there_was_one`,
  `the_dropped_list_is_the_given_tenants_unexpired_drops_newest_first`,
  `a_drop_whose_tenant_is_gone_is_listed_for_nobody_and_still_closes_the_name`,
  `loading_into_lists_only_the_uploads_loading_into_that_table`.
- `rust/crates/lakehouse-auth/tests/openfga.rs` (+3):
  `drop_table_sends_one_delete_to_that_warehouses_catalog_path_with_no_query`,
  `drop_table_is_false_for_a_table_that_is_not_there_and_classifies_the_other_refusals`,
  `drop_table_sends_nothing_for_a_name_that_is_not_a_plain_identifier`.
- `rust/crates/lakehouse-api/src/policy.rs` (+3 unit tests):
  `the_drop_status_needs_governance_write`, `dropping_a_table_needs_governance_write`,
  `the_dropped_list_needs_catalog_read`.
- `rust/crates/lakehouse-api/tests/catalog_restore.rs` (+7 for the status read):
  `the_drop_status_names_an_uploads_table_and_counts_its_rows_only_when_asked`,
  `the_drop_status_says_null_when_the_rows_cannot_be_counted_and_never_zero`,
  `the_drop_status_refuses_every_table_that_is_not_an_upload_of_the_callers_tenant_with_one_sentence`,
  `the_drop_status_of_a_table_the_callers_role_does_not_read_is_the_unknown_assets_404`,
  `the_drop_status_says_a_load_is_in_progress`,
  `without_the_admin_client_the_drop_status_says_dropping_is_not_set_up`,
  `the_drop_status_of_a_dropped_table_says_it_is_already_dropped` (the seventh
  covers D4 rule 4 / D5's "already dropped" row, which the task's list omitted but
  D5 requires every refusal to be tested);
  (+11 for the drop) `a_permitted_person_drops_an_uploads_table_and_the_catalog_is_asked_once`,
  `the_drop_writes_who_dropped_which_table_and_its_row_count_to_the_audit_trail`,
  `a_second_drop_of_a_dropped_table_is_a_409_and_the_catalog_is_not_asked_again`,
  `a_table_that_is_not_an_upload_of_the_callers_tenant_is_not_dropped_and_the_catalog_is_not_asked`,
  `a_table_with_a_load_in_progress_is_not_dropped`,
  `a_caller_whose_role_does_not_read_the_table_cannot_drop_it_and_learns_nothing`,
  `a_drop_is_refused_when_its_audit_entry_cannot_be_written`,
  `a_drop_is_refused_when_the_seven_day_keep_cannot_be_confirmed`,
  `a_catalog_that_does_not_answer_the_drop_leaves_no_record_and_no_upstream_text`,
  `a_catalog_that_refuses_the_drop_or_has_no_such_table_is_a_409_with_no_upstream_text`,
  `without_the_admin_client_nothing_is_dropped`;
  (+4 for the restore) `restoring_a_table_dropped_from_the_console_clears_its_record`,
  `a_restore_is_refused_when_its_audit_entry_cannot_be_written`,
  `a_failed_undrop_keeps_the_record_and_writes_a_failed_audit_entry`,
  `a_caller_whose_role_does_not_read_the_table_cannot_restore_it`;
  (+2 for the list) `the_dropped_list_shows_the_callers_tenants_unexpired_tables_they_may_read_with_when_each_expires`,
  `a_table_is_listed_after_its_drop_and_gone_from_the_list_after_its_restore`;
  (changed) `a_table_dropped_inside_its_window_is_reported_restorable_and_undropped`
  now asserts one `allowed` and one `executed` row (two queries), per D8 — a
  stronger assertion, not a weaker one.
- `rust/crates/lakehouse-api/tests/upload_routes.rs` (+2):
  `a_load_into_a_table_that_was_dropped_and_can_still_be_restored_is_409_and_claims_marks_and_launches_nothing`,
  `once_the_restore_window_has_passed_the_name_takes_a_new_load`.
- `rust/crates/lakehouse-api/tests/route_auth.rs` (+1):
  `a_seeded_data_engineer_is_denied_the_drop_and_its_status`.
- `ops/g9/upload_test.py`: `step_drop_and_restore` (T8).
- `src/services/clients/assets.test.ts` (+5, in `describe("clickhouseAssetService: dropping a table")`):
  "asks for the drop status without a count, and with one only when asked";
  "posts the drop and returns what the API says was dropped"; "throws the API's
  sentence as an invalid_request on a 409"; "throws permission_denied with the
  permission named on a 403"; "reads the dropped tables, and an answer without a
  list as none".
- `src/features/catalog/asset-drop.test.tsx` (new, 6): "names the table and its
  counted rows before the drop can be confirmed"; "says the rows could not be
  counted, and never shows 0"; "offers nothing for a table that was not made by an
  upload"; "says how many saved queries and dashboards that the reader can open
  read the table"; "sends the drop once, says until when the table can be restored,
  and reloads the table"; "shows the server's sentence when the drop is refused,
  and stays open".
- `src/features/catalog/asset-detail-tabs.test.tsx` (+2): "offers Drop table on the
  Snapshots card for a table an upload made"; "offers no Drop table, and asks the
  API nothing about it, without governance:write".
- `src/features/catalog/dropped-tables.test.tsx` (new, 5): "lists each dropped table
  with who dropped it, its rows and the days it has left, and links to its page";
  "says the rows were not counted rather than 0"; "says less than a day is left on
  the last day"; "shows nothing when no table is dropped"; "says so when the list
  could not be read".

Each task's first test was run once against the code before its fix (the fix
stashed), and failed:

- T1: `error[E0432]: unresolved imports ... NewTableDrop, dropped_table,
  forget_table_drop, list_dropped_tables, loading_into, record_table_drop`.
- T2: `error[E0599]: no method named drop_table ... on LakekeeperAdminClient`.
- T3: `404 ... {"error":"No such address."}` for `GET /api/catalog/orders/drop`.
- T4: `POST /api/catalog/orders/drop` answered 405 (the route had only `GET`).
- T5: `restoring_a_table_dropped_from_the_console_clears_its_record`: "the record is
  gone after the restore" failed — the record was still there.
- T6: the ingest answered `200` where the test expects `409`.
- T7: the list test failed (the route did not exist).
- T9: 5 of 17 failed with `clickhouseAssetService.listDroppedTables is not a
  function`.
- T10: the new test file could not import `./asset-drop` (module not found).
- T11: the new test file could not import `./dropped-tables` (module not found).
- T8 has no local test binary (Python gate); its proof is CI.

### Commands run, with their counts

Per commit (from `rust/`, `CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1`,
`CARGO_BUILD_JOBS=6`, plain `cargo`):

| Task | Runs |
| --- | --- |
| T1 | `cargo fmt --check` OK · `cargo clippy -p lakehouse-store --all-targets -- -D warnings` OK · `cargo test -p lakehouse-store --test uploads` **69 passed, 0 failed** |
| T2 | `cargo fmt --check` OK · `cargo clippy -p lakehouse-auth --all-targets -- -D warnings` OK · `cargo test -p lakehouse-auth --test openfga` **18 passed** |
| T3 | clippy `-p lakehouse-api` OK · `--lib the_drop_status_needs_governance_write` **1 passed** · `--lib routes::route_policy_tests` **5 passed** · `--test catalog_restore` **16 passed** · `--test sec11_guard` **4 passed** · `--test route_auth` **94 passed** |
| T4 | clippy `-p lakehouse-auth -p lakehouse-api` OK · `--lib dropping_a_table_needs_governance_write` **1** · `--lib routes::route_policy_tests` **5** · `--test catalog_restore` **27** · `--test permission_catalog` **2 passed, 1 ignored** (the fixture writer is `#[ignore]` by design) · `--test sec11_guard` **4** · `--test route_auth` **95** |
| T5 | clippy OK · `--test catalog_restore` **31** · `--test sec11_guard` **4** · `--test route_auth` **95** |
| T6 | clippy OK · `--test upload_routes` **178** · `--test sec11_guard` **4** · `--test route_auth` **95** |
| T7 | clippy OK · `--lib the_dropped_list_needs_catalog_read` **1** · `--lib routes::route_policy_tests` **5** · `--test catalog_restore` **33** · `--test sec11_guard` **4** · `--test route_auth` **95** |
| T8 | `python3 -m py_compile ops/g9/upload_test.py` OK |
| T9 | `bun install --frozen-lockfile` (779 installs, no changes) · `bun run typecheck` OK · `bun run lint` 0 errors, 11 warnings (all pre-existing) · `bun test src/services/clients/assets.test.ts` **17 passed** |
| T10 | `bun run typecheck` OK · `bun run lint` 0 errors · `bun test src/features/catalog/asset-drop.test.tsx` **6 passed** · `bun test src/features/catalog/asset-detail-tabs.test.tsx` **169 passed** · `bun test src/features/connectors/uploads-panel.test.tsx` **29 passed** |
| T11 | `bun run typecheck` OK · `bun run lint` 0 errors · `bun test src/features/catalog/dropped-tables.test.tsx` **5 passed** · `bun test src/features/catalog/catalog-page.test.tsx` **10 passed** (unchanged) |
| T12 | docs only: `python3 ops/lint/check_bare_iceberg_count.py` and `check_intra_package_imports.py` OK |

`python3 ops/lint/check_bare_iceberg_count.py` and `.../check_intra_package_imports.py`
were run before every commit and both passed every time.

Before handoff, on `a4c751e0`, in the foreground:

```text
cd rust && cargo fmt --check                                   OK
cargo clippy -p lakehouse-store -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings   OK
cargo test -p lakehouse-store --test uploads                    69 passed, 0 failed
cargo test -p lakehouse-auth --test openfga                     18 passed, 0 failed
cargo test -p lakehouse-api --lib routes::route_policy_tests     5 passed, 0 failed
cargo test -p lakehouse-api --lib policy::                      32 passed, 0 failed
cargo test -p lakehouse-api --test catalog_restore              33 passed, 0 failed
cargo test -p lakehouse-api --test upload_routes               178 passed, 0 failed
cargo test -p lakehouse-api --test permission_catalog            2 passed, 1 ignored
cargo test -p lakehouse-api --test sec11_guard                   4 passed, 0 failed
cargo test -p lakehouse-api --test route_auth                   95 passed, 0 failed
python3 -m py_compile ops/g9/upload_test.py                     OK
bun run typecheck                                              OK
bun run lint                                                   0 errors, 11 warnings (pre-existing)
bun run test                                                   4206 passed, 1 skipped, 0 failed (388 files)
python3 ops/lint/check_bare_iceberg_count.py                    OK
python3 ops/lint/check_intra_package_imports.py                 OK
git status                                                     clean
```

### Not verified

- **`cargo test --workspace`**: *not verified locally* — per section 3a and
  AGENTS.md, the full workspace run is CI's on the pull request.
- **The `g9-upload` gate**: *not verified locally, CI*. `py_compile` passed; the
  step's proof (that the catalog's admin identity may drop a `dlt`-made table and
  that it comes back) is the `g9-upload` job on the pull request.
- **The feature page's spec row** reads **"Built, not proven against the catalog"**,
  not "Met in code": the `g9-upload` job has not run on the pull request yet.
- **Compose**: no compose file changed, so no `docker compose` check.
- **`docker ps -a`** was checked before the first `cargo test --test` run: one
  `postgres:16-alpine` was already up (`24d7d1d1667e`, up 8 h, not mine). At the
  end, testcontainer leftovers were present (`0eccdc83`, `3051b903`, `f7c5e3e2`
  ClickHouse; `7d1ee824`, `4c6f2a2b` Postgres); I could not tell which of them my
  runs started on this shared machine, so I removed none rather than risk another
  agent's.

### MISMATCH

- **MISMATCH (T3/T4 tests): the seeded connector already targets `orders`.** The
  task's third status test says to `UPDATE connector SET source_objects = …` on the
  seeded `conn-pg-lakehouse` "with `"target":"orders"`", implying the seed did
  not target it. `0034_seed_connector_ingest_spec.sql` seeds
  `[{"name":"ingest_demo.orders",…,"target":"orders"}]`, so every `orders` asset
  is a connector's table on a fresh database, and every case was refused by D4
  rule 3. I did not change a decision: `drop_app` in `tests/catalog_restore.rs`
  clears that seed's `source_objects` (with a comment saying why), and the one
  test that asserts the connector rule writes the target back.
- **MISMATCH (counts): the per-file base counts in section 3 are one higher than
  this branch has.** Measured here: `--test uploads` 63 base + 6 = 69 (plan: 71);
  `--test openfga` 15 + 3 = 18 (plan: 19); `--test catalog_restore` 8 base + 24 new
  (7 status + 11 drop + 4 restore + 2 list) + 1 `tls_proxy` = 33 (the plan's
  numbers add up to 31 + 1; the seventh status test is the difference);
  `--test upload_routes`
  175 + 2 + 1 `tls_proxy` = 178 (plan: 177); `--test route_auth` 93 base + 2 = 95
  (plan: 93 then 94 — the table-driven sweep adds one case per new `POLICY_TABLE`
  row, and there are two new rows now). CI is the final word.
- **Note (not a mismatch): `daysLeft` cannot return 0 before expiry.** It is
  `Math.ceil`, so a table with an hour left reads "1 day left"; 0 (and so D15's
  "less than a day left") is only reachable at or after `expiresAt`. Implemented as
  D15 says, with `daysLeft` as the one definition of that arithmetic; the test pins
  `expiresAt` to `now`.
- **Note: the R11 lint reads doc comments.** A doc sentence containing the text
  `count (` next to `bronze.` is flagged; one comment in `catalog_drop.rs` was
  re-worded (no predicate added, no test or threshold changed).

## 6. Review

### Round 1 (planner, 2026-11-13, at `285af382`)

Reviewed against sections 0 to 4 as a feature that drops data (an independent reviewer's
notes, then the planner's rulings). The Rust code was read, not built, on the planner's
machine; CI on the pull request (#501) is the run and was still going. The console was run
in a clean checkout of the branch: `bun run typecheck` clean; `bun run lint` 0 errors, 11
warnings; `bun run test` 4206 pass, 1 skip, 0 fail across 388 files.

What holds: no path was found by which a connector's table, a Silver or Gold table, another
tenant's table, an unclaimed table or a table being loaded reaches the catalog's `DELETE`;
there is no `*:*` bypass and no purge; the name checked is the name sent; the 7-day keep is
written before every drop; no existing test was weakened, removed, renamed or ignored. The
developer's MISMATCH about the seeded connector changes the tests' own database only, and a
test where a connector targets the name and the drop is refused is still there.

Two BLOCKERs, both about a state nobody recorded. R2 is the planner's own error in D8.
The fixes go in the commits named below, in this order, after `main` is merged into the
branch (it moved: `43d36f55b`; the merge is clean by `git merge-tree`). One commit per R
group as listed; push after each.

**Commit A, `fix(review): the drop keeps its record when the outcome is not known (R1, R2, R7)`**

- **R1. BLOCKER: a hanging catalog or a closed tab stops the drop between the record and its
  ending.** The admin client has no request timeout (`lakehouse-auth/src/openfga.rs`,
  `reqwest::Client::new()`), so a catalog that accepts the connection and does not answer is
  ended only by the route's 60-second deadline (`routes/mod.rs`, the `tokio::time::timeout`
  around `next.run`), which drops the handler at its current `.await`; a client that
  disconnects drops it the same way. If that lands inside `admin.drop_table(...)`, the
  record and the `allowed` entry stay, no `executed` or `failed` is written, nothing is
  logged, and the caller gets a 408. Fix, all three parts:
  1. In `LakekeeperAdminClient::drop_table`, the `DELETE` request gets its own bound,
     `.timeout(Duration::from_secs(15))`, with a named constant and a doc saying why (a
     hang becomes `OpenfgaError::Transport`). Do not change the client's other calls.
  2. In `routes/catalog_drop.rs`, everything from the first audit entry (`audit_first`) to
     the closing audit entry runs inside ONE `tokio::spawn`ed task that the handler awaits
     (`handle.await`, a join error is a 500 with a fixed sentence and one `tracing::error!`).
     A deadline or a disconnect then stops only the waiting, never the work between the
     record and its ending. The checks before the audit entry stay in the handler. Say this
     in a comment at the spawn (`DATA-55 review R1`). Clone or move what the task needs
     (`AppState` is `Clone`); no reference may cross the spawn.
  3. Do the same in `catalog_restore.rs` for the restore's stretch from its first audit
     entry to its closing one.
- **R2. BLOCKER (D8 step 5 was wrong): an unknown outcome removed the record.** For
  `OpenfgaError::Transport` and `Server` the answer says "it is not known whether the table
  was dropped", and the code then removed the record as if it were known that it was not.
  If the catalog had dropped the table, the name was open to the very load D11 exists to
  refuse, the table was missing from the list of dropped tables, and the trail's last word
  was `failed`. **D8 step 5 is replaced by this rule** (the planner's decision):
  - A definite refusal by the catalog (a 4xx): as built. The record is removed, `failed`
    is written, the answer is D5's.
  - An unknown outcome (`Transport`, `Server`): ask the catalog once whether the table is
    among the deleted ones, `admin.find_deleted_table(&warehouse, BRONZE_NAMESPACE, &table)`,
    inside `tokio::time::timeout(Duration::from_secs(10), …)`:
    - it is there: the drop happened. Keep the record, write `executed`, answer `200` as a
      successful drop;
    - it is not there AND `catalog_restore::table_exists` says the table exists: the drop
      did not happen. Remove the record, write `failed`, answer D5's `503` "The catalog did
      not answer, so it is not known …" is no longer true here, so answer `503` with a new
      fixed sentence: `The catalog did not answer and the table was not dropped. Try again
      shortly.`;
    - anything else (the lookup failed, timed out, or the two answers do not agree): **keep
      the record** (the name stays closed: fail closed), write `failed` with the reason
      `the outcome is not known`, answer `503` with D5's existing sentence ("… it is not
      known whether the table was dropped. Open the table's page again: it says so if it
      was.").
  - **A kept record must not lock a live table for 7 days.** Where a record is read to
    refuse something (`assess` rule 4 in `catalog_drop.rs`, and the dropped-name refusal
    T6 added to `ensure_table_free` in `routes/uploads.rs`): believe the record only while
    the table is absent. Call `catalog_restore::table_exists` (the function the restore
    already uses; make it `pub(super)` if needed, do not copy it): if the table EXISTS, the
    record is stale: remove it (`forget_table_drop`), log one `tracing::warn!` with the
    table name, and go on as if there were no record; if the check itself fails, the record
    stands (treated as dropped). The list of dropped tables (`GET /api/catalog/dropped/tables`)
    is not changed by this: it shows records, and a stale one disappears at the next read
    that heals it.
  - The restore's mirror case (`catalog_restore.rs`): an undrop that answers `Transport` or
    `Server` may have happened. Apply the same idea: check `table_exists` once (10-second
    bound); if the table exists, the restore happened: clear the record, write `executed`,
    answer as a success; otherwise leave the record, write `failed`, answer as today.
  - Rewrite the test `a_catalog_that_does_not_answer_the_drop_leaves_no_record_and_no_upstream_text`
    for the new rule (rule 2: the old assertion pinned the planner's wrong decision; say so
    in the commit body) into three tests, full-sentence names: the catalog answers 500 and
    then lists the table as deleted (record kept, `allowed` then `executed`, 200); answers
    500 and the table still exists (record removed, `failed`, the new 503 sentence); answers
    500 and the lookup fails too (record KEPT, `failed`, D5's 503 sentence, no upstream
    text). Add: a catalog that does not answer at all within the client's bound
    (`ResponseTemplate::new(204).set_delay(Duration::from_secs(20))`; to keep the test
    short, the bound may be read from a `#[cfg(test)]`-settable constant or the client may
    take the bound as a constructor argument the test sets to 1 second: choose the smaller
    change and say which) ends as the third case. Add: a stale record for a table that
    exists is removed by the next status read and by the next load, and the load goes
    through. Add the two tests for the restore's mirror case.
- **R7. SHOULD-FIX: an `allowed` entry with no ending, and a refusal blamed on the
  catalog.** (a) Compute `kept_seconds` (the `i64::try_from` guard) BEFORE `audit_first`,
  and when the record insert fails write `failed` ("the drop could not be recorded") before
  returning the error, so every `allowed` has an ending (D8: two entries per action).
  (b) A registry table name that is not a plain identifier is refused inside the client
  after `allowed` and the record are written, and is then audited as "the catalog refused".
  Test the name in `assess` rule 1 with the client's own check (expose
  `is_plain_identifier` from `lakehouse-auth`; do not write a second copy) and answer D5's
  one sentence `Only a table made by a file upload can be dropped here.` there, before
  anything is written. One test: a registry name with a `/` in it is refused with that
  sentence, no audit entry, no record, no `DELETE`.

**Commit B, `fix(review): a load and a drop of one table cannot both pass (R3)`**

- **R3. SHOULD-FIX: up to 30 seconds lie between "no load is running" and the record.**
  Rule 5 is read once in `assess`; the row count (up to 30 seconds) and two catalog calls
  run before the record closes the name; on the load side `ensure_table_free` runs before a
  workbook conversion and `claim_load`. A load and a drop can each pass the other's check.
  Section 4 called this "the same instant"; it is tens of seconds. Each side now writes
  first and checks second, so one of the two always sees the other:
  1. `drop_table`: directly after the record is written, ask `uploads::loading_into` again
     (settled as in `assess`). If a load is `ingesting`: remove the record, write `failed`,
     answer the existing `409` "A file is being loaded into this table, so it cannot be
     dropped yet.", and do not ask the catalog.
  2. `ingest` (`routes/uploads.rs`): directly after `claim_load` marked the upload
     `ingesting`, read `uploads::dropped_table` for the name (with R2's healing rule). If
     it is dropped: settle the claim as failed the way a refused launch does (find the
     existing path; do not write a new one) and answer T6's `409`, before `launch`.
  Tests: one where the record appears between `ensure_table_free` and `claim_load` (write
  the record directly through the store in the test, with the stand-ins arranged so the
  load reaches `claim_load`): the load answers T6's 409, nothing is launched, the upload is
  not left `ingesting`. One where an upload is `ingesting` when the drop's second check
  runs: 409, no `DELETE`, no record left, `allowed` then `failed`. If the first cannot be
  arranged through the route without a hook, write a unit-level test of the second check
  and say in the handoff what is not covered.

**Commit C, `test(review): the refusals and failure paths of the drop that had no test (R5)`**

- **R5. SHOULD-FIX: rows of D5 and paths of D8 with no test.** Add, in
  `tests/catalog_restore.rs`, full-sentence names, each asserting the status, the exact
  sentence, that the catalog stand-in received NO `DELETE` where one must not be sent, and
  the record and audit state:
  1. the tenant gate on the three routes: `403` with the gate's reason on `POST …/drop`,
     `200 { "supported": false }` on `GET …/drop`, `{ "supported": false, "tables": [] }`
     on the list;
  2. an unknown id is `404 Asset not found` on `GET` and `POST …/drop` (the registry
     stand-in must answer "no such dataset" for that id: today it names a table for every
     slug);
  3. the warehouse cannot be looked up (an error, and "no such warehouse"): `503`, D5's
     sentence, no `DELETE`;
  4. the record insert fails: the store's `database error`, no `DELETE`, `allowed` then
     `failed` (after R7);
  5. two `POST …/drop` at once (`tokio::join!`): exactly one `200`, one `409`, one `DELETE`;
  6. a drop whose rows cannot be counted still drops, with `rowCount: null` in the answer,
     in the record and in both audit entries;
  7. the list for a `*:*` caller who is in no tenant is empty, and the list stops at 100;
  8. `the_drop_status_says_null_when_the_rows_cannot_be_counted_and_never_zero` gets its
     `null` from a count query the stand-in answers with a 500, not from a registry row
     without `n`. Change the stand-in for that test, not the assertion.
  If a test cannot be made to fail-then-pass honestly (for example the record insert
  failure needs a fault the test database cannot produce without changing product code),
  do not fake it: say in the handoff which case is not tested and why.

**Commit D, `docs(review): comments say what the code does, and cite the cases (R6, R9, R10)`**

- **R6. SHOULD-FIX (rule 1): doc comments that say what the code does not do.**
  `catalog_drop.rs`: `status` and `list` claim a `403` for the tenant gate, they answer
  `200` with `supported: false`; `drop_table`'s `# Errors` omits the `400` of an over-long
  id and the `500`s; the module doc lists two routes, the file has three; "Each failure
  before the catalog answers leaves nothing behind" is rewritten to what holds after R1,
  R2 and R7. `rust/migrations/0141_upload_table_drop.sql` (not applied anywhere yet, so its
  header may be corrected in this branch): the crate is `lakehouse-auth`, not
  `lakekeeper-auth`; and "so the API and the catalog agree on when the window ends" is not
  true (the catalog stamps its own, later time): say the record's end is the API's own
  clock and may differ from the catalog's by the seconds between the insert and the
  `DELETE`. `lakehouse-store/src/uploads.rs`, the list's doc: "because no caller of the
  list has none" is false for a `*:*` principal in no tenant; say that such a caller is
  shown nothing. `lakehouse-auth/src/openfga.rs`, `drop_table`'s doc: `Transport` also
  covers a connection lost after the catalog acted. `src/features/catalog/dropped-tables.tsx`:
  the comment about `clockNow()` says the opposite of where it is called; make it true.
- **R9. SHOULD-FIX: small console texts.** `dropped-tables.tsx`: one row reads "1 rows";
  use the singular as the dialog does, and add the case to `dropped-tables.test.tsx`. The
  test "says less than a day is left on the last day" pins `expiresAt` equal to now, which
  the server never lists: rename it to what it pins (`says less than a day is left for a
  row at its end`); D15's text is unchanged. In `asset-detail-tabs.test.tsx`, "offers Drop
  table on the Snapshots card…" must find the button inside that card (`within(...)`), not
  anywhere on the page.
- **R10. SHOULD-FIX (D16, rule 13).** These fix sites cite `DATA-55` without the cases; add
  `(QA LH-DATA-15-06, TC-1731)`: `lakehouse-auth/src/openfga.rs` (the two new sites),
  `routes/catalog_restore.rs` (the four changed sites), `lakehouse-store/src/uploads.rs`
  (the type and the new functions: one citation on the section's first item is enough, say
  "and the functions below"), `src/services/contracts/assets.ts` (the three types),
  `src/components/patterns/confirm-action-dialog.tsx`.

**Not for the developer.**

- **R4.** `POST /api/pipelines/runs/{runId}/retry` re-executes any run that is not an
  authored pipeline's, so a person with `pipeline:write` can re-run an old file load with
  its old table name, around `ensure_table_free` and around rule 5. The gap is older than
  this branch; D11 makes it matter. Read, not run. It gets its own backlog row (the
  planner files it), and the feature page's limits get one line in commit D:
  `docs/core/features/asset-page-depth.md`, at the end of "Limits to tell a customer":
  "- Re-running an old file load from the Pipelines page is not stopped by a drop: it can
  load into a dropped table's name during the 7 days, after which that dropped table can
  no longer be restored. To be closed in its own item. *(proposed, not yet signed)*".
- **R8.** The handoff's reason for the `route_auth` count is wrong (the loops add no test;
  the file has 94 tests of its own, one new, plus the `tls_proxy` test), the number 95 is
  right. No change. The testcontainer leftovers the handoff names are the planner's to
  remove.
- **Section 4 is corrected by this entry:** the race of a load and a drop is not "the same
  instant" and is closed by R3; "no data is lost" holds only with R2 and R3 in.

The developer's handoff for this round, appended below this entry: the merge commit, the
four commits, every command with its counts (`cargo fmt --check`; `cargo clippy -p
lakehouse-api --all-targets -- -D warnings`, the same for `lakehouse-auth` and
`lakehouse-store`; `cargo test -p lakehouse-auth --test openfga`; `cargo test -p
lakehouse-store --test uploads`; `cargo test -p lakehouse-api --test catalog_restore`;
`--test upload_routes`; `--test sec11_guard`; `--test route_auth`; `bun run typecheck`;
`bun run lint`; `bun test src/features/catalog`; `bun run test` once), each new test by
name, each case of R5 or R3 that could not be tested with the reason, and how the client's
bound is set in the R1 test.

### Round 1, developer's handoff

Developer agent, 2026-11-13. Branch `fix/qa-round-data-p4`, pushed after each commit.
`main` was merged first (`b9bb4176`, "Merge remote-tracking branch 'origin/main' into
fix/qa-round-data-p4"); it merged cleanly, `rust/migrations` still holds exactly one
`0141`, and `cargo fmt --check` plus `cargo clippy -p lakehouse-api --all-targets --
-D warnings` were clean before the push. Every commit below is on top of it, in the
task order, and no rebase, force-push or `--ours`/`--theirs` was used.

#### Commits

| Commit | Title |
| --- | --- |
| `b9bb4176` | Merge remote-tracking branch 'origin/main' into fix/qa-round-data-p4 |
| `115c0e14` | fix(review): the drop keeps its record when the outcome is not known (R1, R2, R7) |
| `dc470d7f` | fix(review): a load and a drop of one table cannot both pass (R3) |
| `97d0bf3b` | test(review): the refusals and failure paths of the drop that had no test (R5) |
| `abd51144` | docs(review): comments say what the code does, and cite the cases (R6, R9, R10) |
| `<this commit>` | docs(plan): handoff for review round 1 of data-p4 |

#### Tests added or rewritten, with the failure seen before the fix

The line numbers are the ones in the run that failed, before the fix that made the
test pass.

**Commit A (`115c0e14`)**

- `a_catalog_that_does_not_answer_within_the_clients_bound_keeps_the_record_and_says_the_outcome_is_not_known`
  (R1). Before: `left: 200 right: 503`, the body
  `{"ok":true,"table":"bronze.orders","rowCount":3,…}`, after **20.83 s** — the
  client waited out the catalog's 20-second delay because the drop's `DELETE` had
  no bound of its own.
- `a_catalog_that_answers_500_and_lists_the_table_as_deleted_keeps_the_record_and_answers_a_drop`
  (R2 case 1). Before: `left: 503 right: 200`, `{"error":"The catalog did not
  answer, so it is not known…"}`.
- `a_catalog_that_answers_500_and_a_table_that_still_exists_removes_the_record_and_says_nothing_was_dropped`
  (R2 case 2). Before: `left: "The catalog did not answer, so it is not known
  whether the table was dropped…" right: "The catalog did not answer and the table
  was not dropped. Try again shortly."`.
- `a_catalog_that_answers_500_and_a_lookup_that_fails_keeps_the_record_and_says_the_outcome_is_not_known`
  (R2 case 3). Before: `an unknown outcome keeps the name closed` — the record had
  been removed.
- `a_stale_record_of_a_table_that_exists_is_removed_by_the_next_status_read` (R2
  healing). Before: `a table that is there is not dropped:
  {"droppable":false,"reason":"This table is already dropped…"}`, `left: false
  right: true`.
- `a_stale_record_of_a_table_that_exists_is_removed_by_the_next_load_and_the_load_goes_through`
  (R2 healing, load side; `tests/upload_routes.rs`). Before: `left: 409 right:
  200`, `{"error":"That table was dropped and can still be restored…"}`.
- `an_undrop_that_does_not_answer_and_a_table_that_exists_clears_the_record_and_answers_a_restore`
  (R2 mirror case). Before: `left: 503 right: 200`, `{"error":"the catalog could
  not be asked about this table; try again shortly"}`.
- `an_undrop_that_does_not_answer_and_a_table_that_is_still_gone_keeps_the_record_and_writes_a_failed_entry`
  (R2 mirror case that did not happen). It passed before the fix, on purpose: it
  pins today's keep-the-record branch, which the review keeps ("otherwise leave the
  record, write `failed`, answer as today"). It guards that branch, it does not
  change it.
- `a_registry_name_that_is_not_a_plain_identifier_is_refused_before_anything_is_written`
  (R7). Before: `left: 503 right: 409`, `{"error":"The catalog could not be
  reached, so nothing was dropped. Try again shortly."}` — the refusal came from
  the client, after the audit entry and the record.
- Rewritten, not added: `a_catalog_that_does_not_answer_the_drop_leaves_no_record_and_no_upstream_text`
  is gone. It pinned the planner's wrong decision (an unknown outcome removing the
  record); its three replacements are the first three tests above. The commit body
  says so in those words.

**Commit B (`dc470d7f`)**

- `a_drop_whose_upload_starts_loading_between_its_checks_is_a_409_and_leaves_no_record`
  (R3, drop side). Before: `left: 200 right: 409` and a `DELETE` sent
  (`{"ok":true,…}`), with the load marked `ingesting` while the drop waited on the
  keep write.
- `a_load_whose_table_is_dropped_between_its_checks_is_409_and_settles_its_claim`
  (R3, load side; `tests/upload_routes.rs`). Before: `left: 200 right: 409`, body
  `{"upload":{…,"status":"ingesting",…,"runId":"run-2"},"runId":"run-2"}` — the
  load launched over the drop's record.

**Commit C (`97d0bf3b`)** — tests only, of paths that had none:

- `the_tenant_gate_refuses_the_drop_and_its_two_reads_without_reaching_the_catalog`
  (`403` with the gate's reason on the write, `200 {supported:false}` on the status
  read, `{supported:false, tables:[]}` on the list, no `DELETE`).
- `an_unknown_asset_id_is_a_404_on_the_drop_status_and_on_its_post`.
- `a_drop_whose_warehouse_cannot_be_looked_up_is_503_and_asks_the_catalog_nothing`
  (a `500` from the warehouse list and an empty list, both `503` with D5's
  sentence, no `DELETE`).
- `a_drop_whose_record_cannot_be_written_answers_the_database_error_and_asks_the_catalog_nothing`
  (a `CHECK` constraint on `upload_table_drop` in this test's own database; the
  store's `database error`, no `DELETE`, `allowed` then `failed`). The `failed`
  ending is R7's behaviour, so this one would have failed before commit A (one
  entry only); it is listed here because the review asked for the case.
- `two_drops_of_one_table_at_the_same_moment_are_one_drop_and_one_conflict`
  (`tokio::join!`: exactly one `200`, one `409`, one `DELETE`).
- `a_drop_whose_rows_cannot_be_counted_still_drops_with_a_null_row_count` (`null`
  in the answer, in the record and in both audit entries).
- `the_dropped_list_of_an_unrestricted_caller_in_no_tenant_is_empty`.
- `the_dropped_list_stops_at_one_hundred_rows`.
- `the_drop_status_says_null_when_the_rows_cannot_be_counted_and_never_zero`: its
  stand-in changed (the count query answers `500`); the assertions did not.

**Commit D (`abd51144`)** — console: `dropped-tables.test.tsx` gains "says one
row in the singular, as the dialog does", the test that pinned `expiresAt` equal to
now is renamed "says less than a day is left for a row at its end", and
`asset-detail-tabs.test.tsx`'s Drop-table case now finds the button with
`within(card)`. No assertion was weakened.

#### How the client's bound is set in the R1 test

The smaller change of the two the review allowed: **not** a `#[cfg(test)]`-settable
constant (an integration test links the library built without `cfg(test)`, so it
could never set one), but the bound taken by the client: `DROP_TABLE_TIMEOUT`
(fifteen seconds) is the default, and `LakekeeperAdminClient::with_drop_timeout`
takes it as an argument. The test rebuilds the admin client with one second in
`drop_app_with(..., |state| …)`, the harness hook that hands the built `AppState` to
the test before the router takes it. A deployment never calls the builder.

#### Commands run, with their real counts

Merge step (`b9bb4176`): `git merge origin/main` (clean); `ls rust/migrations|tail -3`
→ `0139_alert_rule_delivery.sql`, `0140_pipeline_inputs_and_steps.sql`,
`0141_upload_table_drop.sql`; `cargo fmt --check` clean; `cargo clippy -p
lakehouse-api --all-targets -- -D warnings` clean.

Commit A:
- `cargo fmt --check` clean; `cargo clippy -p lakehouse-api -p lakehouse-auth
  --all-targets -- -D warnings` clean.
- `cargo test -p lakehouse-auth --test openfga` — 18 passed, 0 failed.
- `cargo test -p lakehouse-store --test uploads` — 69 passed, 0 failed.
- `cargo test -p lakehouse-api --test catalog_restore` — 40 passed, 0 failed.
- `cargo test -p lakehouse-api --test upload_routes` — 180 passed, 0 failed.
- `cargo test -p lakehouse-api --test sec11_guard` — 4 passed, 0 failed.
- `cargo test -p lakehouse-api --test route_auth` — 95 passed, 0 failed.

Commit B:
- `cargo fmt --check` clean; `cargo clippy -p lakehouse-api --all-targets -- -D
  warnings` clean.
- `catalog_restore` 41 passed; `upload_routes` 180 passed; `sec11_guard` 4 passed;
  `route_auth` 95 passed; 0 failed in each.

Commit C:
- `cargo fmt --check` clean; `cargo clippy -p lakehouse-api --all-targets -- -D
  warnings` clean.
- `catalog_restore` 49 passed; `upload_routes` 180 passed; `sec11_guard` 4 passed;
  `route_auth` 95 passed; `uploads` (store) 69 passed; `openfga` (auth) 18 passed;
  0 failed in each.

Commit D:
- `python3 ops/lint/check_bare_iceberg_count.py` → "R11 guard: no bare count()
  against a Bronze Iceberg table found."
- `python3 ops/lint/check_intra_package_imports.py` → "OK: all intra-package
  dispar_orchestrate imports resolve".
- `bun install --frozen-lockfile` → 779 installs across 753 packages, no changes.
- `bun run typecheck` clean.
- `bun run lint` → 0 errors, 11 warnings (all pre-existing, none in a file this
  round touched).
- `bun test src/features/catalog` → 402 pass, 0 fail, 1275 `expect()` calls, 16
  files.
- `cargo fmt --check` clean; `cargo clippy -p lakehouse-api -p lakehouse-auth -p
  lakehouse-store --all-targets -- -D warnings` clean (the migration's header is
  embedded at compile time).
- `catalog_restore` 49 passed; `upload_routes` 180 passed; `sec11_guard` 4 passed;
  `route_auth` 95 passed; `uploads` 69 passed; `openfga` 18 passed.

Before the handoff, on this final tree:
- `bun run test` → **4235 pass, 1 skip, 0 fail**, 13 046 `expect()` calls, 390
  files.
- `python3 ops/lint/check_bare_iceberg_count.py` → "R11 guard: no bare count()
  against a Bronze Iceberg table found."
- `python3 ops/lint/check_intra_package_imports.py` → "OK: all intra-package
  dispar_orchestrate imports resolve".
- `git status` clean before this handoff commit.

The bare-count lint read a comment of mine (`catalog_restore.rs`, "a registry row
without a count (review R5)") as a bare `count(` call and failed CI's "Repo lints"
on `97d0bf3b`; the comment now says "a registry row that answers without a row
count; review R5". The lint, its allowlist and every assertion are unchanged, and
the lint passes on every commit from `abd51144` on (the earlier commits keep their
wording; CI runs the job on a commit, and the branch's head is what it lints).

#### Cases of R3 or R5 that could not be tested

None. Both R3 cases are covered through the routes, with a stand-in the route
cannot skip: the load side holds the `DESCRIBE` that `plan_columns` sends between
`ensure_table_free` and `claim_load` until the test has written the record, and the
drop side holds the keep write until the test has marked the load `ingesting`. All
eight rows of R5 are covered; the record-insert failure uses a test-only `CHECK`
constraint, no product code.

R1's `tokio::spawn` itself (part 2 and part 3) has **no test that provokes a route
deadline or a disconnect**: the harness calls the router with `oneshot`, and
provoking the 60-second deadline would mean a test that waits a minute and then
asserts on a task it cannot observe. What is covered is the client's own bound (the
test above), and the existing suite proves the spawned task still writes the same
records and entries.

#### Not verified

- CI on the pull request. Not mine to run, and the run on `97d0bf3b` failed the
  repo lints for the comment above (fixed in `abd51144`, lints re-run and quoted).
- Full workspace `cargo test` and `cargo clippy --workspace`: by the task's rule,
  only the touched crates and test files were built and run.
- `docker compose --profile '*' config` and the Dagster suite: nothing in this round
  touches `docker-compose.yml`, `dagster/` or `ops/` code (only two `ops/lint`
  scripts were run).
- The G9 gate (T8) against the real catalog: unchanged by this round, not re-run.

#### MISMATCH

None against the code: every finding matched what the code does.

Two deviations from the commit grouping, both mine to report:

- **R3's product code landed in commit A**, not B: it was written in the same pass
  as R2's (both touch the same two functions), so `115c0e14` carries the two
  second checks and `dc470d7f` carries only their tests. No history was rewritten
  to split it — a rebase or force-push is not allowed on this branch.
- The restore's second mirror test passes before the fix as well; it pins the
  branch the review kept, as written above.

### Round 2 (planner, 2026-11-13, at `f3c0875e`)

A second independent reading of round 1's four commits, then the planner's rulings. The Rust
code was read, not built; CI on the pull request (#501) is the run. Console, run in a clean
checkout: `bun run typecheck` clean; `bun run lint` 0 errors, 11 warnings; `bun run test`
4235 pass, 1 skip, 0 fail across 390 files; both repo lints pass. No existing assertion was
weakened; the handoff's counts match the files.

| Round 1 | Closed? |
| --- | --- |
| R1 (cancellation) | Partly: the 15-second bound is right and production uses it; the spawned stretch is right for drop and restore; S2 and S4 below |
| R2 (unknown outcome) | No: the three arms are as ruled, but **the healing rule the planner wrote is unsafe** (S1) |
| R3 (load and drop) | No: both second checks exist, and healing defeats the load's (S1) |
| R5 (missing tests) | Yes; S6 tightens some |
| R6, R7, R9 | Yes; S7 for comments commit A made untrue |
| R10 | One site left (S7) |

Two BLOCKERs. S1 is a hole in round 1's ruling, not a deviation by the developer.

**Commit E, `fix(review): a record is healed only when its drop cannot be in flight (S1, S2, S3)`**

- **S1. BLOCKER: healing removes the record of a drop that is in flight.** The drop writes
  its record first and sends the catalog's `DELETE` afterwards. From the insert until the
  catalog has applied the `DELETE` the table exists and the record is true. Round 1's rule
  ("a record for a table that exists is stale: remove it") lets anything that reads the
  record in that stretch remove it: a load (both checks pass again, R3 is open), a second
  `POST …/drop` (it ends with the table dropped and no record), or just opening the table's
  Activity tab (`GET …/drop`). The tests did not see it because none of their stand-ins says
  "the table exists" during a drop. **The rule is replaced:**
  1. `lakehouse-store/src/uploads.rs`: a new function beside `forget_table_drop`, for
     healing only, e.g. `heal_table_drop(pool, table, dropped_at, grace_seconds) ->
     Result<bool, StoreError>`: `DELETE FROM upload_table_drop WHERE bronze_table = $1 AND
     dropped_at = $2 AND dropped_at <= now() - ($3::bigint * interval '1 second')`, values
     bound, returning whether a row went. `$2` is the `dropped_at` of the record just read,
     so a newer record is never removed. A store test: a young record is not removed; an old
     one is; a record replaced since the read is not.
  2. `catalog_drop.rs`: a named constant `HEAL_GRACE` of **120 seconds**, with a doc that
     says it must exceed the longest stretch a drop can be in flight (the `DELETE`'s 15
     seconds, the outcome checks' two 10 seconds, and margin). In `dropped_now`, when the
     table exists: call the new function with the record's own `dropped_at`; a row went →
     one `tracing::warn!`, "not dropped"; no row went (the record is young, or was replaced)
     → **the record stands** ("dropped").
  3. `catalog_restore.rs`, where `POST …/restore` finds "the table exists" and clears the
     record before answering 409: use the same guarded removal. The restore's own success
     path (the undrop worked, or the mirror case found the table back) keeps the plain
     `forget_table_drop`: there the task knows its outcome.
  4. `Drop`'s own removals for a definite refusal (a 4xx, the load found by the second
     check) stay plain: the task knows its outcome.
  5. **Round 1's second arm is withdrawn** (the planner's ruling again): after a `Transport`
     or `Server` from the `DELETE`, "not listed as deleted and the table exists" does NOT
     remove the record any more, because a `DELETE` whose client gave up may still be applied
     by the catalog. It goes to the third arm: keep the record, `failed` with "the outcome is
     not known", `503` with D5's sentence. If the table is still there after the grace, the
     next read heals the record. The sentence round 1 introduced for that arm, `The catalog
     did not answer and the table was not dropped. Try again shortly.`, is removed with its
     constant; its test is rewritten to expect the third arm (the commit body says why: the
     planner withdrew the arm).
  6. Tests, each with the Iceberg stand-in saying what a real catalog says before the
     `DELETE` lands, "the table exists" (`mount_existing_iceberg_table` or the file's own
     helper): (i) two `POST …/drop` at once: one `200`, one `409` with the already-dropped
     sentence, one `DELETE`, the record PRESENT afterwards; (ii) the load-side test of R3
     with the catalog reachable and the table existing: T6's `409`, the record still there,
     nothing launched; (iii) `GET …/drop` with a fresh record and an existing table:
     `droppable: false`, record kept. The two existing healing tests backdate their record
     (`UPDATE upload_table_drop SET dropped_at = now() - interval '10 minutes'`), with a
     comment saying why. Each of (i), (ii), (iii) must be seen to FAIL before the fix.
- **S2. BLOCKER: a catalog call inside the spawned task has no bound.** In
  `settle_unknown_outcome`, `table_exists` is awaited bare; the Iceberg client has no request
  timeout and the task is no longer cut by the route's deadline, so a hanging catalog leaves
  `allowed` with no ending (R1's own symptom). Wrap it in `tokio::time::timeout(OUTCOME_CHECK_TIMEOUT, …)`;
  only a definite answer counts, everything else keeps the record. The same for the
  restore: `Restore::run` awaits `undrop_table` with no bound inside its task; wrap that call
  in `tokio::time::timeout` (a named constant, 15 seconds, as the drop's) and treat an
  elapsed bound as `OpenfgaError::Transport`, which goes to the mirror case. Every `.await`
  on the catalog inside either task has a bound after this; say so in each task's doc.
- **S3. SHOULD-FIX: "listed as deleted" is taken as proof.** After a `Transport`/`Server`,
  `Found` writes `executed` without asking whether the table is gone; an older deleted copy
  of the same name would make a failed `DELETE` read as a drop. On `Found`, ask the bounded
  `table_exists`: only "the table is not there" is "the drop happened"; `Found` and the
  table exists is "the answers do not agree": the third arm. One test: 500, listed as
  deleted, the table exists → `503`, D5's sentence, record kept, `allowed` then `failed`.

After commit E the unknown-outcome rule is, in full: **deleted AND gone → `executed`, 200,
record kept; anything else → record kept, `failed` ("the outcome is not known"), 503.**
Nothing but a definite 4xx, the second load check, or healing after the grace removes a
record.

**Commit F, `test(review): the spawn and the fail-closed arms are tested (S4, S5, S6)`**

- **S4.** Nothing tests R1's spawn: with `job.run().await` in its place every test passes.
  One test per route: the catalog stand-in answers the `DELETE` (the undrop) after 1.5
  seconds; the `POST` runs in a spawned test task; poll until the stand-in has received the
  call; `abort()` the test task (the handler is dropped mid-call); then poll up to five
  seconds for the audit outcomes `allowed` then `executed` and assert the record's state
  (present after the drop, cleared after the restore). To see it bite, replace the spawn by
  a plain `.await` once, see the trail stay at `allowed`, and put it back; say so in the
  handoff.
- **S5.** The arm "not listed as deleted, and the table is absent or cannot be asked" has no
  test: `DELETE` answers 500, nothing is listed as deleted, the table is missing → `503`
  with D5's sentence, record kept, `allowed` then `failed` with "the outcome is not known".
  A second test for a deleted list that never ends (a next-page token on every page), same
  expectations.
- **S6.** Tighten, without loosening anything: the two-drops test asserts the refused
  answer's exact sentence, that exactly one record is left, and the audit outcomes; the
  tenant-gate, unknown-id and warehouse tests assert that no `catalog.drop` entry and no
  record were written; the test that asserts the second entry is `failed` also asserts the
  first is `allowed`; the list-stops-at-100 test asserts WHICH row is left out (the oldest);
  the no-answer-within-the-bound test asserts the body holds no upstream text.

**Commit G, `docs(review): comments and names after round 2 (S7, S9)`**

- **S7 (rule 1, R10).** `dropped_now`'s `# Errors`: only the read returns an error (a failed
  removal is logged). `Restore::run`'s `# Errors`: the record's clearing does not return an
  error; the 503 of the first audit entry is missing. `OUTCOME_CHECK_TIMEOUT`'s doc: true
  after S2; check it. `drop_table`'s `# Errors`: list the 503s that exist after commit E
  (including "dropping is not set up"). `status` and `list`: a database failure is the
  store's 500 "database error", not a 503; 503 is "no pool configured". `openfga.rs`,
  `drop_table`'s doc: add `(QA LH-DATA-15-06, TC-1731)`.
- **S9.** Rename the struct `Drop` to `DropJob` and `Restore` to `RestoreJob` (a struct named
  `Drop` shadows the prelude's trait in that module). The "settle the claim as failed" block
  in `routes/uploads.rs` is written three times: make it one private helper and call it from
  the three places (rule 4). The two `INTERRUPTED` sentences stay as they are; the planner
  lists them below.
- Feature page, `docs/core/features/asset-page-depth.md`, at the end of "Limits to tell a
  customer", one line: "- If the catalog does not answer a drop, the table stays marked as
  dropped until the product can tell: for up to two minutes a table that was not dropped
  can read as dropped, and a load into its name is refused. A drop the catalog applies later
  than that is not recorded. *(proposed, not yet signed)*".

**For the planner, not the developer.**

- **S8.** The handoff of round 1 lists the load-side test of R3 under commit B; it is in
  commit A (`dc470d7f` adds one test, not two). Nothing claimed as tested is untested. The
  developer corrects the sentence in this round's handoff.
- The two sentences shown when the spawned work itself breaks (a join error) are new text
  on screen that D5 does not list. They are fixed and ours; they are added to D5 by this
  entry as *(proposed, not yet signed)*: the drop's and the restore's `INTERRUPTED`
  constants, as written in the code.
- Not established by reading, and only a real catalog shows: that the catalog answers "not
  found" to the load of a soft-deleted table (the healing rule and the restore rest on it;
  the G9 gate step of T8 is the proof); whether the catalog applies a `DELETE` whose client
  gave up; whether anything between the API and the catalog answers 408 or 429 itself (every
  4xx is a definite refusal here). A spawned task is not awaited by a graceful shutdown once
  its request has gone: accepted, the record and the `allowed` entry then stand, and the
  healing rule or a restore settles them.

The developer's handoff for this round, appended below this entry: the merge commit (if
`main` moved) and the three commits; every command with its counts (`cargo fmt --check`;
clippy for `lakehouse-api`, `lakehouse-store`, `lakehouse-auth`, all targets, `-D warnings`;
`cargo test -p lakehouse-store --test uploads`; `cargo test -p lakehouse-auth --test
openfga`; `cargo test -p lakehouse-api --test catalog_restore`; `--test upload_routes`;
`--test sec11_guard`; `--test route_auth`; both repo lints; `bun run typecheck`; `bun run
lint`; `bun test src/features/catalog`); each new test by name with the failure line seen
before its fix; the spawn's bite check (S4); S8's corrected sentence; anything *not
verified*; every MISMATCH.

### Round 2, developer's handoff

Developer agent, 2026-11-13. Branch `fix/qa-round-data-p4`, pushed after each commit.
`main` was merged twice (it moved between them), and no rebase, force-push or
`--ours`/`--theirs` was used.

#### Merges

| Merge | What |
| --- | --- |
| `639cc9a5` | Merge remote-tracking branch 'origin/main' into fix/qa-round-data-p4 — clean, no conflict; `ls rust/migrations \| tail -4` ended at `0141_upload_table_drop.sql`, so **the migration keeps the number `0141`**: `main` added no `0141` of its own |
| `f43af8b3` | Merge remote-tracking branch 'origin/main' into fix/qa-round-data-p4 — `CHANGELOG.md` conflicted, the only file; both sides' entries are kept, main's first, then this branch's; no other file conflicted; `ls rust/migrations \| tail -3` → `0139`, `0140`, `0141` (still exactly one `0141`) |

#### Commits

| Commit | Title |
| --- | --- |
| `ef3e5185` | fix(review): a record is healed only when its drop cannot be in flight (S1, S2, S3) |
| `686ee8a0` | test(review): the spawn and the fail-closed arms are tested (S4, S5, S6) |
| `63a7dcb5` | docs(review): comments and names after round 2 (S7, S9) |
| `f43af8b3` | Merge remote-tracking branch 'origin/main' into fix/qa-round-data-p4 |
| `<this commit>` | docs(plan): handoff for review round 2 of data-p4 |

The migration's number is **`0141`**, unchanged: `rust/migrations/0141_upload_table_drop.sql`.

#### What each commit does

**Commit E.** `uploads::heal_table_drop(pool, table, dropped_at, grace_seconds)` removes a
record only when it is older than the grace and still carries the `dropped_at` the caller
read, both values bound in one `DELETE`. `HEAL_GRACE` is 120 seconds (`pub(super)` in
`catalog_drop.rs`, with the doc the plan asks for: the `DELETE`'s fifteen, the two outcome
checks' ten each, and margin). `dropped_now` uses it when the table exists; the restore's
`Assessment::Exists` path uses it through one `clear_stale_record` helper; the drop's own
removals for a definite refusal stay plain `forget_table_drop`. Round 1's second arm is gone
with `NOT_DROPPED`, and its test is rewritten to the third arm. `settle_unknown_outcome` now
asks the bounded `table_exists` after `Found` (S3) and, for every other answer, keeps the
record and answers D5's sentence. `bounded_table_exists` bounds the presence check with
`OUTCOME_CHECK_TIMEOUT` (S2), and `RestoreJob::run` bounds `undrop_table` with
`UNDROP_TIMEOUT` (fifteen seconds), an elapsed bound becoming `OpenfgaError::Transport` and
so the mirror case. Both tasks' docs say every catalog `.await` in them is bounded.

**Commit F.** Tests only: S4's two abort tests, S5's two tests, S6's tightenings (all added
or more exact assertions; nothing loosened).

**Commit G.** S7's doc corrections, S9's renames (`DropJob`, `RestoreJob`) and the one
`settle_claim_failed` helper in `routes/uploads.rs` (three call sites, behaviour unchanged),
and the feature page's limit line with its *(proposed, not yet signed)*.

#### Tests added or rewritten, with the failure seen before the fix

**Commit E**

- `healing_a_drop_removes_only_a_record_older_than_the_grace_and_unchanged_since_it_was_read`
  (`tests/uploads.rs`, store). Before: `error[E0432]: unresolved import
  lakehouse_store::uploads::heal_table_drop` at `crates/lakehouse-store/tests/uploads.rs:39:83`
  — the function did not exist yet. A young record stays, an old one goes, and a record
  replaced since the read stays (a newer `dropped_at`, still past the grace, so only the
  equality guard can keep it).
- `two_drops_of_a_table_that_still_exists_are_one_drop_one_conflict_and_keep_the_record`
  (S1 (i)). Before: `catalog_restore.rs:2568`, `left: 200`, the body
  `{"ok":true,"table":"bronze.orders","rowCount":3,…}` — the second drop healed the record
  and dropped too. (Seen in the test's first form at `catalog_restore.rs:2290` and re-checked
  in its committed form at HEAD with the two product files of `ef3e5185^` checked out.)
- `a_load_whose_table_is_dropped_between_its_checks_keeps_the_record_when_the_table_still_exists`
  (S1 (ii), `tests/upload_routes.rs`). Before: `upload_routes.rs:3233`, `left: 200`, the body
  `{"upload":{…,"status":"ingesting",…,"runId":"run-2"},"runId":"run-2"}` — the load launched
  over a record that was still true.
- `a_fresh_record_of_a_table_that_exists_is_not_healed_by_the_status_read` (S1 (iii)).
  Before: `catalog_restore.rs:1485`, `left: true`,
  `{"droppable":true,"table":"bronze.orders","keptDays":7}`.
- `a_catalog_that_lists_a_deleted_table_that_still_exists_says_the_outcome_is_not_known`
  (S3). Before: `catalog_restore.rs:1271`, `left: 200`, `{"ok":true,…}` — `Found` was taken
  as proof.
- Rewritten, not added:
  `a_catalog_that_answers_500_and_a_table_that_still_exists_removes_the_record_and_says_nothing_was_dropped`
  becomes
  `a_catalog_that_answers_500_and_a_table_that_still_exists_keeps_the_record_and_says_the_outcome_is_not_known`
  (S1 item 5). Before the rewrite: `catalog_restore.rs:1221`, `left: "The catalog did not
  answer and the table was not dropped. Try again shortly."`.
- Backdated, assertions unchanged: `a_stale_record_of_a_table_that_exists_is_removed_by_the_next_status_read`
  and `a_stale_record_of_a_table_that_exists_is_removed_by_the_next_load_and_the_load_goes_through`
  each gain `UPDATE upload_table_drop SET dropped_at = now() - interval '10 minutes'`, with a
  comment saying why (healing waits out `HEAL_GRACE`).
- The first arm's test,
  `a_catalog_that_answers_500_and_lists_the_table_as_deleted_keeps_the_record_and_answers_a_drop`,
  gains `mount_missing_table`: S3 makes "the table is gone" part of its rule, and without the
  stand-in the presence check could not answer. It failed at `catalog_restore.rs:1147`,
  `left: 503 right: 200`, until the stand-in was mounted.

**Commit F**

- `a_drop_whose_caller_goes_away_mid_call_still_gets_its_ending` (S4). It passes against the
  spawn commit A shipped; its bite is the check below.
- `a_restore_whose_caller_goes_away_mid_call_still_gets_its_ending` (S4, restore). Same.
- `a_catalog_that_answers_500_with_nothing_listed_and_no_table_keeps_the_record_and_says_the_outcome_is_not_known`
  (S5). It pins an arm commit E created; it passes at once and was not seen to fail, because
  before E this case already ended in the same third arm.
- `a_deleted_list_that_never_ends_keeps_the_record_and_says_the_outcome_is_not_known` (S5).
  Same: a `next-page-token` on every page, twenty pages, `CouldNotTell`.
- S6's tightenings are added or more exact assertions, so none of them fails first: the two
  two-drops tests (exact refused sentence, exactly one record, first entry `allowed`, exactly
  one `executed`, no `failed`), the tenant-gate, unknown-id and warehouse tests
  (`assert_nothing_written`: no `catalog.drop` entry, no record), the lookup-fails test's
  first entry, the list's left-out row (`t_000`, the oldest, absent; `t_001` and `t_100`
  present), and the no-answer test's "no upstream text in the body".

#### The spawn's bite check (S4)

Done once, locally, never committed: in `catalog_drop.rs`'s `drop_table`, `let handle =
tokio::spawn(async move { job.run().await });` was replaced by `std::future::ready(Ok(job.run().await))`,
so the audited stretch ran inside the handler. `cargo test -p lakehouse-api --test
catalog_restore a_drop_whose_caller_goes_away` then failed at `catalog_restore.rs:1468` with
`left: ["allowed"]` — the trail stays at the entry the handler wrote before it was dropped,
never reaching `executed`. The spawn was put back; `git diff` on that file is empty against
`ef3e5185`.

#### S8's corrected sentence

The round 1 handoff lists the load-side test of R3 under commit B. It is in **commit A**
(`115c0e14`), together with the route change it pins:
`a_load_whose_table_is_dropped_between_its_checks_is_409_and_settles_its_claim` is in
`tests/upload_routes.rs` at `115c0e14`. Commit B (`dc470d7f`) adds **one** test, not two
(`a_drop_whose_upload_starts_loading_between_its_checks_is_a_409_and_leaves_no_record`, in
`tests/catalog_restore.rs`). Nothing claimed as tested is untested.

#### Commands run, with their real counts

Merge `639cc9a5`: `git merge origin/main` clean; `ls rust/migrations | tail -4` → `0138`,
`0139`, `0140`, `0141`; `cargo fmt --check` clean; `cargo clippy -p lakehouse-api
--all-targets -- -D warnings` clean.

Commit E (`ef3e5185`):
- `cargo fmt --check` clean (after one `cargo fmt` for an import wrap); `cargo clippy -p
  lakehouse-store --all-targets -- -D warnings` and `-p lakehouse-api` clean.
- `cargo test -p lakehouse-store --test uploads` — **70 passed, 0 failed** (69 before this
  commit).
- `cargo test -p lakehouse-auth --test openfga` — **18 passed, 0 failed**.
- `cargo test -p lakehouse-api --test catalog_restore` — **52 passed, 0 failed** (51 before
  this commit's tests, plus the rewritten arm).
- `cargo test -p lakehouse-api --test upload_routes` — **181 passed, 0 failed** (180 before).
- `cargo test -p lakehouse-api --test sec11_guard` — **4 passed, 0 failed**.
- `cargo test -p lakehouse-api --test route_auth` — **96 passed, 0 failed** (95 before: one
  test of this branch, plus `tls_proxy`; main's `pipeline_preview` added the ninety-sixth).
- `python3 ops/lint/check_bare_iceberg_count.py` → "R11 guard: no bare count() against a
  Bronze Iceberg table found."; `check_intra_package_imports.py` → "OK: all intra-package
  dispar_orchestrate imports resolve".

Commit F (`686ee8a0`):
- `cargo fmt --check` clean; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`
  clean.
- `catalog_restore` **56 passed, 0 failed**; `upload_routes` **181 passed**; `sec11_guard`
  **4 passed**; `route_auth` **96 passed**; `uploads` (store) **70 passed**; `openfga` (auth)
  **18 passed**; 0 failed in each; both repo lints pass.

Commit G (`63a7dcb5`):
- `cargo fmt --check` clean; `cargo clippy` for `lakehouse-auth`, `lakehouse-store` and
  `lakehouse-api`, all targets, `-D warnings`: clean.
- `catalog_restore` **56 passed**; `upload_routes` **181 passed**; `sec11_guard` **4
  passed**; `route_auth` **96 passed**; `uploads` (store) **70 passed**; `openfga` (auth)
  **18 passed**; 0 failed in each; both repo lints pass.
- `bun install --frozen-lockfile` → 779 installs across 753 packages, no changes; `bun run
  typecheck` clean; `bun run lint` → **0 errors, 11 warnings** (the same eleven as round 1,
  none in a file this round touched); `bun test src/features/catalog` → **402 pass, 0 fail,
  1,275 expect() calls, 16 files**.

Merge `f43af8b3`, after resolving `CHANGELOG.md`:
- `ls rust/migrations | tail -3` → `0139`, `0140`, `0141`; `cargo fmt --check` clean; `cargo
  clippy -p lakehouse-api --all-targets -- -D warnings` clean.
- `catalog_restore` **56 passed, 0 failed**; `upload_routes` **181 passed**; `sec11_guard`
  **4 passed**; `route_auth` **96 passed**; both repo lints pass.
- `bun run typecheck` clean; `bun run lint` → 0 errors, 11 warnings; `bun test
  src/features/catalog` → **402 pass, 0 fail**.

`docker ps -a` before the first test run showed ten containers (nine `clickhouse-server:26.8`
and one `postgres:16-alpine`) that were already there; the same ten were there after the last
run, so no container of mine was left behind and none was removed.

#### Not verified

- The restore's abort test (`a_restore_whose_caller_goes_away_mid_call_still_gets_its_ending`)
  was not given its own bite check: the plan asks for the replacement of the spawn once, and
  that was done on the drop's. The two are the same shape in the two handlers.
- The two S5 tests were not seen to fail: they pin arms commit E created, and both cases
  already ended in the third arm before E.
- `cargo test --workspace` was not run, by the plan's rule; only the scoped suites above.
- No real catalog was asked (the G9 step of T8 is the proof there): whether `Lakekeeper`
  answers "not found" to a load of a soft-deleted table, and whether it applies a `DELETE`
  whose client gave up, are as unestablished as the Round 2 entry says.
- `docker compose config` / `up` was not run: no compose file changed.

#### MISMATCH

- **MISMATCH (S1 (i)):** the two drops are not a `tokio::join!` of two calls. With the
  `DELETE` answered late (a 3-second delay on the stand-in), the first drop is held in flight
  while the second runs, which is the stretch the ruling is about and is deterministic; a
  true join can pass or fail by luck. The assertions are the ones S1 (i) and S6 ask for: one
  `200`, one `409` with `ALREADY_DROPPED`, one `DELETE`, exactly one record, the trail's first
  entry `allowed` and exactly one `executed`.
- No other mismatch: every other ruling matched the code as it was.

### Round 3 (planner, 2026-11-13, at `430a0009`)

A third independent reading, of round 2's three commits only, then the planner's rulings.
Read, not built; CI on the pull request (#501) is the run.

| Round 2 | Closed? |
| --- | --- |
| S1 (healing) | Yes: the guarded removal, the 120-second grace, the second arm gone; in all three interleavings the record stays and the second actor is refused |
| S2 (bounds) | Yes: every await on the catalog inside both tasks has a bound |
| S3, S4, S5, S8, S9 | Yes (both abort tests fail without the spawn, by reading; both S5 tests fail if their arm removed the record) |
| S6 | Partly: U4 |
| S7 | Yes for the sites named; commit E made others untrue: U5 |

The developer's MISMATCH (the two-drops test holds the first drop in flight with a delayed
`DELETE` where the ruling said two requests at once) is accepted: it is the stronger test,
because the second request provably arrives inside the stretch.

**No BLOCKER.** Five SHOULD-FIX. U1 and U2 are rare, and both end in the state this
feature must never reach (a dropped table with no record), so they are fixed before the
merge. ONE commit, `fix(review): a drop and a restore remove only their own record (U1 to U5)`.

- **U1. The stretch between the record and the `DELETE` has a step with no bound.** The
  second load check (`loading_into_now` in `DropJob::run`) settles ended loads, which can
  read the object store and wait on the database with no bound of ours. If more than the
  grace passes there, a read heals the true record and the `DELETE` still goes out. Fix:
  1. Wrap the second load check in `tokio::time::timeout` with a named constant
     (`LOAD_CHECK_TIMEOUT`, 30 seconds). An elapsed bound goes where the check's `Err` goes
     today: the record stands, `failed`, **no `DELETE`**. The sentence, for both the error
     and the timeout (use the existing error arm's sentence if it has one of its own;
     otherwise this fixed one, *proposed, not yet signed*): `The loads into this table
     could not be checked, so nothing was dropped. Try again shortly.` (`503`).
  2. The task's own removals (the definite 4xx; the load found by the second check) remove
     ONLY this attempt's record: the same guarded delete with `dropped_at` equal to the
     record this task wrote and no grace (a sibling store function with only `bronze_table
     = $1 AND dropped_at = $2`, or the existing one with a grace of 0: choose the clearer;
     do not write the SQL twice).
  3. `HEAL_GRACE`'s doc states the real sum: 30 (the load check) + 15 + 10 + 10 = 65
     seconds of bounds, and says that the database calls in the stretch have the pool's own
     limits, not ours.
  4. One test: the second load check does not return inside its bound → no `DELETE`, the
     record kept, `allowed` then `failed`. If it cannot be arranged without a hook in
     product code, do not add a hook: say so in the handoff, and test the store function
     of item 2 instead (a record with another `dropped_at` is not removed).
- **U2. The restore's removal can delete a newer drop's record.** `finish_restore` removes
  whatever record the name has when the undrop's answer arrives; if the answer is slow and a
  second person drops the table in those seconds (the old record is past its grace, so a
  read heals it and a new one is written), the new record is deleted and that drop's
  `DELETE` goes out. Fix: read the record (`dropped_table`) before the undrop is sent, carry
  its `dropped_at` in `RestoreJob`, and in `finish_restore` remove only a record with that
  `dropped_at` (item 2's function). No record was read, or the read failed: remove nothing
  and log one line; a leftover is healed by the next read. One test: the undrop answers
  late, the record's `dropped_at` is changed while it is in flight (an `UPDATE` in the
  test), and the record is still there afterwards.
- **U3. The restore's "the table exists" path has no test.** Two tests: `POST …/restore`
  with a fresh record and an existing table → `409` with the "exists again" sentence, the
  record PRESENT; the same with the record backdated ten minutes → `409`, the record gone.
- **U4 (rule 2). Two tests lost an assertion.** The rewritten second-arm test and the S3
  test no longer assert that the first audit entry is `allowed`. Add
  `assert_eq!(outcomes[0].0, "allowed");` (or the file's own form) to both.
- **U5 (rule 1). Comments commit E made untrue.** `settle_not_dropped`'s doc still describes
  the withdrawn arm ("keeps the record unless the catalog says the table is still there"):
  an unknown outcome always keeps the record. `catalog_restore.rs` module doc: the record is
  cleared for a table that exists only past the grace. The three "believed only while the
  table is absent" comments (`catalog_drop.rs`, `catalog_restore.rs`, `routes/uploads.rs`):
  a young record is believed while the table exists too. `OUTCOME_CHECK_TIMEOUT`'s doc: the
  undrop is bounded by `UNDROP_TIMEOUT` in the route, not by the client. The store test's
  comment "A newer drop of the same name" above a row with an OLDER time: "a different
  record".

Accepted and written down (no change): a stall longer than the pool's own limits inside
the stretch, and a `DELETE` the catalog applies after the client gave up and after the
grace, can still leave a dropped table without a record; the feature page's limit line of
round 2 says so. Only a real catalog shows how it treats a soft-deleted table's name; the
G9 gate step on CI is the proof of the main path.

The developer's handoff for this round, appended below this entry: the commit; every
command with its counts (`cargo fmt --check`; clippy for `lakehouse-api` and
`lakehouse-store`, all targets, `-D warnings`; `cargo test -p lakehouse-store --test
uploads`; `cargo test -p lakehouse-api --test catalog_restore`; `--test upload_routes`;
`--test sec11_guard`; `--test route_auth`; both repo lints); each new test by name with the
failure line seen before its fix; whether U1's timeout test could be arranged; anything
*not verified*; every MISMATCH.

### Round 3, developer's handoff

Developer agent, 2026-11-13. Branch `fix/qa-round-data-p4`. `main` had not moved (`origin/main`
is `9d2c27dd`, already merged by `f43af8b3`), so there is no merge commit this round. One commit:

- `6d1f49c3` — `fix(review): a drop and a restore remove only their own record (U1 to U5)`

The five findings, as built:

- **U1.** `LOAD_CHECK_TIMEOUT` (30 s) wraps the second load check in `DropJob::run`. An elapsed
  bound and a failed check both keep the record, write `failed` and answer `503` with
  `LOAD_CHECK_FAILED`; no `DELETE` goes out. The error arm had no sentence of its own (it
  returned the store's error), so both use the fixed one, as the ruling says. The two removals
  the task itself makes (a definite refusal; the load the second check finds) call
  `uploads::forget_table_drop_at(pool, table, record.dropped_at)` — the task's own record, no
  grace, values bound. `HEAL_GRACE`'s doc now gives the real sum, 30 + 15 + 10 + 10 = 65
  seconds of our bounds, and says the database calls in the stretch have the pool's limits.
- **U2.** `POST …/restore` reads `uploads::dropped_table` in the handler, before the task and
  the undrop, and carries `dropped_at` in `RestoreJob`; `finish_restore` calls the same
  `forget_table_drop_at` and so removes only that record. No record read, or a failed read:
  nothing is removed and one `tracing::warn!` says so.
- **U3.** Two tests of `POST …/restore` against a table that exists.
- **U4.** `assert_eq!(outcomes[0].0, "allowed");` in the two tests named.
- **U5.** The withdrawn arm in `settle_not_dropped`'s doc; `catalog_restore.rs`'s module doc
  (past the grace, and only the record read); the "believed only while the table is absent"
  sentences in `catalog_drop.rs` (module doc, `assess`, `dropped_now`), `catalog_restore.rs`
  (`table_exists`) and `routes/uploads.rs` (`ensure_table_free`) — and the two test docs that
  carried the same sentence (`catalog_restore.rs`, `upload_routes.rs`), which the ruling did
  not name; `OUTCOME_CHECK_TIMEOUT`'s doc now names `UNDROP_TIMEOUT` and `DROP_TABLE_TIMEOUT`
  as the undrop's and the `DELETE`'s bounds; the store test's comment now says "a different
  record".

**Tests added or changed, by name**

- `lakehouse-store`, `tests/uploads.rs`:
  `forgetting_a_drop_removes_the_record_it_read_and_no_other` (replaces
  `forgetting_a_drop_removes_it_and_says_whether_there_was_one`, which tested the function the
  other two named here replaced): the record read is removed, an absent one is not an error,
  and a record written afterwards for a second drop of the same name is NOT removed.
  `healing_a_drop_removes_only_a_record_older_than_the_grace_and_unchanged_since_it_was_read`
  changed only in the U5 comment and its last message.
- `lakehouse-api`, `tests/catalog_restore.rs`:
  `a_restore_leaves_a_record_written_while_its_undrop_was_in_flight` (U2);
  `a_restore_of_a_table_that_exists_leaves_a_fresh_record` (U3);
  `a_restore_of_a_table_that_exists_clears_a_record_past_its_grace` (U3);
  and the `outcomes[0].0 == "allowed"` assertion in
  `a_catalog_that_answers_500_and_a_table_that_still_exists_keeps_the_record_and_says_the_outcome_is_not_known`
  and `a_catalog_that_lists_a_deleted_table_that_still_exists_says_the_outcome_is_not_known`
  (U4). No existing assertion was loosened.

**Seen to fail before the fix**

U2's test, with the fix's `src/` withheld (the repo's HEAD code, only the tests applied):
`cargo test -p lakehouse-api --test catalog_restore a_restore_leaves_a_record_written_while_its_undrop_was_in_flight`
→ `1 failed`:
`panicked at crates/lakehouse-api/tests/catalog_restore.rs:2077:5: the record written while the undrop was in flight stays: the restore read another one`
— the old `finish_restore` removed the record by name, so the second drop's record went.

U3's two tests cannot be seen to fail: they pass at `362382ff` and still pass
(`2 passed`), because round 2's S1 commit already guards that removal. They are the missing
coverage U3 asks for, not a bug; see MISMATCH 1.

The store test cannot be run against the old function — it names the new one, which did not
exist — so it was not seen to fail; its new assertions are the two the ruling asks for.

**U1's timeout test: not arranged, no hook added**

It cannot be arranged without a hook in product code. Every call `settle_loading_upload` makes
is already bounded (`SETTLE_CALL_TIMEOUT`, 5 s, on the orchestrator and on the results read), so
reaching 30 s needs several `ingesting` rows against stand-ins that each stall, and the test
would take longer than 30 real seconds even when it does not flake; `tokio::time::pause` would
race the real HTTP stand-ins. Per the ruling, no hook was added; the store test above covers
item 2 instead. The timeout arm itself is read, not run.

**Commands, with their counts** (from `rust/`, foreground, `CARGO_TARGET_DIR` shared)

- `cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — clean, 0 warnings.
- `cargo clippy -p lakehouse-store --all-targets -- -D warnings` — clean, 0 warnings.
- `cargo test -p lakehouse-store --test uploads` — 70 passed, 0 failed.
- `cargo test -p lakehouse-api --test catalog_restore` — 59 passed, 0 failed (56 before, 3 new).
- `cargo test -p lakehouse-api --test upload_routes` — 181 passed, 0 failed (126.28 s).
- `cargo test -p lakehouse-api --test sec11_guard` — 4 passed, 0 failed.
- `cargo test -p lakehouse-api --test route_auth` — 96 passed, 0 failed.
- `python3 ops/lint/check_bare_iceberg_count.py` — R11 guard clean;
  `python3 ops/lint/check_intra_package_imports.py` — all imports resolve.
- `git status` before the commit: the seven edited files and nothing generated.

**Not verified**

- No workspace-wide build or test (`cargo test --workspace` was not run, per the brief); the two
  crates above and their dependencies were compiled. A grep over `rust/` finds no other use of
  the removed `forget_table_drop`, so no other crate should have seen it change — not proven by
  compiling them.
- The `LOAD_CHECK_TIMEOUT` arm is read, not run (above).
- No compose, no console, no Python, no Docker, and the G9 gate step were run; nothing in this
  commit touches them.

**MISMATCH**

1. **U3's tests pass before the fix.** The brief asks that U3's be seen to fail; they cannot,
   because `362382ff` already carries round 2's S1 guard and U3 adds coverage only. Their
   failure lines are therefore not available; the run at HEAD is `2 passed`.
2. **`forget_table_drop` is gone, not merely unused.** Once every removal is guarded (U1's two
   and U2's one) it had no caller left, and keeping an exported removal that deletes any record
   for a name is the footgun this round is about. It is replaced by `forget_table_drop_at`, and
   its store test by the one above. This is one step past the five's letter; reported here.
3. **U5's three comments carried the same sentence in two test files too.** Corrected as well
   (`tests/catalog_restore.rs`, `tests/upload_routes.rs`); the ruling named only the three
   product sites.
