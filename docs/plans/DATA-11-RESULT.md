# `DATA-11` result: catalog search on 10,000 tables

The target (`docs/core/features/catalog-search.md`, decision 1) is results within 500 ms for a
catalog of 10,000 tables. It is measured here, and a miss is reported, not hidden.

## How it was measured

- **Run:** GitHub Actions run **41267430494**, workflow `catalog-search-bench.yml` (manual), on
  `main` at `d34a35fa`, on 2026-11-01. One GitHub-hosted `ubuntu-latest` runner (16 GB).
- **Stack:** the default Compose stack, built from that commit.
- **Command:** `python3 ops/bench/catalog_search_bench.py seed`, then `… run`. The workflow sets
  `BENCH_THROWAWAY_CATALOG=true` and `BENCH_CLICKHOUSE_URL`.
- **Catalog:**
  - **10,000 datasets with 20 columns each**: 5,000 in `bronze_meta` and 5,000 in
    `bronze_meta_sec`, 200,000 column rows in all;
  - the bench's log: "bronze_meta: 5000 datasets, 100000 columns" and "bronze_meta_sec: 5000
    datasets, 100000 columns".
- **Searches:** 50 per route. The bench waits 35 s first. No search comes before that wait, so
  the first search finds no copy and builds it (a cold start; see the `DATA-38` re-run below).
- **Reported per route:** the first search, then p50, p95 and max over the other 49.

## Results

| Route | First search (rebuilds the copy) | p50 | p95 | Max |
| --- | --- | --- | --- | --- |
| `GET /api/catalog?q=` (command palette) | 7,153.9 ms | **489.4 ms** | **751.0 ms** | 791.4 ms |
| `GET /api/catalog/query?search=` (Data Explorer) | 7,292.9 ms | **325.8 ms** | **760.3 ms** | 771.1 ms |

**Re-run after `DATA-24` T2** (#308, the column rows read with `FINAL`): run **41282255663**,
`main` at `d8635457`, 2026-11-01, same workflow and runner type.

| Route | First search | p50 | p95 | Max |
| --- | --- | --- | --- | --- |
| `GET /api/catalog?q=` | 4,766.1 ms | **369.1 ms** | **525.5 ms** | 539.2 ms |
| `GET /api/catalog/query?search=` | 426.1 ms | **252.4 ms** | **530.4 ms** | 5,156.7 ms |

- `FINAL` did not make search slower. The second run is faster on every figure, but one run per
  build on a shared runner cannot tell a code effect from runner noise, so no gain is claimed.
- p95 is now just over 500 ms on both routes.
- The rebuild of the search copy still costs about 5 s:
  - on the first route it was the first search;
  - on the second route it fell on a later search (the max of 5,156.7 ms), because the copy
    expired mid-run.
- That is still `DATA-38`.

**Re-run after `DATA-38`** (#314, the search copy rebuilt in the background): run
**41303508678**, `main` at `303c7b4d`, 2026-11-01, same workflow and runner type.

| Route | First search | p50 | p95 | Max |
| --- | --- | --- | --- | --- |
| `GET /api/catalog?q=` | 7,232.6 ms | **450.2 ms** | **680.5 ms** | 760.9 ms |
| `GET /api/catalog/query?search=` | 583.1 ms | **365.2 ms** | **633.9 ms** | 674.6 ms |

- **The first search is a cold start, not an expired copy.** This corrects the claim above and
  in "Against the target" that the first search "rebuilds the copy after it expires". The bench
  makes no search before its 35 s wait, so when the first search arrives there is no copy at
  all. That search builds the copy while the person waits, and it still does after `DATA-38`,
  by design (`DATA-38` D1: no copy, or one older than five minutes, is built inline). So the
  bench's "first search" has never measured what `DATA-38` changes.
- **What `DATA-38` changes shows on the second route.** The copy built by the first route's
  first search (about 12:08:40) passed its 30 s age as the second route started (12:09:10 to
  12:09:30, the bench's log). In the previous run the same moment cost one search 5,156.7 ms.
  This time the second route's slowest search was 674.6 ms: the expired copy was served at
  once, and rebuilt behind it. That is one run, so it is evidence, not a measured promise. A
  bench mode that searches, waits past the age, and times the next search would measure it
  directly. That is part of `DATA-41`.
- **p95 is still over 500 ms**, at 680.5 ms and 633.9 ms, about as before. It is the cost of
  each search on 10,000 tables, which `DATA-38` does not change. The target is not lowered. It
  moves to `DATA-41`.

## Against the target

- **The typical search meets 500 ms.** The median is 489 ms and 326 ms.
- **The slow searches do not.**
  - The 95th percentile is about 750 to 760 ms on both routes, about 1.5 times the target.
- **The first search after the copy expires takes about 7.2 s.** *(Corrected after the `DATA-38`
  re-run: this was the first search on a stack with no copy yet, a cold start, not an expired
  copy. See that re-run above.)* It rebuilds the whole search
  copy (`build_search_snapshot`: assemble the catalog, read 200,000 column rows, count use) while
  the person waits. With a 30-second copy, that happens to someone at least every 30 seconds
  under use.
- **So the target is met for the median only, not as a promise for every search.** The
  follow-up is `DATA-38`.
- **After `DATA-38`** (run 41303508678): the median still meets 500 ms (450 and 365 ms), p95
  still does not (680 and 634 ms), and an expired copy no longer costs a search its rebuild.
  The p95 target and the cold start are `DATA-41`.

## What this does not cover

- One run on one runner. Search times on a customer's host will differ.
- No concurrent searches: they run one after another.
- The column-row read was measured **before** `DATA-24` T2 adds `FINAL` to it, which may cost
  some time. Re-run this workflow after T2 merges, and add a row.
