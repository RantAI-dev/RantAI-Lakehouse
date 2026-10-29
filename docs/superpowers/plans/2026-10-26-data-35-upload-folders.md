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

### Round 1, 2026-10-28, at `54588c4e` (read, not run; verification runs on the fix round's final commit)

**BLOCKER 1. "No folder" is shown but not sent, or sent and ignored (decision 2; deviations 5 and 7).**
The Table step leaves `folderId` out when the picker is untouched and the table's newest listed upload is in no folder (`src/features/connectors/upload-file-page.tsx:115-116`, `:177`); the route then falls back to the claim's remembered folder (`routes/uploads.rs:2499-2501`, `FolderRef::Absent`). An explicit `null` becomes `None` (`FolderRef::Null`), and the store writes `folder_id = COALESCE($7, folder_id)` / `COALESCE($8, folder_id)` (`lakehouse-store/src/uploads.rs:1005`, `:1030`), which keeps the file's folder. `retryInput` (`src/lib/uploads.ts:362`) also leaves the folder out when it is unset. Failures:
- Load table `t` into folder A (the claim remembers A); move the file to No folder (a move never touches the claim); re-upload into `t`: the step shows No folder, nothing is sent, the file lands in A.
- Load `f` with No folder while the claim remembers A, and the load fails; Try again omits `folderId`; `f` lands in A.
- Re-open a file that sits in folder B and choose No folder: `null` is sent, `COALESCE` keeps B, and the choice is dropped without a word.

The comment at `upload-file-page.tsx:176` ("the folder shown on the Table step is the one sent, `null` included") is false (rule 1). Fix:
- The console always sends what the picker shows (`folderId: folderId ?? null`) whenever the picker is rendered, and `retryInput` sends `upload.folderId ?? null`.
- The store's folder input becomes three-state (keep / set none / set some), so an explicit `null` sets `folder_id = NULL` on the primary and every file of the group.
- An explicit `null` also sets the claim's folder to none: the claim records the latest load's choice (AD-4).
- "Absent means the claim's folder" stays, for API callers only.
- Tests: update `upload-file-page.test.tsx:1052` and `:1127`, which pin the omission today, and `uploads.test.ts:521`. Add route tests for "re-load a file that is in a folder with `null`" (the file and the claim end with no folder) and "absent `folderId` uses the claim's folder".
- Rewrite the feature page line `upload-folders.md:65` to match.

**SHOULD-FIX 2. The migration header gives a false reason for `NO ACTION` (deviation 1; rule 1).** `rust/migrations/0112_upload_folders.sql:33-38` says `RESTRICT` "would fail on whichever row the cascade reached first". Checked on Postgres 16, in a rolled-back transaction: a tenant `ON DELETE CASCADE` over a three-level chain with `parent_id ON DELETE RESTRICT` succeeds. Both are checked at the end of the statement; `NO ACTION` can additionally be deferred. Keep `NO ACTION` (accepted) and correct the sentence. The migration is not applied on `main`, so editing it is allowed. The real consequence stands: a tenant with a file still in a folder cannot be deleted, `23503` through `file_upload.folder_id RESTRICT`. Add that cause to the `# Errors` of `identity::delete_tenant` (`lakehouse-store/src/identity.rs:876`), which names only `app_user_tenant`.

**SHOULD-FIX 3. Merge `main`.** The branch is 297 commits behind and conflicts in `CHANGELOG.md` and `docs/core/BACKLOG.md`. `policy.rs`, `routes/mod.rs`, `tests/route_auth.rs` and `lakehouse-store/src/lib.rs` auto-merge, but nothing has compiled them together, and the green CI at `54588c4e` is for the old base. Migration `0112` is still free (`main` stops at `0111`; `feat/bi-19-access` uses `0113`–`0114`). Resolve hunk by hunk (rule 9), and in `BACKLOG.md` keep `main`'s rows and change only `DATA-35`'s.

**SHOULD-FIX 4. The CHANGELOG entry ends with a stale sentence.** The `DATA-35` entry (`CHANGELOG.md:34`) says the tests "were written but not run … the pull request's CI runs them". Delete the sentence: CI has run them, and the fix round re-runs everything.

**SHOULD-FIX 5 (small, same round).**
- `FOLDER_ID_RULE` is defined twice, at `routes/uploads.rs:284` and `routes/uploads/folders.rs:55` (rule 4): keep one.
- The rename audit records the untrimmed name (`folders.rs:278`): record `changed.name`.

