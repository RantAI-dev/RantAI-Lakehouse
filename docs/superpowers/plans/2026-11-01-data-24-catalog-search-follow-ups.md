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

### Handoff T1 (developer, 2026-11-01)

Commit `3c643ceb` — "fix(data): keep the Owner and Tags filters at a narrow window (DATA-24 T1)" — on `fix/data-24-catalog-search-follow-ups`, on top of `d7745735` (the plan commit). One commit, console only.

What it does:

- The Data Explorer no longer drops the columns that do not fit the window from the table's column list. They are hidden through the table's visibility instead, so their filters keep working in the toolbar's filter menu at every width (decision D1). The thresholds are unchanged (`1760`, `1500`, `1280`, `1100`, `950`, `860`).
- The hide covers **every** column the window is too narrow for — Tags, Owner, Size, Freshness, Type, Layer — not only the two the report named. D1 is "hide, do not remove", and the note under the table now says "The filters still work", which is only true if every hidden column keeps its filters. Narrowing it to Owner and Tags is a one-line change in `data-explorer-page.tsx` if the reviewer wants that; the mechanism is the same either way.
- `useDataTable` takes `hiddenColumns` (`src/hooks/use-data-table.ts`). The page's hides ride on top of the stored layout instead of replacing it, and `keepStoredVisibility` leaves the user's stored choice for those columns alone when the layout is written — so a resize cannot overwrite it, and a wider window brings back what the user had.
- The settings menu leaves a page-hidden column out of its "View" list: it is not the user's to toggle, it comes back with the width.
- The "columns hidden" note now ends with "The filters still work."

Checks run in the foreground on the final commit (`3c643ceb`):

| Command | Result |
| --- | --- |
| `bun run typecheck` | clean, no output |
| `bun run lint` | 0 errors, 10 warnings — the same 10 as on `d7745735` (checked by re-running the lint with the change taken out) |
| `bun test src/features/catalog` | 341 pass, 0 fail, 1095 expect() calls, 16 files |
| `bun run test` (the whole console suite) | 3063 pass, 1 skip, 0 fail, 8336 expect() calls, 313 files |

New test file `src/features/catalog/data-explorer-page.test.tsx` (5 tests):

- at 1280px the Owner and Tags columns are not on screen;
- at 1280px the filter picker still offers the Owner and Tags filters;
- at 1800px the Owner and Tags columns are on screen;
- the note says the filters still work;
- the window's hide is not written into the saved layout — hiding a column the user can see stores that choice and leaves `owner`/`tags` out of `app:table-layout:/data`.

Checked that the tests bite: against the pre-change code (`d7745735`) 2 of the 5 fail — "offers the Owner and Tags filters at 1280px all the same" and "says the filters still work when it hides a column". The saved-layout test fails too when `keepStoredVisibility` is taken out (1 of 5), so the guard is pinned.

Not verified, and why:

- **Rust, Python and compose checks were not run.** T1 touches the console only; per AGENTS.md a commit runs the checks for the languages it touched. T2 (Rust, SQL) and T3 (workflow, bench) are untouched by this commit.
- **The widths themselves are not verified.** The thresholds are the estimates already in the file (derived from the column caps, not measured); this change does not move them, and the plan does not ask for a measurement.
- **One consequence to be aware of.** The sort/filter whitelist in `useDataTable` is built from the table's column ids, which now always include the columns the window hides. A hand-written `?sort=tags` (the UI never offers sorting by tags — the column has `enableSorting: false`, and the server refuses to sort by a list) is therefore forwarded to the server at every width and answered with a 400, where before a narrow window dropped it silently. At a wide window this was already the case, so the behaviour is now the same at every width instead of different. Not changed here: the whitelist is shared by every table and is out of T1's scope.

## 4. Review and results (planner appends below)
