# Schema versions for Silver and Gold tables — Implementation Plan

**Status:** approach approved by the product owner on 2026-10-05, not
started. Written by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md` on `main`. The planner writes no product code.

**Decision record:** `docs/adr/0015-recorded-schema-versions.md`. Read it
first; this plan does not repeat its reasoning.

**Base:** `feat/connectors`, in `/home/hv/lakehouse` (the product owner
asked on 2026-10-05 for the work to be made on this branch).

**Goal, in the user's words:** raw tables have schema versions and Silver
and Gold tables do not; add schema versions for those layers, recording them
when a build finishes and on a periodic check.

This file carries what a feature page would (what the user can do, limits,
checklist): the change is one card and one list on a page that exists.

---

## 1. What the user can do when this is done

1. Open a Silver or Gold table in Data Explorer and see, on the Schema tab,
   its schema versions newest first, each saying what changed: columns
   added, dropped, retyped or reordered.
2. See in the same card since when versions have been recorded, and that
   earlier changes are not known.
3. See the current version in the About card on Overview, as a raw table
   shows it.
4. See each schema change in the Activity tab's change history, among the
   other changes to the table.
5. See nothing invented: a table the console has not looked at yet says so.

Not included: reading a table's rows as they were under an earlier version;
versions for raw tables (they have their own, from the catalog); telling a
rename from a drop and an add.

## 2. Limits to tell a customer

- Versions are recorded from the day this is deployed. The first one is
  when the console first saw the table.
- A change is recorded when the console next looks: when a run finishes,
  and on the alerts schedule. Two changes between two looks show as one.
- A renamed column shows as one dropped and one added.
- Only the definition is kept, not the rows.

## 3. Anchors (verified at `2f51e38`)

Line numbers drift; re-find by symbol.

- **The pattern to copy for the store:** `routes/quality.rs`,
  `ensure_table` (~351: `CREATE DATABASE IF NOT EXISTS console`, `CREATE
  TABLE IF NOT EXISTS console.quality_run …`, guarded by a `OnceCell`) and
  `record_run` (~373, values through `SqlLiteral`).
- **The pattern to copy for the background pass:** `routes/quality.rs`,
  `PASS_RUNNING` (~745), `PassGuard` (~747), `spawn_scheduled_pass` (~870).
- **The tick:** `routes/alerts.rs::run` calls
  `quality::spawn_scheduled_pass(&state)` (~581) and reports
  `qualityPassStarted`.
- **A run finished:** `routes/pipelines.rs::run_finished_event` (~1914).
  The file is the pipeline team's, taken from `main`: add one line and
  nothing else (`routes/lineage.rs` is treated the same way).
- **Start-up:** `main.rs`, after the `bootstrap_*` calls (~83-115).
- **Which databases:** `silver` and `serving`, the pair
  `routes/catalog.rs::clickhouse_asset_detail` serves (`db != "silver" &&
  db != "serving"`). *Corrected in review (R1):* this line first named
  `Config::gold_source_schema`, which only the export routes read; the
  catalog opens Gold tables as `serving.<table>` whatever that setting is.
- **How a table's columns are read today:** `routes/catalog_source.rs`
  `clickhouse_source` (~64: `system.columns … ORDER BY position`).
- **Where the detail of a Silver or Gold table is built:**
  `routes/catalog.rs` `clickhouse_asset_detail` (~1699) and
  `clickhouse_detail_body` (~2098), which sets `"schemaVersions": []`
  (~2131). `mark_governance` (~1523) and `mark_annotation_and_history`
  (~1359) are how other facts are added to a detail body after it is built.
- **The engine client:** `lakehouse-clickhouse` `ChClient::rows`, `exec`
  (`lib.rs` ~225, ~244); `lakehouse_core::ident::SqlLiteral`;
  `routes::lakehouse::is_unknown_table_error`.
- **Console, already there:** `AssetDetail.schemaVersions: { version, at,
  change }[]` (`src/services/contracts/assets.ts` ~174). `asset-schema.tsx`
  `SchemaVersions` (~263) already lists `a.schemaVersions` when the table
  has no Iceberg versions, and its empty state says an engine table keeps
  no history. `asset-overview.tsx` (~127-133) already labels the About row
  from `a.schemaVersions[0]`. `asset-activity.tsx` `changeTimeline` (~355)
  merges only Iceberg versions into the change history.

## 4. Tasks

One task per commit when the product owner has looked; until then the work
stays **uncommitted** in the working tree (section 6).

### T1 — Record and read versions (API)

New module `rust/crates/lakehouse-api/src/routes/schema_versions.rs`,
declared in `routes/mod.rs`. A module doc that says what ADR 0015 decides
and what a version is not.

- **Store.** `console.table_schema_version`: `table_key String`, `version
  UInt32`, `columns String` (a JSON array of `[name, type]` pairs, in the
  table's column order), `observed_at DateTime64(3, 'UTC')`; `MergeTree
  ORDER BY (table_key, version)`. Created on first use, as `quality.rs`
  does.
- **Pure functions**, unit-tested without an engine:
  - `changes(before, after) -> Vec<String>`: one sentence per change,
    matched by name: `Added <name> (<type>)`, `Dropped <name> (<type>)`,
    `Changed <name> from <type> to <type>`; when the same columns with the
    same types only moved, `Reordered columns`. Never empty for two
    different lists.
  - The first version of a table reads `First recorded with <n> columns`
    (`1 column` in the singular).
  - `entries(versions) -> Vec<…>`: newest first, each `{ version, at,
    change, current }`, `change` the sentences joined with ` · `, `current`
    true on the newest only. Two consecutive stored versions with the same
    column list (two passes that raced) are one entry.
- **One pass**, `observe(state) -> Result<usize, ChError>` (how many
  versions it recorded):
  1. Read every table's ordered columns in `silver` and `serving` (see
     section 3; not the export setting) from `system.columns`, in one
     statement. The two database names are passed as `SqlLiteral`s.
  2. Read the newest recorded column list per table from the store, in one
     statement.
  3. For each table whose list differs from its newest recorded one, or
     that has none: insert version `newest + 1` (from 1), `observed_at =
     now64(3)`, in one `INSERT` for the whole pass.
  A table with no columns is skipped. More than 2,000 tables: record the
  first 2,000 by name and log that the rest were not looked at; never
  silently.
- **Background start**, `spawn_pass(state) -> bool`: one pass at a time, a
  guard that clears however the pass ends, the shape of
  `quality::spawn_scheduled_pass`. A failed pass is logged and changes
  nothing.
- **Read**, `for_table(ch, table_key) -> Vec<entry>`: the table's entries.
  No store yet (`is_unknown_table_error`) is an empty list. Any other
  failure is logged and is an empty list too: the page still loads, and an
  empty list already reads as "none recorded".
- **Hooks:**
  - `routes/alerts.rs::run`: beside the quality pass, on the tick only
    (not with `?id=`); the answer gains `schemaPassStarted`.
  - `routes/pipelines.rs::run_finished_event`: one line, after the caller
    is known to be the orchestrator and before anything can return early.
    Cite ADR 0015 in a one-line comment. Touch nothing else in that file.
  - `main.rs`: once at start-up, after the bootstraps, in the background;
    an engine that is not reachable yet is a logged warning.
  - `routes/catalog.rs`: the detail of a Silver or Gold table carries
    `schemaVersions` from `for_table`. Raw tables are untouched.
- No new route, so no `POLICY_TABLE` entry. The detail route's existing
  permission and tenant gate cover the new field.
- **Accept:** unit tests for `changes` (each kind, several at once, the
  reorder case), the first version's sentence, `entries` (order, `current`,
  the collapsed duplicate). `wiremock` tests for `observe`: a new table
  gets version 1; an unchanged table gets nothing; a changed one gets the
  next number; one `INSERT` for several tables; a failed read records
  nothing. A test that the pass does not start twice at once. A test that
  the detail body of a Silver table carries the entries, and that a failed
  read leaves it an empty list. The alerts route test for
  `schemaPassStarted`.

### T2 — Show them (console)

- `asset-schema.tsx`, `SchemaVersions`: for a table without Iceberg
  versions, when `a.schemaVersions` has entries: the list as now, with a
  `current` pill on the newest, and the card's description reading
  "Recorded by the console each time this table's columns change, since
  <date of the oldest entry>. Changes before that are not known, and a
  renamed column shows as one dropped and one added." When it has none and
  the table is not an Iceberg table: an empty state titled "No schema
  version recorded yet", saying the console records one when it next looks
  at the table. The old sentence that an engine table keeps no history
  goes.
- `asset-activity.tsx`, `changeTimeline`: also merge `a.schemaVersions`
  into the change history (`actor` "Schema v<n>", `summary` the change,
  `at` its time). A raw table's history is unchanged.
- `contracts/assets.ts`: the entry type gains `current: boolean`; the
  comment says where the list comes from for each kind of table. Update
  the in-browser fixture if it sets the field.
- The About row on Overview already works from the list; keep it, and make
  it say "recorded <when>" for these tables so it does not read as the
  table's age.
- **Accept:** component tests for the list with its description and pill,
  the empty state, the change history with a schema entry among audit
  entries, and that a raw table's cards are unchanged.

### T3 — Documents

- `CHANGELOG.md` `[Unreleased]`: one entry.
- `docs/OPERATIONS.md`: the new `console.table_schema_version` table; that
  versions are recorded on the alerts schedule and when a run finishes, so
  an install without the orchestrator records them only at start-up.
- `docs/FEATURE_COVERAGE.md`: the asset detail row.
- **Accept:** each sentence names only what exists.

## 5. Out of scope

- A manual "record now" button or route.
- Versions for raw tables, or any change to how theirs are read.
- Recording that a table was dropped.
- Any change to `routes/pipelines.rs` beyond the one line.
- Any Postgres migration.

## 6. Working on this machine

- Work in `/home/hv/lakehouse` on `feat/connectors`. It is the product
  owner's checkout: their console dev server (port 3000) reloads every
  saved file under `src/`. Never kill or restart it, never run `next
  build` or `bun install` here, and do not leave `src/` in a state that
  does not compile.
- **Leave the work uncommitted.** The product owner looks first. Do not
  `git add`, commit, stash, reset or switch branches. Never push.
- Untracked and not yours: `docs/plans/FEAT-CONNECTORS-REPORT.md`,
  `lark-import/`, `ops/g3/bronze_catalog.py`. Never read or print `.env`.
- Rust: `cd rust && export
  CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
  CARGO_BUILD_JOBS=4`. `df -h /` before a build; stop and report under
  15 GB free. No `cargo clean`, no `--release`. Per task: `cargo fmt
  --check`, `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
  `cargo test -p lakehouse-api <filter>`. Run cargo from a copy of the tree
  only if a test reads `.env` by accident (one state test is known to; it
  is not yours to fix).
