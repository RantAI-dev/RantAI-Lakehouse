# BI-4 result: parallel tiles (part A) and remembered results (part B)

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

## 4. Part B: remembered results

Date: 2026-10-25. Engine: a throwaway `clickhouse/clickhouse-server:26.8` container
(version `26.8.9.10`, 2 GB, 1 CPU, `default` user), driven with `clickhouse-client`; removed
afterwards. Script kept outside the repo.

### 4.1 The plan's section 5 check: does `system.parts.modification_time` move?

Fingerprint statement (the shape `result_cache::read_prints` sends, here with a literal table
list): `SELECT database, table, toString(max(modification_time)), toString(sum(rows)) FROM
system.parts WHERE active AND database = 'serving' AND table IN ('t','v','g') GROUP BY database,
table`. Raw output:

```
empty table                     (no row: a table with no active part is absent)
after insert 1 (10 rows)        serving t 2026-10-24 20:16:53 10
after insert 2 (5 rows)         serving t 2026-10-24 20:16:56 15
g created (CTAS, 15 rows)       serving t ... 15 / serving g 2026-10-24 20:16:58 15
g CREATE OR REPLACE (7 rows)    serving g 2026-10-24 20:17:00 7
g CREATE OR REPLACE (same 7)    serving g 2026-10-24 20:17:03 7
view serving.v                  count() of system.parts rows for it: 0
fingerprint statement time      0.179 s wall (including `docker exec` start-up)
```

Result: **yes**. The time moves on every insert and on a `CREATE OR REPLACE TABLE ... AS
SELECT` (the Gold build's statement), even when the new table has the same rows. A view is not
in `system.parts`, so a result that reads one is "unwatchable" and falls back to the
10-minute bound. Limits found: the time has one-second resolution, so a change that leaves
both the newest part's second and the row count as they were (a replace with the same number
of rows inside the same second as the previous write) is not seen until the next change; and a
table with no active part is absent, which the cache reads as unwatchable (10 minutes), not as
empty.

### 4.2 What was NOT measured (not verified)

- The warm-cache open time of a 12-tile dashboard and the **3-second target**: the API binary
  was not built or run on this machine (disk and memory; AGENTS.md), no console, no data of
  the size a customer has. The target is **neither met nor failed**. The in-process facts that
  stand behind it are the unit tests in `result_cache::tests` and
  `routes::dashboard::time_grain::a_second_open_is_remembered_and_fresh_computes_again`
  (a second open is served without a tile query; only the one fingerprint read goes to the
  engine). Take the number with `ops/bench/dashboard_open_bench.py` (section 3) twice on the
  same board, first open then second open, and paste the raw output here.
- That `referenced_tables` on the rewritten statement names the physical tables when masking
  wraps them: the cache reads them from the same rewritten statement the engine runs, and a
  table it cannot find in `system.parts` is treated as unwatchable (10 minutes), so a wrapping
  can only shorten a result's life, never lengthen it. **Correction (BI-4 B review P-S1):**
  that holds only for data read through a `FROM`/`JOIN` factor. Data read through
  `dictGet*`/`joinGet*` or `IN <table>` names no table factor, so the fingerprint could not
  see it; since the review such a statement gets an unwatchable print of its own (10 minutes
  or the person's minutes), and a statement that calls a clock or random function is also
  never served across a calendar day in the report time zone (P-B1). Whether a masked tile
  reaches the watchable path was not checked against a real policy.
- `cargo deny check` and `cargo audit` for the `moka` dependency: not run (no tool on this
  machine). `moka` 0.12.16 with only the `future` feature was already in `Cargo.lock` through
  `iceberg`; the lockfile change is the one line that adds `moka` to `lakehouse-api`'s
  dependency list, no new crate.
