# DATA-41: catalog search under 500 ms at p95, and no cold start

Backlog `DATA-41` (Task, Data, P2). Base: `main` at `5661a043`. Planner: Claude Opus. Developer:
another agent (`AGENTS.md`). Target: `docs/core/features/catalog-search.md` decision 1, results
within 500 ms for 10,000 tables. The target is not lowered.

## 0. What was measured (bluefin, 2026-11-01, 10,000 tables × 20 columns)

The investigation report is pasted under section 6. In short:

- **p95** is 678–739 ms on bluefin, against 680 ms on the GitHub run 41303508678, so it is a code
  cost and not runner noise. A search is single-threaded CPU, with 5 Postgres statements
  (≈1–2 ms, auth) and no ClickHouse query.
  - **≈190 ms is the scan.** `catalog_search::search` (`catalog_search.rs:342`) rebuilds every
    asset's candidate list per search (`candidates()` `:173`). That is ≈450,000 lowercased
    `String`s and a `Vec<char>` per token (`:285-287`). Each word that still matches adds
    ≈180 ms more.
  - **≈146 ms is cloning and serialising every match.** `GET /api/catalog?q=` returns all
    10,000 matches, 6.6 MB, but its only caller, the command palette, keeps 8
    (`src/lib/palette-search.ts:15`). `/api/catalog/query` pages to 50 rows, but
    `apply_filters` deep-clones every match first (`catalog_query.rs:405`).
- **The cold start, and the live list, are quadratic.** In `list_body` (`catalog.rs:503`),
  `total_of`, `author_of`, `col_of` and `frequency_of` (`:541-565`) each `.find()` across all
  10,000 rows once per asset. The copy build therefore takes ≈9.2 s at 10,000 tables, as
  `n^1.9`. `GET /api/catalog` with no term (the lineage page and the Query Studio schema
  browser) runs that build on **every request**: 8.7–10.7 s.
- `DATA-38` works: a search 35 s after the copy was built took 372.8 ms.

## 1. Decisions (planner, 2026-11-01)

- **D1. Lookups by key, not by scan.** `list_body` builds one `HashMap` per registry read
  (sync rows and column counts), keyed by slug, once per assembly. The four closures become map
  lookups, so the build is O(n). The assembled rows must be byte-identical to today's for the
  same input, which a test proves.
- **D2. The search index is built once, with the copy.** `CatalogSearchSnapshot`
  (`catalog_search_cache.rs:59`) gains a prepared index, built in `build_search_snapshot`: per
  asset, its candidates already lowercased and tokenised. `search()` takes it and allocates
  nothing per candidate. The matching rules do not change. Every existing test in
  `catalog_search.rs` keeps passing unchanged, and they are the proof: `DATA-11` D3, `DATA-32`
  D1 and the review fixes.
- **D3. The palette asks for what it shows.**
  - `GET /api/catalog` gains an optional `limit` (1 to 200, the Data Explorer's
    `MAX_PAGE_SIZE`). With `q` and `limit`, the route ranks every match as today and returns the
    first `limit`. It also gains `total`, the number of matches, so nothing is lost.
  - Without `limit` the answer is as today, so no other caller changes.
  - The client (`src/services/clients/assets.ts:43-46`) passes `limit` through, and the
    palette sends `PALETTE_ASSET_RESULT_LIMIT`.
  - `capPaletteAssetResults` stays as the second guard.
  - Contract order: contract, client, `services/index.ts`, feature (`AGENTS.md`).
  - `limit` outside 1..200 is a 400 with a plain message.
- **D4. No copy built under a person after start-up.** Once the API's database is up, a
  background task builds the copy, using the `'static` builder of `DATA-38` D3. A search that
  arrives first still builds inline, single-flight, as today. If the start-up build fails, it is
  logged, and the first search builds instead.
- **D5. The bench measures `DATA-38` directly.** `ops/bench/catalog_search_bench.py run` gains
  `--after-expiry`. It makes one search, waits past the copy's 30 s age, then times the next
  search, and reports it as `afterExpiryMs` beside the existing numbers. The workflow passes it.
- **D6. `apply_filters` does not clone when there is nothing to filter.** With empty filters it
  passes the matches through by reference, or by an index list. That is `catalog_query.rs:403`
  and its caller `catalog.rs:429`.

## 2. Tasks (one commit each, in this order)

T1 changes `list_body` in `routes/catalog.rs`, which `DATA-40` (branch
`fix/data-40-one-name-one-dataset`) also changes. **T1 therefore comes last.** Before T1, run
`git fetch origin && git merge origin/main`. If `DATA-40` has not merged into `main` yet, stop
after T5, hand off, and say so. The planner then sends T1 after `DATA-40` merges.

- **T2 (D2). The prepared index.**
  - Add the index type and its build in `catalog_search.rs` and `catalog_search_cache.rs`, and
    rework `search()` to use it.
  - Add a `#[cfg(test)]` timing guard only if it is not flaky. Prefer a test that counts
    allocations, or none: the bench is the measurement.
  - *Accept:* every existing `catalog_search` test passes unchanged;
    `cargo test -p lakehouse-api catalog_search`; clippy for the crate.
- **T3 (D6). No clone without filters.**
  - *Accept:* `cargo test -p lakehouse-api catalog_query`.
- **T4 (D3). `limit` and `total`.**
  - Change the route, with route tests: the limit is applied after ranking, `total` is the full
    count, there is a 400 outside 1..200, and the answer without `limit` is unchanged.
  - Change the contract, the client and the palette, with a Vitest test that the palette sends
    the limit.
  - *Accept:* `cargo test -p lakehouse-api catalog`; `bun run typecheck && bun run lint && bun
    run test`.
- **T5 (D4, D5). Build at start-up, and the bench mode.**
  - Spawn the start-up build where the API brings its database up, with a test that the copy
    exists after start-up without a search.
  - Add the bench flag and pass it in the workflow (`catalog-search-bench.yml`).
  - *Accept:* `cargo test -p lakehouse-api catalog_search_cache`; `python3 -m py_compile
    ops/bench/catalog_search_bench.py`; `actionlint` on the workflow.
- **T1 (D1). Lookups by key.** After `DATA-40` is in `main`.
  - Add a test that the assembled rows are identical to the scan's for a fixture of several
    hundred assets, with duplicates and missing sync or column rows.
  - *Accept:* `cargo test -p lakehouse-api catalog`.

**The measurement (planner, after merge):** the planner runs `catalog-search-bench.yml` on
`main` and records p50, p95, max, first and `afterExpiryMs` in `docs/plans/DATA-11-RESULT.md`.
DATA-41 is Done only if p95 is under 500 ms on both routes. Otherwise the gap is reported.

## 3. Out of scope

- Changing the matching rules, or adding a search engine.
- Paging the no-term `GET /api/catalog` list. After D1 it is O(n). If it is still slow, that is
  a follow-up.
- Concurrency of searches.

## 4. Handoff (developer appends below)

## 5. Review (planner appends below)

## 6. The investigation report (bluefin, 2026-11-01)

Kept in the planner's thread. Its numbers are quoted in section 0, and its `file:line` anchors
were checked against `5661a043` when this plan was written.
