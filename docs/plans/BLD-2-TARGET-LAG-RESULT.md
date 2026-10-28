# BLD-2 Target Lag — measured gate result

The D11 measurement for backlog item BLD-2 (an authored pipeline that
refreshes itself within a target lag), run as the `g3a-dagster` gate
sequence from clean compose projects. This file is rewritten strictly
from surviving run logs only (review round 2, BLOCKER 4); every number
below is a quoted line or statistic from `/home/shiro/rantai/review/bld2-gate-r1.log`,
`/home/shiro/rantai/review/bld2-gate-r6.log`, `/home/shiro/rantai/review/bld2-gate-r8-1min.log`,
or `/home/shiro/rantai/review/bld2-gate-r8-final.log`.

## 1. Final Acceptance Gate Run (1-minute target floor)

| | |
| --- | --- |
| Commit | `29d889340b1a` (branch `feat/bld-2-target-lag`) |
| Date (UTC) | `2026-10-17T01:43:18+00:00` (run start, from log header) |
| Compose project | `bld2r8fin`, created for this run and removed with `down -v` |
| Full output | `/home/shiro/rantai/review/bld2-gate-r8-final.log` |
| Env file | `/home/shiro/rantai/bld2-gate.env` |

Reconstructed (not a verbatim quote of the log, which prints only `===` section
headers): the sequence in `/home/shiro/rantai/review/run-bld2-r8-final.sh`, with
the four image builds and the `up` service list written on one line each.

```bash
docker compose -p bld2r8fin --env-file /home/shiro/rantai/bld2-gate.env --profile dagster build lakehouse-api dagster-code-location oidc-mock g7-test-runner
docker compose -p bld2r8fin --env-file /home/shiro/rantai/bld2-gate.env --profile dagster up -d --no-build lakehouse-api dagster-code-location dagster-webserver dagster-daemon g3a-source-init clickhouse-iceberg-init
docker compose -p bld2r8fin --env-file /home/shiro/rantai/bld2-gate.env --profile dagster run --rm g3a-test-runner
docker compose -p bld2r8fin --env-file /home/shiro/rantai/bld2-gate.env --profile dagster run --rm g7-test-runner
docker compose -p bld2r8fin --env-file /home/shiro/rantai/bld2-gate.env --profile '*' down -v --remove-orphans
```

### Measured lag (Bronze commit -> Silver rows), 1-minute target

Quoted from `/home/shiro/rantai/review/bld2-gate-r8-final.log`:

```
[g7] TARGET_LAG_SAMPLE_1: Bronze commit -> Silver reached 2001 rows in 18.34s (commit_t is taken after the ingest run reached SUCCESS, so the sample EXCLUDES the ingest run and reads low by its duration plus up to 2 s of poll granularity — review R1 SHOULD-FIX 15)
[g7] TARGET_LAG_SAMPLE_2: Bronze commit -> Silver reached 2002 rows in 32.70s (commit_t is taken after the ingest run reached SUCCESS, so the sample EXCLUDES the ingest run and reads low by its duration plus up to 2 s of poll granularity — review R1 SHOULD-FIX 15)
[g7] TARGET_LAG_SAMPLE_3: Bronze commit -> Silver reached 2003 rows in 16.37s (commit_t is taken after the ingest run reached SUCCESS, so the sample EXCLUDES the ingest run and reads low by its duration plus up to 2 s of poll granularity — review R1 SHOULD-FIX 15)
[g7] TARGET_LAG_SAMPLE_4: Bronze commit -> Silver reached 2004 rows in 28.41s (commit_t is taken after the ingest run reached SUCCESS, so the sample EXCLUDES the ingest run and reads low by its duration plus up to 2 s of poll granularity — review R1 SHOULD-FIX 15)
[g7] TARGET_LAG_SAMPLE_5: Bronze commit -> Silver reached 2005 rows in 32.39s (commit_t is taken after the ingest run reached SUCCESS, so the sample EXCLUDES the ingest run and reads low by its duration plus up to 2 s of poll granularity — review R1 SHOULD-FIX 15)
[g7] TARGET_LAG_RESULT: 5 samples = [18.34, 32.7, 16.37, 28.41, 32.39]s, avg = 25.64s
[g7] TARGET_LAG_RESULT: ALL 5 SAMPLES MET THE 1-MINUTE TARGET (<= 60s)
```