**Checked and fine.**
- The store rules are enforced in the database and are race-safe: one transaction under `pg_advisory_xact_lock(hashtext('upload_folder:'||tenant))` for create, update, delete and move. Depth is `parent_depth + subtree height > 3`. Sibling names are unique through the `lower(name)` index (`23505` → `NameTaken`). An upload racing a delete is refused by the `RESTRICT` key (`23503` → `NotEmpty`).
- The tenant is bound in every query. Ingest checks `folder_in_tenant` before it claims. A move is all-or-nothing.
- Routes: static paths come before `{id}`; all five are `connector:manage` and asserted both ways in `route_auth.rs`; responses use fixed sentences and `ApiJson`; there is no `unwrap` outside tests; SQL values are bound.
- Console: imports go through `@/services`, fetches through `apiFetch`, and there are no new mocks.
- The gate's `step_folders` follows the Python rules.
- The commits after the handoff (`57595d76`, `5c1b8f1d`, `371084d2`, `54588c4e`) weaken nothing.

**Deviations.** Accepted: 2, 3, 4, 8, 9, 10. Accepted with conditions:
- 1: accepted, but its stated reason is false (SHOULD-FIX 2).
- 5: accepted for API callers only, once BLOCKER 1 is fixed.
- 6: accepted for this slice. A table whose uploads have dropped out of the newest-100 window then preselects No folder; returning the claim's folder from an API route is the follow-up.

Rejected:
- 7: rejected (BLOCKER 1).

Open: BLOCKER 1, SHOULD-FIX 2 to 5.

### Handoff (round 1 fixes)

Developer: a non-Opus agent. Branch `feat/data-35-upload-folders`, worktree
`/home/shiro/rantai/wt-pr259`, base `main` at `481fd7e8`. Nothing was opened,
merged or pushed to `main`; the branch is pushed to
`origin/feat/data-35-upload-folders`.

#### Commits

| Finding | Commit | What |
| --- | --- | --- |
| SHOULD-FIX 3 (the merge) | `d04a84cf` | `main` merged in; `CHANGELOG.md` and `docs/core/BACKLOG.md` resolved by hand |
| BLOCKER 1 | `76fa1a5e` | the console sends the folder the Table step shows; the store's folder input is three states |
| SHOULD-FIX 2 | `59f3d47e` | the migration header's false reason for `NO ACTION`; `delete_tenant`'s `# Errors` |
| SHOULD-FIX 4 | `59185a9f` | the stale sentence at the end of the `DATA-35` changelog entry |
| SHOULD-FIX 5 | `4b7dc55e` | one `FOLDER_ID_RULE`; the rename audit records `changed.name` |

#### What each finding got

