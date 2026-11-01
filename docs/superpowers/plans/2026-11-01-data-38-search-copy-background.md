# DATA-38: catalog search does not wait while its copy is rebuilt

Backlog `DATA-38` (Task, Data, P2), from `docs/plans/DATA-11-RESULT.md`. Base: `main` at
`2c110ebc`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code disagrees,
stop and report it.

## 0. What is wrong (measured)

- **The rebuild stalls a search.** On 10,000 tables, rebuilding the search copy takes about 5 s:
  4.8 s and 5.2 s in run 41282255663, and 7.2 s before the `FINAL` fix.
- **Someone waits for it every 30 s.**
  `SearchSnapshotCache::get_or_build` (`rust/crates/lakehouse-api/src/catalog_search_cache.rs:79`)
  drops an expired copy and rebuilds it inline (`:89-101`). The 30-second TTL is
  `SEARCH_SNAPSHOT_TTL` (`routes/catalog.rs:682`). Under use, a person waits for the rebuild
  once every 30 s.
- **p95 is just over target.** The warm p95 is 526 and 530 ms against the 500 ms target. Whether
  that comes from the ranking itself is not known yet (see T2).

## 1. Decisions

- **D1. Stale-while-revalidate.** The cache keeps serving an expired copy while ONE background
  task rebuilds it.

  | Case | What happens |
  | --- | --- |
  | Age < `ttl` (30 s) | Serve it. |
  | `ttl` ≤ age < `max_stale` (5 min) | Serve it at once. If no rebuild is running, start one with `tokio::spawn`. When it succeeds, swap the new copy in. When it fails, log it, keep the old copy, and let the next request after `ttl` start another. |
  | Age ≥ `max_stale`, or no copy at all (the first search, or after `invalidate`) | Build inline, single-flight, as today. A failure caches nothing. |

  So no copy older than 5 minutes is ever served (`max_stale` is a constant, with its reason in
  the doc comment). Normally a search sees data at most 30 s plus one rebuild old.
- **D2. `invalidate` still drops the copy**, so the next search rebuilds inline. A console edit
  must show at once (`DATA-11` D2), and edits are rare.
- **D3. The builder becomes `'static`.** `get_or_build` takes a builder that can be moved into the
  spawned task: an `Arc<Self>` and a closure that owns a clone of `AppState`. Check that
  `AppState: Clone`; if it is not, stop and report.
  - The clock stays a parameter, so the tests do not sleep.
  - Only one background rebuild may run at a time: use a flag in the slot.
- **D4.** Update the module doc comment and `SEARCH_SNAPSHOT_TTL`'s comment, and
  `docs/core/features/catalog-search.md` decision 1 if it states the freshness. Cite `DATA-38`.

## 2. Tasks

- **T1 (D1–D4).** Unit tests, each with a full-sentence name:
  - a fresh copy is served;
  - an expired copy is served at once, and one rebuild starts;
  - two expired reads start one rebuild, not two;
  - a successful rebuild swaps the copy in;
  - a failed rebuild keeps the old copy, and the next read after `ttl` retries;
  - a copy past `max_stale` is rebuilt inline;
  - after `invalidate`, the next read rebuilds inline;
  - the first-ever read builds inline.

  *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api catalog_search_cache`; `cargo test -p lakehouse-api catalog`.
- **T2 (planner, after merge).** Re-run `catalog-search-bench.yml` and add a row to
  `DATA-11-RESULT.md`. If p95 is still over 500 ms, the planner opens a follow-up that profiles
  the ranking. That is not this item.

## 3. Handoff (developer appends below)

## 4. Review (planner appends below)
