# DATA-24: catalog search follow-ups

Backlog `DATA-24` (Task, Data, P2), from the delivery review of 2026-10-12. Reassigned to the
planner lane on 2026-11-01. Base: `main` at `9131dced`. Planner: Claude Opus. Developers: other agents
(`AGENTS.md`). If the code disagrees, stop and report it.

## 0. The four sub-items, checked

1. **The 500 ms target on 10,000 tables was never measured** (`catalog-search.md:60`,
   acceptance row 14 accepted as an exception).
   - `docs/plans/DATA-11-RESULT.md` does not exist.
   - The harness exists, `ops/bench/catalog_search_bench.py`, with two subcommands:
     - `seed` writes 10,000 datasets of 20 columns. It needs
       `BENCH_THROWAWAY_CATALOG=true` and `BENCH_CLICKHOUSE_URL`.
     - `run` sends 50 searches to each route, after waiting out the 30-second cache, and prints
       JSON.
   - It has never run against a stack.
2. **The Owner and Tags filters vanish on a laptop.**
   - `src/features/catalog/data-explorer-page.tsx:189-190` hides `tags` below 1760 px and
     `owner` below 1500 px.
   - Hidden columns are **removed** from the column list (`:223-227`), so their filters go with
     them (`:175`).
3. **The column rows are read without `FINAL`.**
   - `search_column_index` (`rust/crates/lakehouse-api/src/routes/catalog.rs:749-755`) reads
     `bronze_meta*.dataset_column`, a `ReplacingMergeTree ORDER BY (slug, key_asli)`
     (`dagster/dispar_orchestrate/bronze_catalog.py:136-146`) that each connector run writes
     again.
   - The per-slug column count (`catalog.rs:528-529`) has the same problem, so `col_count` can
     be inflated.
   - `scripts/seed-clickhouse.sql:170,207` declares those tables as plain `MergeTree`, which
     disagrees with Dagster.
4. **Acceptance rows 10, 11 and 13 already passed** on 2026-10-23
   (`catalog-search.md:99-112`). **Nothing to do.** The backlog row is out of date here.

## 1. Decisions

- **D1 (sub-item 2). Hide, do not remove.** At narrow widths the Owner and Tags columns become
  **hidden** through the table's column visibility, not removed from the column list. Their
  filters therefore stay available in the toolbar's filter menu at every width.
  - Keep the same thresholds and the existing "columns hidden" note, which now also says
    "filters still work".
- **D2 (sub-item 3).**
  - `search_column_index` and the column count read with `FINAL`, the way the asset-detail
    query already does (`catalog.rs:2337-2345`, which says why).
  - `scripts/seed-clickhouse.sql` declares both tables as
    `ReplacingMergeTree ORDER BY (slug, key_asli)`, matching `bronze_catalog.py`.
- **D3 (sub-item 1).** A new manual workflow, `.github/workflows/catalog-search-bench.yml`
  (`workflow_dispatch`), starts the core stack on a GitHub runner, runs the bench's `seed`
  into a throwaway catalog, then `run`, and writes the JSON to the step summary.
  - **The planner runs it** and writes `docs/plans/DATA-11-RESULT.md`: the time, the command and
    the run, and whether 500 ms was met. A miss is reported, not hidden (decision 1 of the feature
    page).
  - Fix whatever the bench needs to run (it has never run). Do not change what it measures.
- **D4.** No change for sub-item 4. The planner corrects the backlog row.

## 2. Tasks

- **T1 (console, D1).**
  - Test in `data-explorer-page.test.tsx`: at a width of 1280 the Owner and Tags filters are
    offered, while the columns are not shown.
  - *Accept:* `bun run typecheck`; `bun run lint`; `bun test src/features/catalog`.
- **T2 (Rust and SQL, D2).**
  - A test that a column re-registered by a second run is counted once (unit, or a route test
    with the ClickHouse fake the catalog tests use).
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api catalog`.
- **T3 (D3).** The workflow, plus any bench fix.
  - *Accept:* `actionlint` on the workflow; `python3 -m py_compile ops/bench/catalog_search_bench.py`;
    the bench's own tests, if any.

T1 can be built on its own. T2 and T3 follow on the same branch.

## 3. Handoff (developers append below)

## 4. Review and results (planner appends below)