**BLOCKER 1.** The console now sends what the picker shows: `upload-file-page.tsx`
sends `folderId: folderId ?? null` whenever the picker is rendered (a picker that
could not be read sends no folder, and the table's remembered folder applies),
and `uploads.ts` `retryInput` sends `upload.folderId ?? null`. The store's folder
input became three states, `ClaimFolder` (`Keep`, `None`, `Some(folder)`, not an
`Option<Option<String>>`): an explicit "no folder" sets `folder_id = NULL` on the
primary and on every file of a grouped load and sets the claim's folder to none
(the claim records the latest load's choice, AD-4); an absent `folderId` still
falls back to the claim's folder, for API callers. The comment at
`upload-file-page.tsx` that said the folder shown is the one sent was false and
now says what the code does. Tests: a store test for the three states, and three
route tests (a re-load with `folderId: null` of a file that sits in a folder; an
absent `folderId` that uses the claim's folder; a grouped load with `null`). The
three console tests that pinned the omission now pin the sending of `null`. The
feature page's line about a table remembering its folder and the `DATA-35`
changelog entry were rewritten to match.

**SHOULD-FIX 2.** `NO ACTION` is kept; the sentence that said `RESTRICT` "would
fail on whichever row the cascade reached first" is corrected (both are checked at
the end of the statement, so a tenant cascade over a chain of folders succeeds
under either; `NO ACTION` can also be deferred). The real consequence is now
written where the reader looks: `identity::delete_tenant`'s `# Errors` names the
`23503` through `file_upload.folder_id`'s `RESTRICT`.

**SHOULD-FIX 4.** The sentence "The Rust and console tests of this change were
written but not run on the machine they were written on; the pull request's CI
runs them." is gone.

**SHOULD-FIX 5.** `FOLDER_ID_RULE` is defined once, in `routes/uploads.rs`, and
the folders' routes read it (`pub(super)`). The rename audit records
`changed.name`, the name the folder ended up with, not the one that was typed
(the store trims it); the route test renames a folder to `"  Old  "` and pins the
audit's name as `"Old"`.

#### The merge, per file

- `docs/core/BACKLOG.md`: every row is `main`'s, unchanged — the statuses, the PR
  numbers and the dates of `BI-1`, `BI-4`, `BI-5`, `BI-16`, `BI-17`, `BI-19`,
  `BI-26`, `DATA-17`, `DATA-18`, `SEC-20`, `OPS-12`, `OPS-13` and the rest. Only
  `DATA-35`'s own text is this branch's: its row in the feature table, its entry
  in "Ready to start", and its line in the "Next" table. No other row's status,
  PR or date was touched.
- `CHANGELOG.md`: both sides' entries are kept. `main`'s `Added` entries (the
  embedding `BI-26`, the public links `BI-19`, the uploads of 2 GB, the file
  formats, the scheduled reports `RPT-1`, the export `BI-1`), its `Changed` entry
  (the Rust CI job, `OPS-12`) and its `Fixed` entries (the catalog sample per
  tenant, the history page's cursor) are all there, and `DATA-35`'s entry stands
  at the top of `Added`.
- `policy.rs`, `routes/mod.rs`, `tests/route_auth.rs` and
  `lakehouse-store/src/lib.rs` merged without a conflict.

#### Commands run, with results

Per commit, on the tree of that commit (the machine's `cargo` lock serialises
every invocation behind another worktree's build, so the runs are one per
finding, not one per file):

- `cd rust && cargo-locked fmt --check`: clean, on every commit.
- `cd rust && cargo-locked clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` (BLOCKER 1's tree): clean.
- `cd rust && cargo-locked clippy -p lakehouse-store --all-targets -- -D warnings` (SHOULD-FIX 2's tree): clean.
- `cd rust && cargo-locked clippy -p lakehouse-api --all-targets -- -D warnings` (SHOULD-FIX 5's tree): clean.
- `cd rust && cargo-locked test -p lakehouse-store --test upload_folders --test uploads` (BLOCKER 1's and SHOULD-FIX 2's trees): 14 and 59 pass, 0 fail.
- `cd rust && cargo-locked test -p lakehouse-api --test upload_routes --test upload_folder_routes` (BLOCKER 1's tree): 147 and 6 pass, 0 fail.
- `cd rust && cargo-locked test -p lakehouse-api --test upload_folder_routes` (SHOULD-FIX 5's tree): 6 pass, 0 fail.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 11 warnings (all pre-existing). `bun run test`: 2925 pass, 1 skip, 0 fail, 8031 assertions, 299 files.

The full block, on the final commit (`4b7dc55e`):

- `cd rust && cargo-locked fmt --check`: clean.
- `cd rust && cargo-locked clippy --workspace --all-targets --all-features -- -D warnings`: clean.
- `cd rust && cargo-locked test --workspace`: **157 binaries, 8271 pass, 0 fail, 49 ignored**. (Built with `CARGO_PROFILE_DEV_DEBUG=line-tables-only` and
  `CARGO_PROFILE_TEST_DEBUG=line-tables-only`, as CI does (`OPS-12`): with the
  full debug information the 68 `lakehouse-api` test binaries filled the disk and
  the linker died with a bus error. The shared `CARGO_TARGET_DIR` was pruned with
  the repo's own `cargo-target-prune.sh` under the heavy lock; no `cargo clean`,
  no per-worktree `target/`.)
- `bun run typecheck`, `bun run lint`, `bun run test`: as above.
- `python3 -m py_compile ops/g9/upload_test.py`: compiles.
- `python3 ops/lint/check_intra_package_imports.py`: "all intra-package
  dispar_orchestrate imports resolve".
- `python3 ops/lint/check_bare_iceberg_count.py`: "no bare count() against a
  Bronze Iceberg table found".
- `cd dagster && python -m pytest dispar_orchestrate -q`: **2083 pass, 1 skip,
  306 deselected** (`-m "not loader"`, 46.88 s) and **306 pass, 2084 deselected**
  (`-m loader`, 1289.69 s). Run with `/home/shiro/rantai/venv-pr186/bin/python`
  (pytest 9.1.1, not CI's 8.3.4): the machine's system `python3` has no pytest.
- `docker compose --profile '*' config --quiet`: clean with the CI's throwaway
  values. Without them it refuses to start, as it must (`SEC-18`: no default
  credentials).
- `git status`: clean; nothing generated is staged.
- `ls rust/migrations | cut -c1-4 | sort | uniq -d`: empty (no duplicate number).
- `grep -rn '^<<<<<<<\|^>>>>>>>'` over the tracked files: no conflict marker left.
- Exited testcontainers: `docker ps -a --filter
  label=org.testcontainers.managed-by=testcontainers --filter status=exited -q |
  xargs -r docker rm -v` removed 0 (the shared container is still up).

#### Not verified, and why

- **The gate `ops/g9/upload_test.py` `step_folders`: not run.** It needs a
  deployed stack (the API, the object store, Dagster and Postgres), and this
  machine has none running.
- **No browser check** of the Uploaded files tab or of the Table step: the
  feature page's checklist `DATA-35-AC1` to `AC7` are all "Not run", which needs
  a running stack and a browser.
- **`cargo deny check` and `cargo audit`: not run.** They are the security
  workflow's checks, not the verification block, and they need the network to
  fetch the advisory database. `rust/deny.toml` was not touched by this round.
- **The `lakehouse-api` test binaries were built with `line-tables-only`**, as
  CI does, so a backtrace prints a file and a line but not the full debug
  information. Nothing else differs from a default build.
- **The dagster tests ran on pytest 9.1.1**, not CI's 8.3.4; both runs are
  green.

#### Where the code differs from the plan's words

1. **The three states live in the store, not in the route.** `ClaimFolder` is a
   store type (`lakehouse_store::uploads`), and the API's `FolderRef` maps onto
   it in `effective_folder`. The plan's AD-4 only says the load records the
   folder on the claim; the three states are the review's BLOCKER 1, applied at
   the store so that the file, the group and the claim are written by one
   statement each.
2. **An absent `folderId` leaves the file's folder alone** when the table's
   claim remembers none. The plan and the review say "absent means the claim's
   folder"; a table that remembers none has no folder to join, and a file that
   was moved after its load keeps the folder it was moved to. Only an explicit
   `null` changes a file's folder.
3. **The console sends no folder at all when the picker could not be rendered**
   (the folder list could not be read). The plan's deviation 5 says the console
   always sends what the Table step shows; with no picker there is nothing to
   show, and the table's remembered folder applies, as it did before.

### Round 2, 2026-10-28, at `ad35f041`

- **BLOCKER 1: fixed** (`76fa1a5e`). Read at the code:
  - `ClaimFolder` has three states (keep / none / some). The file and group updates use `CASE WHEN <named> THEN <folder> ELSE folder_id END`, and the claim is updated only when a folder was named. So an explicit `null` clears the folder on the file, on every file of the group, and on the claim.
  - An absent `folderId` maps to the claim's folder, or to keep when the claim remembers none.
  - The console sends `folderId ?? null` whenever the picker is rendered, and `retryInput` always sends the folder.
- The handoff's three new deviations are all accepted:
  - 1: the three states live in the store.
  - 2: an absent value with no remembered folder keeps the file where it is.
  - 3: no folder is sent when the folder list could not be read.
- **SHOULD-FIX 2, 4 and 5: fixed** (`59f3d47e`, `59185a9f`, `4b7dc55e`). **SHOULD-FIX 3: fixed** (`d04a84cf`). BACKLOG and CHANGELOG were checked against `main`: only `DATA-35`'s own lines differ.
- **Reviewer verification at `ad35f041`**, run in the foreground with `line-tables-only` as CI does:
  - `cargo fmt --check` clean.
  - `cargo clippy --workspace --all-targets --all-features -D warnings` clean.
  - `cargo test --workspace`: 157 binaries, 8271 passed, 0 failed, 49 ignored.
  - `bun run typecheck` clean; `bun run lint` 0 errors (the 11 warnings already on `main`); `bun run test` 2925 pass, 1 skip, 0 fail.
- Then `main` was merged in (#256, #257, and #267, the `Cargo.lock` fix for `main`'s stale lock). The merge has no conflicts and changes only `Cargo.lock` and `lakehouse-api/Cargo.toml`, and `cargo metadata --locked` passes. CI runs the full set on that merge.
- *Not verified*: the g9 gate `step_folders` and the browser checks `DATA-35-AC1`…`AC7` need a running stack. They are for the product owner's acceptance.

No open BLOCKER.
