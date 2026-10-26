# `DATA-35` Folders for uploaded files (plan)

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-26 |
| Spec | `docs/core/specs/data-35.md` |
| Feature page | `docs/core/features/upload-folders.md`: eight decisions, all signed 2026-10-26 |
| Base | `feat/data-phase-3-uploads` (PR #186) at `9520ead3`, because this touches the same upload code; every anchor below was read on this branch. The pull request is stacked on #186 and merges after it |
| Branch | `feat/data-35-upload-folders`, worktree `/home/hv/lakehouse-data35` |
| Migration | `0112` (free on every branch on 2026-10-26: the highest in use anywhere is `0111`; `0104`–`0109` are named in other plans) |

## 1. What exists (read, not run)

`S` = `rust/crates/lakehouse-store/src`, `A` = `rust/crates/lakehouse-api/src`.

- **Uploads table** `file_upload` (`0057`, `0058` tenant and claims, `0090`, `0091`, `0092` `loaded_with`). Store `S/uploads.rs`: `struct Upload` `:184-281`, `COLS` `:283-286`, `list(pool, tenant, limit)` `:600-610` (newest first, clamped 1..=500), `upload_in_tenants` `:650`, `mark_group_within_limits` `:896-967` (the other files of a grouped load get `loaded_with = <primary>`), claims `table_claim` `:1139`, `claim_table` `:1180` (`upload_table_claim`, keyed by table, outlives deleted uploads).
- **Routes** `A/routes/uploads.rs`: fixed sentences `:247-346`; `list` `:1240-1259` (`tenant_scope::resolve`, `LIST_LIMIT` 100 `:205-207`); `IngestRequest` `:1867-1881` and `parse_ingest_request` `:1947-2010` (hand-parsed JSON map); `claim_load` `:2368-2420`; `ingest` `:2566-2651`; `delete` `:2731-2778`; audit `:2118`, `:2143`. Router `A/routes/mod.rs` `uploads_router` `:816-883` (per-id routes behind `require_upload_in_tenants`; static paths such as `sessions` mounted before `{id}`). Policy `A/policy.rs:805-826`, all `connector:manage`; `tests/route_auth.rs:1239-1288` pins them both ways.
- **Console**: `src/features/connectors/uploads-panel.tsx` (the tab: a plain table, no search, Delete disabled with a `title` reason while loading `:148-176`); wizard Table step `upload-table-step.tsx:25-115`, parent `upload-file-page.tsx` (`load()` builds `IngestUploadInput` `:141-163`); retry `upload-run-view.tsx:69-90` with `retryInput` (`src/lib/uploads.ts:342-360`); contract `src/services/contracts/uploads.ts` (`Upload` `:146-205`, `IngestUploadInput` `:352-367`), client `src/services/clients/uploads.ts`.
- **Folder UI to reuse**: `src/lib/folder-tree.ts` (generic: `buildFolderTree`, `folderPath` "A / B", `groupBoardsByFolder` with a no-folder section first; `MAX_FOLDER_DEPTH = 4` hard-coded `:10`); `src/features/dashboards/folder-dialogs.tsx` (`ManageFoldersDialog`, `MoveBoardDialog`, both bound to `dashboardService`, one item per move).
- **Dashboard folders** live in ClickHouse without a tenant column (`lakehouse-bi`): not reusable as storage; only the pure placement rules (`lakehouse-bi/src/folders.rs` `check_placement` `:164-204`) are a model.
- **Assistant**: no tool reads uploads; no `tool_schemas.json` change.

## 2. Architecture decisions (planner)

- **AD1. Own Postgres table, tenant-scoped.** `upload_folder(id TEXT 'uf-<uuid>' PRIMARY KEY, tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE, parent_id TEXT NULL REFERENCES upload_folder(id) ON DELETE RESTRICT, name TEXT NOT NULL CHECK 1..100 characters after trim, created_by TEXT, created_at, updated_at)`, unique per `(tenant_id, COALESCE(parent_id, ''), lower(name))` (decision 6). `file_upload.folder_id TEXT NULL REFERENCES upload_folder(id) ON DELETE RESTRICT` with an index `(tenant_id, folder_id)`. `upload_table_claim.folder_id TEXT NULL REFERENCES upload_folder(id) ON DELETE SET NULL` (the table's folder for re-uploads, AD4). Migration `0112`, why-header.
- **AD2. The rules live in the store, checked against the database**, never against the console's 100-row list: depth at most 3 (decision 3), no cycle on a move, sibling names unique case-insensitively, a folder can be deleted only when it holds no file **and no folder** (decision 8; a folder whose only content is a sub-folder is not empty: delete the sub-folder first — stated on the page). Each refusal is a fixed sentence.
- **AD3. Routes**, all `connector:manage` (decision 4), tenant from `tenant_scope::resolve`, a folder of another tenant answers 404 like an upload does: `GET /api/uploads/folders` (each folder with `fileCount` and `childCount` counted on the server); `POST /api/uploads/folders` `{name, parentId?}`; `PATCH /api/uploads/folders/{id}` `{name?, parentId?}` (rename and move a folder); `DELETE /api/uploads/folders/{id}` (409 with the sentence while not empty); `POST /api/uploads/move` `{uploadIds: [...], folderId: string|null}` (all ids of the caller's tenant or nothing moves). Static paths mounted before `/api/uploads/{id}`. Every route in `POLICY_TABLE` and pinned both ways in `route_auth.rs`. Audit events `upload.folder.create|rename|move|delete` and `upload.move`.
- **AD4. Choosing a folder while uploading** (decision 2): `IngestRequest` gains an optional `folderId`; the ingest sets it on the primary and on every other file of a grouped load, and records it on the table's claim. When the Table step opens for a table that already exists, the folder is preselected from the claim (`GET` of the claim's folder through the list or a small field on the existing upload), and the person can change it. A retry resends the same folder. Moving files later never touches the claim except when every remaining upload of that table is moved together; keep it simple: the claim records the folder of the latest load.
- **AD5. Listing**: `GET /api/uploads` gains `folderId=<id>|none` and `q=<text>` (file name or table name, case-insensitive) so a folder's files and a search are not cut by the newest-100 window; without them it keeps today's behaviour. Each upload in the response carries `folderId`.
- **AD6. Console**: the tab gets a folder column on the left (a tree built with `folder-tree.ts`, depth made a parameter, "No folder" first and "All files"), the open folder's files on the right, a search box (the Connectors tab's `useTableUrlState` + `DataTableSearch` pattern) whose results show each file's folder path, multi-select with "Move to folder…", and "New folder / Rename / Delete" in the folder's menu; Delete disabled with the reason while `fileCount + childCount > 0`. The dashboard dialogs are generalised to take a service instead of copying them, or new upload-specific dialogs are written if generalising would change dashboard behaviour (the developer chooses and says why).

## 3. Tasks (one commit each, in order; scoped checks per commit)

- **T1 Migration and store.** `0112_upload_folders.sql` (AD1) and `S/upload_folders.rs` (create, rename, move with the rules of AD2, delete when empty, list with counts) plus `folder_id` on `Upload`, `COLS`, `list` filters (AD5), `move_uploads`. Tests in `tests/uploads.rs` style: depth, cycle, uniqueness, delete refused with a file and with a sub-folder, another tenant's folder not found, migration test that an existing upload keeps a null folder.
- **T2 Routes.** AD3 routes, sentences, audit, `POLICY_TABLE`, `route_auth.rs` both ways; route tests in `tests/upload_routes.rs` for each rule and the cross-tenant 404s.
- **T3 Ingest.** AD4: `folderId` in `parse_ingest_request`, carried through `claim_load` to the upload rows (grouped load included) and the claim; refused with a sentence when the folder is not the caller tenant's; route tests including a grouped load and a re-upload.
- **T4 Console: the tab.** AD6 tab, contract and client (`UploadFolder`, list/create/rename/move/delete, `moveUploads`, list filters), stubs updated in `uploads-panel.test.tsx` (its `stubFetch` throws on unknown calls); tests: tree, No folder, search across folders with the path, multi-move, delete disabled with the reason.
- **T5 Console: the Table step.** The folder picker with "New folder", preselected for an existing table, sent with `folderId`; retry keeps it; tests in `upload-file-page.test.tsx` (`describe("Table step")`).
- **T6 Gate and docs.** `ops/g9/upload_test.py` `step_folders` after `step_several_files` (create two levels, upload into the inner one, move, re-upload joins the folder, delete refused then allowed, depth 4 refused; cleanup); `CHANGELOG.md`; feature page limits (the developer's lines only; the planner does PRODUCT and BACKLOG).

## 4. Verify, do not assume

1. That the `upload_table_claim` row exists for every uploaded table the Table step can append to (T3).
2. That the static `/api/uploads/folders` and `/api/uploads/move` paths are not caught by `/api/uploads/{id}` (T2, a route test).
3. That `folder-tree.ts` with a depth parameter leaves the dashboard tests unchanged (T4).

## 5. Out of scope

Folders for connector tables, folders in the Data Explorer or the Catalog, permissions per folder, deleting a folder with its files (all on the feature page).

## 6. Rules

`AGENTS.md` in full; fixed sentences only (SEC-11); tenant scope on every query (SEC-16); cite `DATA-35 decision N` / `AD-N` at the code.

## 7. Handoff

Developer: a non-Opus agent (Claude Sonnet 5.5). Branch `feat/data-35-upload-folders`, worktree `/home/hv/lakehouse-data35`, stacked on `feat/data-phase-3-uploads` (#186). One task per commit, in plan order. Nothing was pushed and no pull request was opened or edited.

### Slice 1 (T1 to T6): commits

| Task | Commit | What |
| --- | --- | --- |
| T1 | `4b9a1653` | `0112_upload_folders.sql`; `lakehouse-store/src/upload_folders.rs` (rules of AD-2); `folder_id` on `Upload` and `COLS`; `list_filtered` (AD-5); `LoadClaim.folder_id`; `tests/upload_folders.rs` |
| T2 | `8de7fca9` | `routes/uploads/folders.rs`, router, `POLICY_TABLE`, `route_auth.rs` both ways; `folderId`/`q` on `GET /api/uploads`; `tests/upload_folder_routes.rs` |
| T3 | `16d9fb3e` | `folderId` on the ingest (`effective_folder`), carried through `claim_load` to every file of a group and the claim; route tests in `tests/upload_routes.rs` |
| T4 | `1a0fb156` | contract and client, `src/lib/upload-folders.ts`, tree, dialogs, the tab, tests |
| T5 | `8e0ffef4` | Table step folder picker (`upload-folder-field.tsx`), `retryInput` resends the folder, tests |
| T6 | `f19181b9` | `ops/g9/upload_test.py` `step_folders`; `CHANGELOG.md`; feature page limits |

### Commands run, with results

- `ls rust/migrations | cut -c1-4 | sort | uniq -d`: empty (no duplicate number).
- `cd rust && cargo fmt --check`: clean after `cargo fmt` on my files (`git status` showed only my files changed). That is the only cargo command run.
- `bun build --no-bundle <file>` (a transpile to stdout, a syntax check only) on every console file touched or added: all parse. This is not a typecheck.
- `python3 -m py_compile ops/g9/upload_test.py`: compiles; `python3 -I ops/g9/upload_test.py --help` prints the new docstring.

### Not verified, and why

- **Rust: `cargo clippy` and `cargo test`, not run** (`df -h /` showed 1.4 to 1.6 GB free, under the 8 GB the task allows). Everything in T1 to T3 is uncompiled: the store module, the routes, 13 store tests (`tests/upload_folders.rs`), 6 route tests (`tests/upload_folder_routes.rs`), 4 ingest route tests and a unit test (`routes/uploads.rs`, `tests/upload_routes.rs`). Expect compile or clippy findings on first CI.
- **Console: `bun test`, `bun run typecheck`, `bun run lint`, not run** (under 2 GB free; no `node_modules` symlink made). Two tests lean on base-ui `Select` option clicks (`moves a file into a folder chosen in the picker`, and the two option clicks in the Table step folder tests), which I could not try.
- **Gate `ops/g9/upload_test.py` `step_folders`: not run** (needs a deployed stack).
- **No browser check** of the tab or the Table step (feature page checklist `DATA-35-AC1` to `AC7` all Not run).
- **Verify item 1 of section 4** (a claim exists for every table the Table step can append to): by the code, `ingest` calls `claim_table` before `claim_load`, so the claim exists when the folder is recorded; tables a connector loads have no upload claim and are refused anyway. Not checked against a real database that predates `0058`.
- **Verify item 2** (static paths are not caught by `/api/uploads/{id}`): a route test exists (`the_static_paths_are_not_taken_by_the_per_upload_routes`), not run.
- **Verify item 3** (`folder-tree.ts` with a depth parameter leaves the dashboard tests unchanged): `folder-tree.ts` was not touched at all; the upload tab has its own `UPLOAD_FOLDER_MAX_DEPTH` (3) in `src/lib/upload-folders.ts`, because the 4 in `folder-tree.ts` is read only by the dashboard dialogs.

### Where the code differs from the plan's words (for the reviewer to accept or reject)

1. **`parent_id` is `NO ACTION`, not `RESTRICT` (AD-1).** Both refuse to delete a parent that still has children; `RESTRICT` is checked row by row, so a tenant delete (`ON DELETE CASCADE`) that removes a parent and its children in one statement would fail on whichever row it reached first. `file_upload.folder_id` is `RESTRICT` as the plan says. A consequence either way: a tenant whose files still sit in folders cannot be deleted from the database until they are moved out (stated in the migration header and on the feature page).
2. **The tab's search is a plain input with a 250 ms debounce, not `useTableUrlState` + `DataTableSearch` (AD-6).** That pair needs the data table's query-key context and nuqs; the panel is a plain table with none of it. The search is not in the URL.
3. **Folder actions are buttons under the tree (New folder, New subfolder, Rename, Delete folder), not a menu** (a menu is harder to test blind). Delete is disabled with the reason in text under the buttons.
4. **Dashboard dialogs not generalised (AD-6, the developer's choice):** upload-specific dialogs in `upload-folder-dialogs.tsx`. Reason: those are bound to `dashboardService`, move one board, carry the personal-space rule and a four-level limit.
5. **Ingest without `folderId` uses the table's remembered folder (AD-4, decision 2), and `folderId: null` means "no folder" on purpose.** The plan says only that the load records the folder on the claim; a server fallback makes "a re-upload joins the table's folder" hold for API callers and the gate too. The console always sends what the Table step shows.
6. **The Table step preselects the folder of the newest listed upload of the table**, not the claim's: no route returns the claim's folder (the plan's "through the list or a small field on the existing upload"). The two can differ after files are moved; the page limits say so.
7. **An explicit "No folder" on a re-upload does not clear the claim's folder** (store `folder_id: None` means "leave as is" for files and claim alike).
8. **Route tests for the folder routes are in a new file `tests/upload_folder_routes.rs`**, not `tests/upload_routes.rs` (that file's stand-ins for the bucket, Dagster and ClickHouse are not needed for folders and are not shareable); the ingest tests are in `tests/upload_routes.rs`.
9. **An extra audit event `upload.folder.move`** (a folder moved, as distinct from renamed), beside the five in AD-3.
10. **`GET /api/uploads` keeps `LIST_LIMIT` 100 with the filters**, so a folder with more than 100 files shows its newest 100.

## 8. Review

(Planner: findings tagged `BLOCKER` or `SHOULD-FIX`.)