- No `docker` commands. The reviewer builds and restarts the API.
- No prettier; `rustfmt` only on files where every hunk is yours.
- Before the handoff, once: `cargo fmt --check && cargo clippy --workspace
  --all-targets --all-features -- -D warnings && cargo test -p
  lakehouse-api`; `bun run typecheck && bun run lint && bun run test`.

## 7. Acceptance checklist (product owner)

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Open a Gold table, Schema tab | "Schema versions" lists v1, "First recorded with N columns", marked current, with the sentence saying since when | |
| 2 | Overview, About card | A Schema row: v1 and when it was recorded | |
| 3 | Activity tab, change history | An entry "Schema v1" | |
| 4 | Open a Silver table | The same three | |
| 5 | Open a raw table | Its versions as before, from the catalog | |
| 6 | (operator) Add a column to a Gold table, wait for the next tick, reload | v2, "Added <name> (<type>)", current; v1 below it | |
| 7 | (operator) Drop that column, wait, reload | v3, "Dropped <name> (<type>)" | |

## 8. Handoff (developer)

T1, T2 and T3 are in the working tree, uncommitted. Round 1 built them;
round 2 applied the planner's review (R1-R4) and one blocker (B1). A pass of
a running API against a running `ClickHouse` has not been run, and the
console was not looked at in a browser: the API on the dev stack has not been
rebuilt, so a Silver or Gold page shows "No schema version recorded yet"
until it is.

