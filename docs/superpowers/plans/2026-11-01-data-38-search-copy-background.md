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

### Handoff (developer, 2026-11-01)

**Commit.** `24a1b819` — `fix(catalog): the search copy is rebuilt in the background (DATA-38)`, on
`fix/data-38-search-copy-background` (base `main` at `2c110ebc`, plan commit `82d6251f`). Pushed to
`origin/fix/data-38-search-copy-background`.

**Files.** `rust/crates/lakehouse-api/src/catalog_search_cache.rs` (the cache, D1–D3),
`rust/crates/lakehouse-api/src/routes/catalog.rs` (the builder and `SEARCH_SNAPSHOT_TTL`'s comment,
D3–D4), `docs/core/features/catalog-search.md` (D4).

**What was built (D1–D4).**

- **D1.** Stale-while-revalidate, exactly the plan's table. Younger than the TTL: the copy is
  served. From the TTL to `MAX_STALE`: the copy is served at once and ONE `tokio::spawn`ed
  rebuild replaces it; a failed rebuild is logged (`tracing::warn!`), the old copy stays, and the
  next read after the TTL starts another. `MAX_STALE` or older, or no copy at all: built inline,
  single-flight, as before, and a failure caches nothing. `MAX_STALE` is the constant
  `catalog_search_cache::MAX_STALE` (five minutes), with its reason in its doc comment.
- **D2.** `invalidate` still drops the copy, so the next search rebuilds inline and a console edit
  shows at once.
- **D3.** The builder is `'static`: `get_or_build` takes the cache as `self: &Arc<Self>` (the
  `Arc<Self>` of D3, so the background task can put the new copy in) and a builder that owns what it
  needs. `AppState` IS `Clone` (`state.rs:74`), so the route hands the builder
  `Arc::new(state.clone())` and it owns a clone of `AppState`. The clock stays the `now` parameter,
  so the age rules are unit-tested without sleeping.
- **D4.** The module doc comment and `SEARCH_SNAPSHOT_TTL`'s comment say the new rule and cite
  `DATA-38`.

**Two decisions the plan left open, made here (for the review).**

1. *A background rebuild that raced with `invalidate`.* The plan does not say what happens when a
   rebuild is in flight and a console edit drops the copy. The slot carries a `generation` counter:
   `invalidate` (and an inline install) bumps it, and a background rebuild puts its copy in only if
   the generation is unchanged. So a copy that may have been read before the write cannot outlive
   it — the guarantee the old `invalidate` doc comment gave — without `invalidate` having to wait
   for the rebuild.
2. *`MAX_STALE` lives in the cache module, not next to `SEARCH_SNAPSHOT_TTL`.* The TTL stays a
   parameter (the route's freshness policy); `MAX_STALE` is a bound the cache enforces on itself, so
   no caller can widen it by accident.

**The feature page (D4).** D4 names "decision 1 if it states the freshness". Decision 1 is the
speed target and does not state the freshness; decision 2 does ("The copy is at most 30 seconds
old"). Decision 2 was updated instead, with a note under the table saying what `DATA-38` changed
(the promise is the same, the mechanism is not) and a new bullet under "Limits to tell a customer".
The signed column now reads "2026-10-09; `DATA-38` changed the mechanism on 2026-11-01". The
acceptance checklist is untouched: row 14's measurement is T2's.

**One wording difference.** `docs/core/BACKLOG.md` says "Rebuild the copy in the background before
it expires"; D1 starts the rebuild when the copy is found expired. The effect for the reader is the
same (nobody waits for a rebuild), and D1 was implemented as decided.

**Commands run (foreground, from `rust/`, through `/home/shiro/rantai/bin/cargo-locked`).**

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean, no output |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | finished, 0 warnings |
| `cargo test -p lakehouse-api catalog_search_cache` | 10 passed, 0 failed (the module is compiled into the lib and the bin, so 10 + 10) |
| `cargo test -p lakehouse-api catalog` | 554 passed, 0 failed, 0 ignored, over 75 test binaries (270 in the lib, 272 in the bin) |

**Not verified, and why.**

- The full workspace block (`cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace`) was NOT run: the machine rules for this task say to run only the scoped
  commands the plan lists and no workspace run. CI runs the workspace block on the PR.
- The frontend, Python and compose checks were NOT run: this change touches Rust and Markdown only.
- T2 (re-running `catalog-search-bench.yml`, and the p95 after this change) was NOT done: it is the
  planner's task, after the merge. No p95 or latency number is claimed here.
- The behaviour on a real deployment (a person searching while the copy is rebuilt) was NOT run: no
  stack was started for this task, and the acceptance checklist belongs to the product owner.

**The testcontainers the sqlx tests started (`epic_herschel`, `postgres:16-alpine`) were removed
afterwards.**

## 4. Review (planner appends below)
