# `DATA-15` Asset page depth — Implementation Plan

| | |
| --- | --- |
| Item | `DATA-15`, Data roadmap phase 2 (master plan `2026-10-14-data-phase-2.md`) |
| Branch / PR | `feat/data-12-certification-governed-tags`, PR #106; built on the sub-branch `feat/data-15-asset-page` in worktree `/home/hv/lakehouse-data15`, merged back into the phase branch by the planner |
| Base verified at | `57de99d7` (phase branch, `origin/main` `53d05f00` merged in) |
| Scope | **The spec's Target rows and `DATA-15-AC1`–`AC4`** (`docs/core/specs/data-15.md`). The product owner decided on 2026-10-16 that phase 2 follows the specs; the feature page's extra decisions apply only where they agree with a spec row |
| Written | 2026-10-16 by the planner (Claude Opus) for a developer agent, under `AGENTS.md` |

## 1. The spec rows and what this plan builds

| Spec row | Target | This plan |
| --- | --- | --- |
| Data preview | Up to 1,000 rows, with masking applied | Built (task 1) |
| Usage | 30 days: queries, frequent users, frequent queries, tables joined with it, column popularity | Built (task 2). Column popularity is counted only from queries that read this table alone; a query that joins tables cannot say which table a bare column belongs to, and is left out and said so |
| Access | Grant and revoke from the asset, for users with the right to manage access | Built (task 3). The product's permissions are global (`query:read` reads every table), so a grant from this page gives a global permission for 30 days, and the page says so |
| Restore | Restore a dropped table within the retention window (7 days) | Built (tasks 4–5): soft delete with a 7-day window on every Lakekeeper warehouse, and undrop from the page. **Reported:** a table dropped before this lands was hard-deleted and cannot come back |
| History | Table versions with time, operation and author | Built on the read side (task 6): the author is read from the version's summary where the product recorded it. **Reported:** loads written through dlt record no author today, so those versions say "unknown"; recording it in the loader is new backlog item work for the orchestration owners |

## 2. What exists today (anchors at `57de99d7`)

`A` is `rust/crates/lakehouse-api/src`, `R` is `A/routes`.

| # | Code | Verdict |
| --- | --- | --- |
| F1 | `R/catalog.rs`: `SAMPLE_MAX_ROWS = 100` (`:1952`), `sample` (`:1971`) clamps `limit`, `governed_sample` (`:2005`) builds `SELECT * FROM {} LIMIT {limit}` (`:2030`) with no time limit and passes it through `rewrite_sql_for_principal`. Console `src/features/catalog/asset-sample.tsx` offers 25/50/100 (`:22`); `sample-grid.tsx` renders every row, no virtualisation. Test `sample_route_masks_rows_and_caps_the_limit` (`R/catalog.rs:5813`) pins 100. | Cap and console change. |
| F2 | `R/catalog_governance.rs`: `USAGE_DAYS = 7` (`:278`) is used by the asset page's `usage` (`:342`) **and** by search ranking `use_by_asset` (`:370`). `summarize_usage` (`:290`) returns `queries7d`, `users7d`, `avgLatencyMs`; only the caller's own SQL is shown, because SQL can carry literals. `history_mentioning` (`lakehouse-store/src/queries.rs:143`) returns at most 5,000 rows. `referenced_tables` (`A/sql_rewrite.rs:118`) gives every table a statement reads. No whole-statement column extractor exists. Console card `asset-activity.tsx:498`. | New window, new figures, a literal-free form of a query, a column counter. |
| F3 | Access requests: `access_request` (`R/catalog.rs:3301`), `decide_access_request` (`:3426`), `GRANT_DAYS = 30` (`:3407`); the decide audit is `resource_kind "approval"` and does not name the grantee. `access_grant` (`rust/migrations/0040_access_requests.sql:51`): `approval_id NOT NULL`, no `granted_by`, no asset column; `revoked_at`/`revoked_by_user_id` exist but nothing writes them. Grants are merged into the principal on every request (`lakehouse-auth/src/repository.rs:106`), so a revoke applies at the grantee's next request. `access:approve` is Governance Admin's (`0040:72`); POLICY_TABLE `A/policy.rs:225`. Requestable permissions in the console: `catalog:write`, `lineage:read`, `query:read` (`asset-detail-page.tsx:60`). | Direct grant, list, revoke are new. |
| F4 | Nothing in the product drops an Iceberg table. Every warehouse is created without a delete profile (`docker-compose.yml` `lakekeeper-warehouse-init`, the seaweedfs variant, `lakehouse-auth/src/openfga.rs` `ensure_warehouse`), and Lakekeeper 0.13.3's default is hard deletion. The admin client is `AppState::lakekeeper_admin: Option<…>` (`A/state.rs:181`), `None` when its token file is unreadable. `routes/storage.rs` `restore_asset` is a deprecated stand-in. A dropped Bronze table's page still opens (registry row) and shows "No Iceberg table for this asset" (`asset-activity.tsx:94`). | Soft delete, a status read and an undrop are new. |
| F5 | `SnapshotDetail` (`lakehouse-iceberg/src/rest.rs:264`) keeps four summary keys; `snapshot_json` (`R/lakehouse.rs:243`) emits no author. dlt writes no snapshot properties; the product's own pyiceberg commits are in `dagster/dispar_orchestrate/adapters/sink.py` (clear) and `adapters/history.py`. | Author is a summary key the product writes where it commits. |

