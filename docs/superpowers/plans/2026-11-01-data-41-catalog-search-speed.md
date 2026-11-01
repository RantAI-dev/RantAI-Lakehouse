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

### Handoff (developer, 2026-11-01)

T2, T3, T4 and T5 are done, in the plan's order, one commit each. **T1 is
not done**: `DATA-40` (`fix/data-40-one-name-one-dataset`) is not in `main`
yet, and the plan puts T1 last, after `DATA-40` has merged. The branch is
merged with `origin/main` (`git merge origin/main` → "Already up to date";
`main` is at `5661a043`). The check was
`git merge-base --is-ancestor origin/fix/data-40-one-name-one-dataset
origin/main`, which fails.

#### Commits (branch `perf/data-41-catalog-search-speed`, pushed after each)

| Commit | Task | Decision | What it does |
| --- | --- | --- | --- |
| `45dd7745` | T2 | D2 | `SearchIndex`: the candidates are lower-cased and tokenised once, with the copy; `search()` reads the index and allocates nothing per candidate. |
| `29171a7f` | T3 | D6 | `apply_filters` filters in place; a search that filters nothing copies nothing. |
| `f63a32e2` | T4 | D3 | `GET /api/catalog?limit=` (1..200) and `total`; the palette asks for eight. |
| `f3c68cd6` | T5 | D4, D5 | The copy is built at start-up, in the background; the bench's `--after-expiry` measures the search that finds the copy too old. |

#### Checks run (and their counts)

Scoped to the crate, per the task's rules — no full-workspace cargo run.

| Command | Result |
| --- | --- |
| `cargo fmt --check` (in `rust/`) | clean, no output |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean, no warnings |
| `cargo test -p lakehouse-api --lib catalog_search` (T2) | 39 passed, 0 failed |
| `cargo test -p lakehouse-api --lib catalog_query` (T3) | 39 passed, 0 failed |
| `cargo test -p lakehouse-api --lib catalog` | 282 passed, 0 failed |
| `cargo test -p lakehouse-api catalog` (T4) | 279 + 281 passed, 0 failed (the lib and the bin target) |
| `cargo test -p lakehouse-api --lib catalog_search_cache` (T5) | 12 passed, 0 failed |
| `bun run typecheck` (T4) | clean, no output |
| `bun run test` (T4) | 3083 pass, 1 skip, 0 fail (313 files) |
| `python3 -m py_compile ops/bench/catalog_search_bench.py` (T5) | clean |
| `python3 -m unittest discover -s ops/bench -p 'test_*.py'` (T5) | 18 tests, OK |
| `actionlint .github/workflows/catalog-search-bench.yml` (T5) | clean, no output |
| `cargo test -p lakehouse-api` (the whole crate, extra) | 91 test binaries, 6823 passed, 0 failed, 42 ignored |

#### The bench on bluefin (2026-11-02, this branch, throwaway compose project `data41bench`)

The same throwaway setup the investigation used: `docker compose -p
data41bench up -d --build lakehouse-api`, the bench's own `seed` (10,000
tables × 20 columns, half in each registry), then `run --rounds 50
--datasets 10000 --after-expiry`. The stack was torn down afterwards
(`docker compose -p data41bench down -v`).

| Route | p50 | p95 | max | first | afterExpiryMs |
| --- | --- | --- | --- | --- | --- |
| `GET /api/catalog?q=` | 199.7 ms | **270.8 ms** | 293.3 ms | 12223.0 ms | 149.5 ms |
| `GET /api/catalog/query?search=` | 98.1 ms | **151.7 ms** | 167.0 ms | 149.0 ms | 159.4 ms |

Both routes are under the 500 ms target at p95 on this branch (the
investigation measured 678–739 ms at p95 on `main`, on this machine, with
the same setup).

The 12223 ms `firstMs` of the first route is the copy being rebuilt
inline: the bench ran about seven minutes after the stack came up, and a
copy older than `MAX_STALE` (five minutes) is not served — it is rebuilt
and the caller waits. It is the cost of the copy, not of a search; every
search after it is 150–270 ms.

#### Notes for the reviewer

- **T3's shape differs from the plan's wording, on purpose.** The plan says
  `apply_filters` should pass the matches through "by reference, or by an
  index list". It filters the rows **in place** instead: with no filters it
  touches no row at all, and with filters the rows that do not match are
  dropped. That is the same goal (no copy) with a smaller diff, and it
  avoids a `Cow` whose `DerefMut` panics on a borrowed value. The route
  therefore sorts the rows it holds **before** it filters them, which
  leaves the survivors in the same order as filtering first and sorting
  after would (`apply_sort` is a total order, `id` included).
- **The index shares one prepared text per distinct lower-cased text.** A
  column name is on thousands of tables, so preparing it per asset would
  cost memory in proportion to the catalog's assets rather than its
  distinct texts. `the_index_prepares_each_text_once_and_shares_it` pins
  it.
- **`routes::catalog_search` is `pub(crate)` now** (the cache module holds
  the index, so the type has to be nameable from outside the module).
  `routes::catalog` is `pub(crate)` for the same reason (`main.rs` starts
  the start-up build).
- `spawn_startup_search_copy` carries `#[allow(dead_code, reason = ...)]`:
  the binary starts the copy, and the library target compiles the same
  module without a start-up path of its own.
- `bun run lint` is **red on `main` too**: 12356 problems, 649 errors, all
  of them inside the local `dagster/.venv` (a gitignored virtualenv with
  bundled JS). The four files this branch touches are lint-clean
  (`bun --bun eslint <the four files>` → no output). Not caused by this
  branch, and not fixed by it.

#### Not verified

- **The full-workspace cargo run** (`cargo clippy --workspace --all-targets
  --all-features` and `cargo test --workspace`): not run. The task scoped
  the checks to the crate this branch touches. The whole `lakehouse-api`
  crate was run instead: `cargo test -p lakehouse-api` — 91 test binaries,
  6823 passed, 0 failed, 42 ignored.
- **T1** — blocked on `DATA-40` (see the top of this entry).
- The bench numbers above are from **this branch on bluefin**, not from the
  GitHub workflow. The planner's measurement (on `main`, after the merge)
  is the one that decides whether `DATA-41` is Done.

## 5. Review (planner appends below)

## 6. The investigation report (bluefin, 2026-11-01)

Kept in the planner's thread. Its numbers are quoted in section 0, and its `file:line` anchors
were checked against `5661a043` when this plan was written.