| Sample | Rows reached | Elapsed | Target (60 s) met? |
| :---: | :---: | :---: | :---: |
| 1 | 2001 | 18.34 s | yes |
| 2 | 2002 | 32.70 s | yes |
| 3 | 2003 | 16.37 s | yes |
| 4 | 2004 | 28.41 s | yes |
| 5 | 2005 | 32.39 s | yes |

Worst sample **32.70 s**, average **25.64 s** (from the log's `avg = ` line).
Headroom against the 60 s floor: **27.30 s** (computed as `60 s - 32.70 s`).
Exit status: 0 (`[g7] PASS`).

### Unchanged inputs start no runs

Quoted from `/home/shiro/rantai/review/bld2-gate-r8-final.log`:

```
[g7] baseline run count before unchanged checks: 7
[g7] UNCHANGED_CHECK_1/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_2/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_3/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_4/5: status=met, runs=7 (no new run)
[g7] UNCHANGED_CHECK_5/5: status=met, runs=7 (no new run)
```

Five checks, one per check interval, zero new runs, status `met` throughout.

---

### Why 60 s passes now when it did not before (like-for-like)

The gate measures the same thing in r1, r6 and r8: `commit_t` is taken after
the ingest run's `SUCCESS`, one row is appended per sample, the wait is for
Silver's distinct count to reach Bronze's, and the sensor ticks every 30 s.
What changed between the pre-R6a 1-minute samples (40.7 to 68.94 s, quoted in
the plan; no log on disk, see the note below) and the R8 samples (16.35 to
32.70 s) is R6a itself. At a 60 s target the budget (the request-to-land
median plus 60 s) exceeds the target, so the run is requested at the first
check that sees the change, as plan note 6.3 predicted. Before R6a the
threshold was positive and the pipeline waited one more tick before asking.
It is not a faster machine or a different measurement.

## 2. All Runs Backed by Surviving Logs

| Run | Commit (12 chars) | Date (UTC) | Project | Target | 5 samples (s) | Worst | Result | Headroom | Evidence log on disk |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Pre-R6a 2-min run | `9a59250c77ec` | `2026-10-16T20:01:03+00:00` | `bld2r1f` | 120 s | 103.50, 129.77, 115.69, 101.79, 129.69 (avg 116.09 s) | 129.77 s | FAILED (`max=129.77s`) | None (exceeded target by 9.77 s, computed) | `/home/shiro/rantai/review/bld2-gate-r1.log` |
| Post-R6a 2-min run | `47fcf9ad4618` | `2026-10-16T20:32:16+00:00` | `bld2r6q` | 120 s | 77.20, 95.13, 68.90, 71.11, 89.10 (avg 80.29 s) | 95.13 s | PASSED (`<= 120s`) | 24.87 s (computed: `120 - 95.13`) | `/home/shiro/rantai/review/bld2-gate-r6.log` |
| R8 initial 1-min run (see the note under the table: `3b418d07de54` plus an uncommitted floor patch) | `3b418d07de54` | `2026-10-17T00:59:44+00:00` | `bld2r81m` | 60 s | 16.37, 22.36, 22.32, 26.35, 16.35 (avg 20.75 s) | 26.35 s | PASSED (`<= 60s`) | 33.65 s (computed: `60 - 26.35`) | `/home/shiro/rantai/review/bld2-gate-r8-1min.log` |
| R8 final 1-min run | `29d889340b1a` | `2026-10-17T01:43:18+00:00` | `bld2r8fin` | 60 s | 18.34, 32.70, 16.37, 28.41, 32.39 (avg 25.64 s) | 32.70 s | PASSED (`<= 60s`) | 27.30 s (computed: `60 - 32.70`) | `/home/shiro/rantai/review/bld2-gate-r8-final.log` |

*Note on the R8 initial 1-min run:* the log header says commit `3b418d07de54`,
but at that commit the API floor (`MIN_TARGET_LAG_MINUTES`), the migration CHECK
and the gate's `TARGET_LAG_MINUTES` were 2 minutes, while the log prints
`configuring targetLag=1 minutes`. It ran on `3b418d07de54` plus an uncommitted
patch that set those to 1 (saved as
`/home/shiro/rantai/review/bld2-r2-wip-0101.patch`, 3 files). The R8 final run
on `29d889340b1a` is clean: committed code only.

*Note on earlier unlogged run:* The pre-R6a 1-minute run quoted in plan Section 8 Decision R6 (`[40.70, 68.94, 56.85, 56.90, 52.82]` s, worst 68.94 s) has no surviving log file on disk.

Commands executed for each logged run (reconstructed, abbreviated with `...`;
the logs themselves print only `===` section headers and the output of each step.
Only the two R8 runs have a surviving script, `run-bld2-r8-1min.sh` and
`run-bld2-r8-final.sh` in the same directory; the r1 and r6 lines are inferred
from the project names and the log structure, not quoted):
- `bld2-gate-r1.log`: `docker compose -p bld2r1f --env-file /home/shiro/rantai/bld2-gate.env --profile dagster build ... && up ... && run g3a-test-runner && run g7-test-runner`
- `bld2-gate-r6.log`: `docker compose --env-file /home/shiro/rantai/bld2-gate.env -p bld2r6q --profile dagster build ... && up ... && run g3a-test-runner && run g7-test-runner`
- `bld2-gate-r8-1min.log`: `docker compose -p bld2r81m --env-file /home/shiro/rantai/bld2-gate.env --profile dagster build ... && up ... && run g3a-test-runner && run g7-test-runner`
- `bld2-gate-r8-final.log`: `docker compose -p bld2r8fin --env-file /home/shiro/rantai/bld2-gate.env --profile dagster build ... && up ... && run g3a-test-runner && run g7-test-runner`

---

## 3. What the content fingerprint costs (BLOCKER 4, SHOULD-FIX 22)

The content fingerprint is computed with the 3-column query from
`mart_content_fingerprint` in `rust/crates/lakehouse-api/src/gold_export_history.rs`:

```sql
SELECT toString(count()) AS row_count,
       toString(ifNull(sum(cityHash64(*)), toUInt64(0))) AS content_sum,
       (SELECT toString(cityHash64(groupArray(concat(name, ':', type))))
        FROM (SELECT name, type FROM system.columns
              WHERE database = 'silver' AND table = 'g7_dedupe_select_out' ORDER BY position)) AS schema_hash
FROM `silver`.`g7_dedupe_select_out` FORMAT JSON
```

Measured results on `silver.g7_dedupe_select_out` (2005 rows):

Label for the two `[fp]` blocks below: the log prints `clickhouse-client`'s
JSON pretty-printed across many `[fp] ... run N:` lines. The `"elapsed": ...
(... s)` lines are assembled from the one log line that carries
`"elapsed": <seconds>,` plus a rounding added here, and the JSON data blocks
are the `row_count`, `content_sum` and `schema_hash` values taken from separate
log lines. They are not verbatim single lines of the log.

### From `/home/shiro/rantai/review/bld2-gate-r1.log` (commit `9a59250c77ec`)

```
[fp] silver.g7_dedupe_select_out run 1: "elapsed": 0.02213758 (0.022 s)
[fp] silver.g7_dedupe_select_out run 2: "elapsed": 0.010757992 (0.011 s)
[fp] silver.g7_dedupe_select_out run 3: "elapsed": 0.01101926 (0.011 s)
```

Data returned in run 2 & 3:
```json
{
  "row_count": "2005",
  "content_sum": "18094079235951061584",
  "schema_hash": "8698929433857648039"
}
```

### From `/home/shiro/rantai/review/bld2-gate-r8-final.log` (commit `29d889340b1a`)

```
[fp] silver.g7_dedupe_select_out run 1: "elapsed": 0.010633516 (0.011 s)
[fp] silver.g7_dedupe_select_out run 2: "elapsed": 0.011588246 (0.012 s)
[fp] silver.g7_dedupe_select_out run 3: "elapsed": 0.009232482 (0.009 s)
```

Data returned:
```json
{
  "row_count": "2005",
  "content_sum": "13702958784301655157",
  "schema_hash": "8698929433857648039"
}
```

The 3-column query runs in ~10–12 ms on this 2005-row table (~0.011 s).
*Note:* `bld2-gate-r6.log` tested an abbreviated 2-column query returning 0.006 s and 0.007 s.

---

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
