# BLD-2 Target Lag — measured gate result

The D11 measurement for backlog item BLD-2 (an authored pipeline that
refreshes itself within a target lag), run as the `g3a-dagster` gate
sequence from a clean compose project. This file is rewritten from the
run's own log only (review round 1, BLOCKER 9); every number below is a
quoted line of that log.

## The run

| | |
| --- | --- |
| Commit | `47fcf9ad4618` (branch `feat/bld-2-target-lag`) |
| Date (UTC) | 2026-10-16T20:32:16+00:00 (run start, from the log's first line) |
| Compose project | `bld2r6q`, created for this run and removed with `down -v` |
| Full output | `/home/shiro/rantai/review/bld2-gate-r6.log` |
| Env file | `/home/shiro/rantai/bld2-gate.env` (outside the repository; CI-only throwaway values) |

```bash
# images built from THIS head, then the two gate runners, then teardown
docker compose --env-file /home/shiro/rantai/bld2-gate.env -p bld2r6q --profile dagster \
  build lakehouse-api dagster-code-location oidc-mock g7-test-runner
docker compose --env-file /home/shiro/rantai/bld2-gate.env -p bld2r6q --profile dagster \
  up -d --no-build lakehouse-api dagster-code-location dagster-webserver dagster-daemon \
  g3a-source-init clickhouse-iceberg-init
docker compose --env-file /home/shiro/rantai/bld2-gate.env -p bld2r6q --profile dagster run --rm g3a-test-runner
docker compose --env-file /home/shiro/rantai/bld2-gate.env -p bld2r6q --profile dagster run --rm g7-test-runner
docker compose --env-file /home/shiro/rantai/bld2-gate.env -p bld2r6q --profile dagster down -v
```

The whole sequence ran under `flock /home/shiro/rantai/.heavy.lock`, and
the runner both teed its output and had it redirected, so every line
appears twice in the log; the two copies are identical.

Fixture, as the log states it:

```
[g7] authored pipeline 'pl-g7-dedupe-select-mvbfs0km' created and activated
[g7] silver.g7_dedupe_select_out holds 2000 rows = the 2000 distinct ids of bronze.g3a_orders
[g7] configuring targetLag=2 minutes on pl-g7-dedupe-select-mvbfs0km
[g7] confirmed sensor authored__pl_g7_dedupe_select_mvbfs0km_lag is RUNNING
[g3a] PASS
[g7] PASS
```

## 1. Measured lag (Bronze commit -> Silver rows), 2-minute target

Each sample inserts one order into the Postgres source, lets
`bronze_ingest_job` load it, and stops the clock when the Silver table
holds the expected new row count. Quoted:

```
[g7] TARGET_LAG_SAMPLE_1: Bronze commit -> Silver reached 2001 rows in 77.20s (...)
[g7] TARGET_LAG_SAMPLE_2: Bronze commit -> Silver reached 2002 rows in 95.13s (...)
[g7] TARGET_LAG_SAMPLE_3: Bronze commit -> Silver reached 2003 rows in 68.90s (...)
[g7] TARGET_LAG_SAMPLE_4: Bronze commit -> Silver reached 2004 rows in 71.11s (...)
[g7] TARGET_LAG_SAMPLE_5: Bronze commit -> Silver reached 2005 rows in 89.10s (...)
[g7] TARGET_LAG_RESULT: 5 samples = [77.2, 95.13, 68.9, 71.11, 89.1]s, avg = 80.29s
[g7] TARGET_LAG_RESULT: ALL 5 SAMPLES MET THE 2-MINUTE TARGET (<= 120s)
```

| Sample | Rows reached | Elapsed | Target (120 s) met? |
| :---: | :---: | :---: | :---: |
| 1 | 2001 | 77.20 s | yes |
| 2 | 2002 | 95.13 s | yes |
| 3 | 2003 | 68.90 s | yes |
| 4 | 2004 | 71.11 s | yes |
| 5 | 2005 | 89.10 s | yes |

Worst sample **95.13 s**, average **80.29 s**, both from the log's own
`avg = ` line. The gate FAILS the job when any sample exceeds the target
(review BLOCKER 8); this run's gate returned 0 and printed `[g7] PASS`.

## 2. Unchanged inputs start no runs

```
[g7] baseline run count before unchanged checks: 7
[g7] UNCHANGED_CHECK_1/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_2/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_3/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_4/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_5/5: status=met, runs=7 (no new run)
```

Five checks, one per check interval, zero new runs, `met` throughout.

## 3. The two earlier runs, and why the floor is 2 minutes (R6b, R6c)

Both earlier numbers are the reviewer's round-1 gate runs, quoted in the
plan's decision R6. They are recorded here so the floor is shown as
measured rather than chosen; what survives on disk is named for each.

| Run | Commit | Target | 5 samples (s) | Worst | Evidence on disk |
| --- | --- | --- | --- | --- | --- |
| Round 1, before R6a | the head before `9a59250c` | 60 s | 40.70, 68.94, 56.85, 56.90, 52.82 | 68.94 | no log survives; quoted in the plan's R6 |
| Round 1, before R6a | `9a59250c` | 120 s | 103.33, 111.42, 121.44, 123.77, 123.76 | 123.77 | `/home/shiro/rantai/review/bld2-gate-r1.log`, header `commit 9a59250c77ec, project bld2r1f, 2026-10-16T20:01:03+00:00`; the file was cut when the reviewer stopped the run and holds only its first sample (103.50 s) |
| This run | `47fcf9ad4618` | 120 s | 77.20, 95.13, 68.90, 71.11, 89.10 | 95.13 | `/home/shiro/rantai/review/bld2-gate-r6.log` (section 1) |

The lag grew with the target (68.94 s at 60 s, 123.77 s at 120 s),
because the budget left out the queue and launch a requested run waits
through, so every batched run landed late by about that gap. R6a makes
the budget the median `creationTime -> endTime` plus two check
intervals; with that in place the 2-minute target is met by every sample
with 24.87 s of headroom, which is the floor D11 asks for and is **not**
raised further in this PR (R6b).

## 4. What this measurement does and does not cover

- **The 30 s measurement caveat (plan 6.1).** The lag the pipeline
  measures starts at the check that FIRST saw the input change, so a
  sample reads up to one check interval low against the stopwatch, which
  starts at the Bronze commit.
- **The timer caveat (review SHOULD-FIX 15).** `commit_t` is taken after
  the ingest run was polled to `SUCCESS` (2 s poll granularity), so each
  sample EXCLUDES that run's own duration and reads low by it. The gate
  prints this on every sample line. The real end-to-end time from "the row
  was committed to Postgres" to "the row is in Silver" is therefore
  LONGER than the numbers in section 1 by the ingest run's duration.
- One stack, one machine, one pipeline shape (Bronze `Iceberg` ->
  Silver `ClickHouse`, two transforms, ~2000 rows). Not a claim about
  other zones, table sizes or hardware.
- Not verified in a browser by the developer: the console's Freshness
  panel and the list's Lag column.

## 5. What the content fingerprint costs (review SHOULD-FIX 22)

Handoff 5.2 claimed `count()` + `sum(cityHash64(*))` runs in "< 10 ms"
on the largest Silver table in the seed, with no measurement. Measured
at the end of the same run, on the largest Silver table the gate left
behind, with the SQL `mart_content_fingerprint` itself builds:

```
[fingerprint] table = `silver`.`g7_dedupe_select_out`
2005	9088869826723785630
0.006
2005	9088869826723785630
0.007
```

That is `clickhouse-client --time` twice on a 2005-row, two-column table:
6 ms and 7 ms. It is a full scan (`count()` plus `sum(cityHash64(*))`,
not `max(_content_hash)`), and on a table of this size it is cheap; the
claim holds for this size only and is not evidence about a large mart.