Migration numbers: this item uses **`0078`** (reserved for `DATA-15` in the master plan; `0077` and `0080` are taken on other branches). Check `ls rust/migrations | cut -c1-4 | sort | uniq -d` prints nothing.

## 3. Decisions already made by the planner

- **Preview.** `SAMPLE_MAX_ROWS = 1000`. The raw read becomes
  `SELECT * FROM {} LIMIT {limit} SETTINGS max_execution_time = 30` before
  the rewrite (the rewrite keeps a trailing `SETTINGS`, shown in `DATA-16`).
  The console asks for 1,000 rows once and shows them 100 per page with a
  pager ("Rows 101–200 of 1,000"); the sort orders all rows loaded, and says
  so. `DETAIL_SAMPLE_ROWS` stays 5.
- **Usage window.** A new constant `ASSET_USAGE_DAYS = 30` for the asset
  page. Search ranking keeps its own 7 days under a renamed constant
  (`USE_RANKING_DAYS`), so `DATA-11`'s ranking does not change. The answer
  becomes `usage: { days: 30, queries, users, avgLatencyMs, basedOn,
  truncated, frequentUsers, frequentQueries, joinedTables, columns }`:
  - `basedOn`: how many history rows were read; `truncated: true` when the
    5,000-row read was full (the card says "based on the newest 5,000
    queries").
  - `frequentUsers`: top 10 `{ name, queries }`, name as the history records
    it.
  - `frequentQueries`: top 10 `{ sql, runs, lastAt }` where `sql` is the
    query with every literal replaced by `?` (a `sqlparser` visit of `Value`
    nodes, then `to_string`), and queries are grouped by that form. A query
    that does not parse is left out of this list and counted in
    `unparsed`. Nobody's literals leave the server.
  - `joinedTables`: top 10 `{ table, assetId, queries }` of the other tables
    read in the same statement (`referenced_tables`), `assetId` when the
    table is a catalog asset.
  - `columns`: top 20 `{ name, queries }` from statements that read this
    table and no other: every identifier in the statement whose last part is
    one of this table's columns counts once per statement. `columnsNote`
    states that statements reading several tables are not counted.
  `recentQueries` (the caller's own) stays as it is. The keys `queries7d`
  and `users7d` are removed; every consumer, mock and test is updated in the
  same task (no optional-ing).
- **Grants (`access:approve`).** Migration `0078_access_grant_direct.sql`:
  `access_grant.approval_id` becomes nullable; add `granted_by_user_id UUID
  REFERENCES app_user ON DELETE SET NULL`, `catalog_id TEXT` (at most 200),
  and a `CHECK` that a row has an approval or a granter. Routes, all
  `access:approve`, tenant check first like the other catalog writes:
  - `GET /api/catalog/{id}/grants`: active, unexpired grants made from this
    asset (direct ones by `catalog_id`, decided requests by their approval's
    resource), each `{ id, user, permission, grantedAt, expiresAt, via:
    "request" | "direct", grantedBy }`.
  - `POST /api/catalog/{id}/grants` `{ email, permission }`: `permission` one
    of the three requestable ones; the user found by email (unknown email:
    `404` "no user with that email"); granting to yourself refused (`403`,
    as self-approval is); the user already holding it refused (`409`).
    Creates a 30-day grant (`GRANT_DAYS`).
  - `DELETE /api/catalog/{id}/grants/{grantId}`: sets `revoked_at` and
    `revoked_by_user_id`; a grant already revoked or expired is `404`.
  - Each write audits with `resource_kind "catalog"`, `resource_id` the
    asset, `action` `catalog.grant` / `catalog.revoke`, args `{ permission,
    grantee, expiresAt }`, and `change_entry` gains the two sentences, so
    the asset's history names both people.
  - The Access tab gains a Grants card for `access:approve` holders: the
    list, a form (email, permission), Revoke. Its text says "A grant gives
    this permission for every table, for 30 days."
- **Soft delete (7 days).** Every warehouse gets the delete profile
  `{ "type": "soft", "expiration-seconds": 604800 }`: in the compose
  warehouse init (and its seaweedfs variant) after create, idempotent for an
  existing warehouse (`POST /management/v1/warehouse/{id}/delete-profile`),
  and in `ensure_warehouse` for tenant warehouses. Read Lakekeeper 0.13.3's
  management API shapes from the image or its published OpenAPI before
  writing; if they differ from these, stop and report.
- **Restore (`governance:write`).**
  - `GET /api/catalog/{id}/restore` (`catalog:read`): for an asset read from
    an Iceberg table that is missing, `{ restorable, droppedAt, expiresAt,
    reason }` from Lakekeeper's deleted-tabulars list for the namespace;
    for a present table `{ restorable: false, reason: "the table exists" }`;
    for a ClickHouse table `{ supported: false, reason }`; without the admin
    client `{ supported: false, reason: "restore is not configured on this
    install" }`.
  - `POST /api/catalog/{id}/restore`: undrop the matching tabular. Refused
    with a fixed sentence: a table of that name exists again (`409`; the
    next load recreated it), nothing restorable (`404`), window passed
    (`410`), not configured (`503`). Audited `catalog.restore`.
  - The asset page, where it says "No Iceberg table for this asset", shows
    the dropped date and, for `governance:write`, "Restore (until <date>)".
- **Author.** `SnapshotDetail` gains `author: Option<String>` read from the
  summary key `rantai.author`; `snapshot_json` emits `author` (null when
  absent); the Snapshots card adds an Author column showing "unknown" for
  null. The product's own pyiceberg commits (`sink.py` clear, `history.py`)
  set `rantai.author` to the job's run principal label (the connector id
  and run, e.g. `connector conn-x, run 1a2b`). dlt's commits are not
  changed by this task.

## 4. Tasks

One task per commit, Rust first. Cite `DATA-15` and the finding (`F1`…) at
each fix site and in the commit body.

1. **Preview (F1).** API cap, time limit, console pager. Tests: the sample
   test now pins 1,000 and the `SETTINGS` clause and still asserts masked
   rows; console tests: 1,000 rows shown 100 per page, the pager, the sort
   note.
2. **Usage (F2).** The two constants, the store read, the literal-free form,
   the three top-10 lists, the column count, the contract, the card, the
   mock. Tests: literals (`'x'`, `42`, `IN (1,2)`) become `?` and two
   queries differing only in literals group together; a join statement adds
   the other table to `joinedTables` and nothing to `columns`; a
   single-table statement counts each column once; `truncated` at the cap;
   search ranking still uses 7 days (`use_by_asset` test unchanged).
3. **Grants (F3).** Migration `0078`, store functions (create direct,
   list for asset, revoke), routes with `POLICY_TABLE` rows, audit, history
   sentences, the Grants card. Tests: store `sqlx::test`s for each function
   and the `CHECK`; handler tests for every refusal and the happy paths;
   `route_auth` named cases (a Data Engineer gets `403` on all three); a
   test that a revoked grant no longer reaches `load_principal_for_user`.
4. **Soft delete (F4).** Compose init(s) and `ensure_warehouse`. Check:
   `docker compose --profile '*' config --quiet`; a unit test for the
   request `ensure_warehouse` sends (wiremock). The real proof is task 5's
   gate step.
5. **Restore (F4).** The two routes, the admin-client calls, the console
   control. Tests: wiremock Lakekeeper for each answer; `route_auth` cases.
   Gate: a step in `ops/g3a/g3a_test.py` after its existing checks: drop
   the gate's Bronze table through the Iceberg REST catalog, see
   `GET …/restore` report it restorable, `POST …/restore`, read the table's
   row count back through the API. This is `DATA-15-AC3`'s proof.
6. **Author (F5).** `SnapshotDetail`, `snapshot_json`, the card, the two
   pyiceberg commit sites. Tests: a summary with and without the key; the
   Dagster unit tests for the property on both commit sites (no network).
7. **Handoff.** Scoped checks per commit; the full block once at the end
   (see section 5); append the Handoff entry here.

## 5. Build limits on this machine

Shared target `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target`,
`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`; `df -h /` before each cargo
command, stop under 10 GB free. No `cargo test`, no `cargo clean` (Rust tests
are *not verified*; CI on #106 runs them). `touch rust/crates/*/src/lib.rs
rust/crates/lakehouse-store/src/*.rs` before a quoted clippy run. Another
developer may be building `DATA-13` at the same time with the same target
dir: if cargo waits on the build lock, wait. TypeScript: `bun install
--frozen-lockfile`, then `bun run typecheck && bun run lint && bun run
test`. Python: the two `ops/lint` lines and the Dagster suite with
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate
-q` for task 6. No docker `up` here; compose is proven by CI's gates.

## 6. Out of scope

- Per-table permissions (`DATA-20`), an entity relationship diagram.
- Recording an author on dlt's commits (new backlog item for the
  orchestration owners).
- A drop button in the product.
- Counting dashboard reads as usage.

## Handoff

*(developer)*

## Review

*(planner)*
