# `DATA-2` Keep the last copies of a published Gold table, remove the rest

| | |
| --- | --- |
| Backlog | `DATA-2` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open (was: DATA-10, shipped) |

## Why

Each publish of a changed Gold table appends a full copy, stamped with its
export time, to the open-format table. Old copies are never removed
(`features/gold-publish-per-mart.md`, Limits), so storage grows by one copy
per publish and outside readers must filter on the newest export time.

## What users get

A published table keeps a fixed number of recent copies. Older copies and
the files behind them are removed automatically.

## Decisions (product owner, 2026-10-14)

- Keep the **last N copies**, N set by the operator; older copies are removed.
- Outside readers still filter on the newest export time, so the
  "latest copy only" view (`DATA-6`) is still built.

## Target specs

"Today" is `main` at `99b42812`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Copies kept | All, forever | The last 7 copies *(proposed)*, per table; an operator setting, at least 1 |
| When trimmed | Never | After each successful publish |
| Storage freed | Never | The removed copies' data files are deleted: rows removed, then old snapshots expired and orphan files removed, as Bronze maintenance does |
| Shown | Nothing | Copies kept, oldest kept export time, and the last trim's result on the table's publish page |
| Failure | n/a | A failed trim is recorded with a fixed reason and never shown as done; the published data is untouched |

## How (planner's note, to be confirmed by the plan)

The Rust Iceberg library this repo uses (`iceberg` 0.10.1) can append and
expire snapshots but cannot delete rows or overwrite. The trim therefore
runs in the Dagster code location with PyIceberg, which can delete rows by a
filter on the export time, followed by snapshot expiry. The plan verifies
this on the compose stack before anything is built; if it does not work, the
item is reported, not approximated.

## Benchmark

Apache Iceberg's own table maintenance: snapshot expiry keeps at least 1
snapshot and expires those older than 5 days by default
(`history.expire.min-snapshots-to-keep`, `history.expire.max-snapshot-age-ms`).
Snowflake and Databricks replace a table's contents on each refresh and keep
history only as time travel.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-2-AC1` Publish a table 9 times with a change each time; 7 copies remain, the newest 7
- `DATA-2-AC2` (operator) The object store holds no data files for the removed copies after the trim
- `DATA-2-AC3` An outside reader filtering on the newest export time sees the same rows as the console
- `DATA-2-AC4` A forced trim failure is shown with a reason; the published rows are unchanged
- `DATA-2-AC5` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Keeping copies by age instead of by count.
- The "latest copy only" view (`DATA-6`).

## Asking the assistant

Not part of the dashboards assistant.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
