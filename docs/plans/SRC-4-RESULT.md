# `SRC-4` result: slice 0, Kafka Connect and the applier measured

The measurement of ADR 0016 (`docs/adr/0016-change-capture-per-connector-on-kafka-connect.md`)
that decision 13 asks for before any product code. **Verdict: both targets are met
in both load modes, in two runs of the same code.** The ADR can move from Proposed
to Accepted; the limits under "What was not measured" and "Corrections to the plan"
go with it.

Measured on the throwaway branch `measure/src-4-cdc` (cut from
`feat/data-phase-4-src-4` at `67fa4724`, merged with the amended plan at `130e3ee3`;
never merged to a product branch), workflow `src4-measure`, final commit `2a2671c9`.
Each run is one job on a GitHub-hosted `ubuntu-latest` runner, from a fresh stack.
Harness: `ops/src4/measure_cdc.py`; applier prototype:
`dagster/dispar_orchestrate/cdc_apply.py`; stack:
`ops/src4/docker-compose.src4.override.yml`.

| | CI run | Result |
|---|---|---|
| Run A | [40816001350](https://github.com/RantAI-dev/RantAI-Lakehouse/actions/runs/40816001350) (push) | success, all verdicts `met` |
| Run B | [40817900374](https://github.com/RantAI-dev/RantAI-Lakehouse/actions/runs/40817900374) (`workflow_dispatch`, same commit) | success, all verdicts `met` |

Three earlier runs on the same branch are not results; each stopped on a defect
of the prototype or the harness, which the commit after it names: `40808162436`
(the harness read the connector's status before it existed), `40808958427` (the
applier named its columns `__source_ts_ms` where `dlt` writes `_source_ts_ms`, so
every batch after the first copy was refused), `40813284847` (Debezium 3.7.0 (the image's real version; the tag `3.8` is not a 3.8 release, corrected 2026-10-30) ignores
the plan's delete option, see below; inserts and updates were already within
target there).

## What was measured, and how

Two identical source tables in schema `src4` of the stack's own `postgres:16`
(`orders` for "current row per key", `orders_history` for "history"), one
connector with two tables, one topic and one Bronze table per mode, so each mode
starts from a clean source. 10,000 seeded rows, copied first. The applier is the
Dagster job `cdc_apply_job` with one sensor per mode, `minimum_interval_seconds=30`,
batches of up to 10,000 events, offsets committed after the Bronze commit.

- **Freshness.** Per mode, 30 inserts, 30 updates and 30 deletes interleaved, one
  every 2 s. Latency is the time from just before the statement is sent to the
  return of the first ClickHouse poll that sees the change in Bronze. Both ends
  round upward (the commit is inside the interval; the poll's own duration, median
  0.31 to 0.35 s in all runs, is added), so a figure here is never below the true
  latency; it can be above it by up to a poll's duration and the statement's own round trip. A change that is never seen is
  counted as `not visible` and fails the target; none was missed.
- **Correctness.** After the 90 and again after the catch-up, Bronze compared with
  the source. Current mode: same keys, same values (`id`, `customer`, `amount`,
  `status`), deleted keys absent, no key twice. History mode: the multiset of
  (`id`, operation, `customer`, `status`) equals the snapshot plus `src4.audit`
  (a trigger that records every change the source commits), and for every key the
  events in Kafka-offset order equal the order the source committed them.
- **Stop and catch up.** Connector paused (state asserted `PAUSED`) and the mode's
  sensor stopped (asserted `STOPPED`); 108,000 changes committed at the source,
  counted from the audit table, 75,600 updates, 21,600 inserts and 10,800 deletes
  (exactly 70, 20 and 10 %); one further sentinel update; connector and sensor
  resumed; time to the sentinel in Bronze. The 108,000 changes are written as fast
  as the source takes them (2 to 3 s), not at 30 per second over an hour: what is
  tested is the backlog an hour of changes leaves, not the hour.
- **Cost.** `docker stats` of the broker, Kafka Connect and the code location
  (which runs the applier), sampled every 5 s over the whole run (79 samples
  each); the applier's batch times from its `CDC_BATCH` log lines.

## Freshness (seconds, 30 changes per cell)

| Mode | Kind | Run A p50 | p95 | max | Run B p50 | p95 | max |
|---|---|---|---|---|---|---|---|
| current | insert | 5.17 | 30.08 | 36.08 | 5.07 | 22.01 | 28.01 |
| current | update | 4.99 | 28.08 | 34.08 | 5.32 | 20.01 | 26.01 |
| current | delete | 5.63 | 26.08 | 32.08 | 5.72 | 18.01 | 24.01 |
| history | insert | 6.10 | 40.30 | 46.30 | 5.95 | 38.65 | 44.65 |
| history | update | 5.98 | 38.30 | 44.30 | 6.23 | 36.65 | 42.65 |
| history | delete | 5.54 | 36.30 | 42.30 | 5.79 | 34.65 | 40.65 |

Target (decision 5): p95 of each kind within **120 s**, in both modes. **Met**: the
worst p95 is 40.30 s (history, insert, run A). The p50 is about 5 to 6 s; the p95
and max are the cost of the sensor's 30 s interval plus the run's start (a change
that arrives just after a tick waits for the next one). Nothing but the sensor
interval would shorten them; the target does not need it.

## Correctness and catch-up

| Mode | Check | Run A | Run B |
|---|---|---|---|
| current | after the 90: rows, missing, extra, duplicated, different | 10,000 / 0 / 0 / 0 / 0, equal | 10,000 / 0 / 0 / 0 / 0, equal |
| current | after catch-up: rows | 20,800, 0 / 0 / 0 / 0, equal | 20,800, 0 / 0 / 0 / 0, equal |
| current | catch-up of 108,000 changes | 104.8 s | 94.1 s |
| history | after the 90: events expected / in Bronze, missing, extra, keys out of order | 10,090 / 10,090, 0 / 0 / 0, equal | 10,090 / 10,090, 0 / 0 / 0, equal |
| history | after catch-up: events expected / in Bronze | 118,091 / 118,091, 0 / 0 / 0, equal | 118,091 / 118,091, 0 / 0 / 0, equal |
| history | catch-up of 108,000 changes | 61.4 s | 61.9 s |

Target: Bronze equals the source after both, zero missing or duplicated rows.
**Met**, all eight checks. History received exactly 108,000 new events plus the
90 of the freshness step and the sentinel (118,091 less the 10,000 snapshot rows
= 108,091).

## Cost

| | Run A | Run B |
|---|---|---|
| Broker (`apache/kafka:3.8.0`) peak memory | 580 MiB (608,174,080 B) | 637 MiB (668,362,342 B) |
| Kafka Connect worker peak memory | 632 MiB (662,280,601 B) | 735 MiB (770,283,929 B) |
| Code location (runs the applier) peak memory | 992 MiB (1,039,977,676 B) | 1.04 GiB (1,111,322,787 B) |
| Applier batches, current mode / history mode | 61 / 51 | 62 / 51 |
| Time per batch, median, current / history | 3.40 s / 3.65 s | 3.53 s / 3.80 s |
| Time per batch, max, current / history | 7.58 s / 4.82 s | 7.96 s / 4.84 s |
| Seconds per 1,000 events, current / history | 2.00 / 1.71 | 2.09 / 1.73 |
| Largest batch | 10,000 events | 10,000 events |

Broker plus worker peak at 1.2 to 1.3 GiB together in this load (ADR 0016 says
"roughly 1.5 to 2 GB"; the figure to write into its consequences is about
**1.3 GiB measured at this load, 10,000 to 20,000 rows and 108,000 changes**, and
that the code location, which already exists, peaked above 1 GiB while applying
a 10,000-event batch). `docker stats` every 5 s can miss a shorter peak. No target
was set for cost.

## Image digests

| Service | Image |
|---|---|
| Kafka Connect | `quay.io/debezium/connect:3.8@sha256:d67d678c726d1659f458e057e4a3a50a3b14c3e85a3be7f31de9db53dfb46668` (tag `3.8`, the newest 3.x tag; **its config says `DEBEZIUM_VERSION=3.7.0.Final`**, so every "Debezium 3.8" in this file means 3.7.0, corrected 2026-10-30; its Kafka Connect runtime reports 4.3.1) |
| Broker | `apache/kafka:3.8.0@sha256:c89f315cff967322c5d2021434b32271393cb193aa7ec1d43e97341924e57069` |
| Source | `postgres:16` at `sha256:1beaa3084eee2676f20a9cd9928f0bf2c5bf674f8e49f81017912183d377c61c` |
| Reader | `clickhouse/clickhouse-server:26.8` at `sha256:d426d2f612e1e189c90f9a3f8dcd85a03c6851273833817a296bb327e63aa7a7` |

## Corrections to the plan found by measuring

1. **Deletes in current mode.** `dlt` 1.30.0's Iceberg merge has no
   `hard_delete` (plan Amendment 1, already applied): the applier deletes the
   batch's keys and appends the survivors in one `pyiceberg` transaction. Run
   A and B show it correct, including a key deleted and re-created inside one
   batch (unit-tested) and a replayed batch (unit-tested on a local catalog;
   the crash itself was not provoked in CI).
2. **The connector option.** The plan names `delete.handling.mode=rewrite` and
   `drop.tombstones`. Debezium 3.7.0 ignores that pair: the worker's log showed the
   effective `delete.tombstone.handling.mode = tombstone`, so every delete reached
   the topic with no value and none reached Bronze (run `40813284847`). The
   3.8 spelling is `transforms.unwrap.delete.tombstone.handling.mode=rewrite`.
   Slice 2's connector renderer must use it.
3. **Column names.** `dlt` writes `__source_ts_ms` as `_source_ts_ms` (and `__op`,
   `__deleted`, `__offset` likewise). The applier names its columns `_op`,
   `_source_ts_ms`, `_deleted` and `_offset`; a test asserts every name it writes
   is a fixed point of `dlt`'s naming.
4. **Two source tables** instead of the plan's single `src4.orders`, one per mode
   (a clean source and a clean Bronze table for each mode; the plan's check
   "history: one row per event" needs a source whose whole history is known).
5. **Retries.** The op carries the repository's default retry policy
   (`test_op_source_metadata.py` demands it); in run `40808958427` the schema
   refusal was retried by it and hid for 13 minutes. A retry of a batch is safe
   (a replay gives the same table), but slice 2 should classify a refusal as not
   retryable.

## What was not measured

- Any database other than PostgreSQL; MariaDB, Oracle, SQL Server, MongoDB and
  MySQL behaviour, the Oracle driver and DBZ-9681 in particular.
- The API, the console, migrations, the secrets path (the connector's password was
  in the connector JSON in this stack; ADR 0016 point 4 is untested), and the
  fixed pipeline's removal.
- A real hour of changes at 30 per second (the backlog was written in 2 to 3 s),
  sustained load, and a source with more than 10,000 rows.
- Failure behaviour: a crash of the worker or the applier between the Bronze commit
  and the offset commit was reasoned and unit-tested, not provoked; Kafka Connect
  restart, rebalance and slot loss were not exercised.
- Schema change at the source (the applier refuses a new column by design), a
  second connector, a second partition, more than one concurrent applier run per
  topic.
- Latency below the sensor's 30 s granularity; the broker, the worker and the
  code location were never limited in memory or CPU. The runner's CPU count was
  not recorded.
- Peak memory is the maximum of 79 samples 5 s apart.
- The full `python -m pytest dispar_orchestrate` run of the Dagster tests was
  2,401 passed, 1 skipped, 3 failed locally on this VM (`test_load_memory.py`,
  three cases of `test_the_load_of_a_four_times_larger_file_does_not_need_more_memory`);
  those cases measure process memory, were not run on the base commit, and are not
  touched by this slice. *Not verified* whether they fail there too.

## Verdict against the targets

| Target | Mode | Verdict |
|---|---|---|
| D5: p95 of each change kind within 120 s | current | **met** (worst p95 30.08 s) |
| | history | **met** (worst p95 40.30 s) |
| Bronze equals the source after the 90 and after the catch-up, zero missing or duplicated | current | **met** (4 of 4 checks, both runs) |
| | history | **met** (4 of 4 checks, both runs) |
| Cost stated | | broker + worker 1.2 to 1.3 GiB at peak in this load; code location above 1 GiB while applying |
