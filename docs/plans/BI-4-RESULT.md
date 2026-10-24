# BI-4 result: parallel tiles (part A), remembered results (part B pending)

Plan: `docs/superpowers/plans/2026-10-24-bi-4-caching-parallel-tiles.md`
(tasks T1, T2). Feature page: `docs/core/features/result-caching-and-parallel-tiles.md`
(decisions 3 and 9). Written by the developer; the planner reads and signs.
This file states what was measured, how, and what was not. It states no
verdict on the 3-second target: that target is for a warm cache (part B), and
no real-engine number exists yet.

## 1. What was measured: a mock engine, in-process

**Not a dashboard on a real ClickHouse.** This is the only timing taken so
far, and it shows the scheduling, not the speed of a real board.

- Test: `routes::dashboard::typed_filters::twelve_slow_tiles_load_six_at_a_time_in_order_and_one_failure_stays_its_own`
  (`rust/crates/lakehouse-api/src/routes/dashboard.rs`).
- Method: `get_body` over a `wiremock` ClickHouse. Twelve stored charts; every
  tile query is answered after a fixed 300 ms delay; the chart `c08` is
  answered with HTTP 500. The test asserts the chart order of the payload, that
  `c08` has an `error` and the other eleven have rows, that the first six tile
  queries arrive within 200 ms of each other, and that the whole call takes
  under 1.5 s.
- Command and output (one run, `--test-threads=1`, this VM, debug build):

  ```
  cargo test -p lakehouse-api --lib -- twelve_slow_tiles --test-threads=1
  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2520 filtered out; finished in 0.70s
  ```

- Reading: eleven tiles answered at 300 ms each take 3.3 s when run one after
  another (arithmetic, not a run of the old code); the call took 0.70 s,
  which is two rounds of six. `tile_pool::tests` show the same with a paused
  clock (twelve tiles in exactly 600 ms, peak 6 in flight; two boards of 12
  under 8 process slots peak at 8).
- The "before" is not a measurement: the old sequential loop no longer exists
  in this branch, and the test was not run against it.

## 2. What was NOT measured

- A real 12-tile dashboard on ClickHouse, before (`feat/bi-phase-4`) and after
  (this branch): this needs two API builds deployed against the same data, and
  the shared stacks on this machine run other branches. The script for it
  exists (`ops/bench/dashboard_open_bench.py`, T2) and has not been run
  against anything. Machine, data size and ClickHouse version are therefore
  not recorded.
- Warm-cache open time and the 3-second target: part B.
- Behaviour under real engine limits (connection pool, `max_concurrent_queries`)
  with 24 tiles in flight per process.
- The `system.parts.modification_time` check of the plan's section 5: part B.

## 3. How to take the real number

```
API_URL=http://<api> BENCH_COOKIE='<session cookie header>' \
  python3 ops/bench/dashboard_open_bench.py --board <12-tile board id> --rounds 20
```

Run once on `feat/bi-phase-4` and once on this branch, same stack data, quiet
machine, and paste the raw JSON line here with the machine, data size and
ClickHouse version.