- **API.** New `routes/schema_versions.rs` (store, `changes`, `entries`,
  `observe`, `spawn_pass`, `for_table`), declared `pub(crate)` in
  `routes/mod.rs` because `main.rs` calls it. Hooks: `main.rs` after the
  bootstraps, `alerts::run` (`schemaPassStarted`, not on `?id=`), one
  statement in `pipelines::run_finished_event` right after the caller check,
  and `catalog::mark_schema_versions` in `clickhouse_asset_detail`.
- **Console.** `contracts/assets.ts` (`current`), `asset-schema.tsx`
  (list, `current` pill, since-date description, new empty state),
  `asset-activity.tsx` (`changeTimeline` takes the recorded list as a third
  argument), `asset-overview.tsx` ("v2 · recorded 3d ago"), the in-browser
  fixture, tests in `asset-detail-tabs.test.tsx`.
- **Sentence order in `changes`:** Added and Changed in the table's column
  order, then Dropped, the order the raw-table wording already has
  (`src/lib/schema-history.ts`). `Reordered columns` only when it is the
  only difference.
- **Round 2.**
  - **R1.** The pass looks at the constants `SILVER_DATABASE` (`silver`) and
    `GOLD_DATABASE` (`serving`), the pair `catalog::clickhouse_asset_detail`
    serves, and no longer reads `Config::gold_source_schema` (the export
    routes' setting). Their docs name that function. Module doc and
    `docs/OPERATIONS.md` follow; the paragraph about the non-default setting
    is gone, since the gap is. Tests pin the two names and that a
    non-default `GOLD_SOURCE_SCHEMA` does not change them. The plan's anchor
    "Which databases" (section 3) and step 1 of T1 name the setting; they are
    superseded by this.
  - **R2.** The pass's two reads and `for_table` go through `read`, which
    uses `ChClient::query` and treats an answer with empty `meta` as a failed
    read (`ChClient` turns a `2xx` body that is not JSON into an empty
    result, which `rows` would call "no rows" and the pass would answer with
    version 1 for every table). `observe` returns a private `EngineError`
    (`Failed(ChError)` or `NotAResult`, `thiserror`); `observe` is now private
    to the module for that reason. The fake engine's reads now send a real
    `meta`; tests cover an empty `200` on each read and on `for_table`.
  - **R3.** `is_unknown_table_or_database_error` in `routes/lakehouse.rs`
    beside `is_unknown_table_error`, with unit tests. Used in
    `schema_versions.rs` (the private copy is gone), `quality.rs` (one
    condition) and `governance.rs::is_missing_quality_source`, which keeps
    only its `Code: 81.`.
  - **R4.** Both limits (one writer per engine; no cap on the read, and why)
    are in the module doc and in `docs/OPERATIONS.md`.
  - **B1.** The newest-version read aliased `max(version) AS version`, which
    the real engine refuses (`Code: 184`, `ILLEGAL_AGGREGATION`). It now
    selects `AS newest_version` and `AS newest_columns`; the row readers and
    mocks follow, and a test asserts on the sent text that no aggregate is
    aliased to a column name.
- **Ran, from this tree.** See the report that came with this handoff for
  the exact commands and counts.

## 9. Review (planner)
