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
- **Searches:** 50 per route. The bench waits 35 s first, so the 30-second search copy is stale
  and the first search has to rebuild it.
- **Reported per route:** the first search, then p50, p95 and max over the other 49.

## Results

| Route | First search (rebuilds the copy) | p50 | p95 | Max |
| --- | --- | --- | --- | --- |
| `GET /api/catalog?q=` (command palette) | 7,153.9 ms | **489.4 ms** | **751.0 ms** | 791.4 ms |
| `GET /api/catalog/query?search=` (Data Explorer) | 7,292.9 ms | **325.8 ms** | **760.3 ms** | 771.1 ms |

## Against the target

- **The typical search meets 500 ms.** The median is 489 ms and 326 ms.
- **The slow searches do not.**
  - The 95th percentile is about 750 to 760 ms on both routes, about 1.5 times the target.
- **The first search after the copy expires takes about 7.2 s.** It rebuilds the whole search
  copy (`build_search_snapshot`: assemble the catalog, read 200,000 column rows, count use) while
  the person waits. With a 30-second copy, that happens to someone at least every 30 seconds
  under use.
- **So the target is met for the median only, not as a promise for every search.** The
  follow-up is `DATA-38`.

## What this does not cover

- One run on one runner. Search times on a customer's host will differ.
- No concurrent searches: they run one after another.
- The column-row read was measured **before** `DATA-24` T2 adds `FINAL` to it, which may cost
  some time. Re-run this workflow after T2 merges, and add a row.
