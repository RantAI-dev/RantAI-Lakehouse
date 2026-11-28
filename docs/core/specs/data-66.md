# `DATA-66` Iceberg sync per table: on every update, on a schedule, or by hand

| | |
| --- | --- |
| Backlog | `DATA-66` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open (`DATA-6`'s code #612, `DATA-63` #686, `DATA-64` #699, `DATA-65` #717 and `BLD-12` #715 are merged) |

## Why

The product owner, 2026-11-27: "the important thing now is that the choice to
publish to Iceberg can be real time: when ClickHouse is updated, Iceberg is
updated. And it must have a setting." ClickHouse stays the primary store for
Silver and Gold (`DATA-3`, which would have moved Gold to Iceberg first, is
killed by the same decision). The Iceberg copy is what outside tools read, so
it must keep up with ClickHouse, and how closely it keeps up is the table
owner's choice.

## What users get

On a Silver or Gold table's "Publish in open format (Iceberg)" card, with
publishing on, a **Sync** setting with three choices:

1. **On every update.** When the table changes in ClickHouse, its Iceberg
   tables (`<zone>.<t>` and `<zone>.<t>_latest`) are updated on their own,
   whether a pipeline or something else changed it.
2. **On a schedule.** Hourly at a chosen minute, or daily at a chosen time,
   in a chosen time zone. Nothing happens between runs.
3. **Manual only.** Only "Publish now" updates it.

A failed sync never fails or undoes the ClickHouse write; the card says
"Last publish failed" or "Out of date" until a sync succeeds.

## Target specs

"Today" is `main` at `b6617a3e7`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page.

| Capability | Today | Target |
| --- | --- | --- |
| Choice of sync per table | None: every switched-on table is exported after any authored pipeline succeeds (all tables, one serial run) and nightly at 04:00 | Three modes per `(zone, table)`, stored in Postgres, shown and changed on the card, returned by the API |
| Pipeline write to `<zone>.<t>` updated | One all-tables run after any authored pipeline, queued behind the previous all-tables run; not measured | A run for **that table alone**, asked for by the pipeline's own step as soon as its write is done; p95 under **60 s** for a 100,000-row table on the compose stack *(proposed)*, measured |
| Change made outside a pipeline | Picked up only by the nightly run | Noticed within **15 s** (watcher interval) and synced; p95 under **90 s** end to end for a 100,000-row table *(proposed)*, measured |
| Retry after a failed sync | Next pipeline success or the nightly run | Every **5 minutes** while the newest run is a failure, for "On every update" tables *(proposed)* |
| Scheduled | Fixed 04:00 daily for every table | Hourly at :MM, or daily at HH:MM, in an IANA time zone; one run per slot, unchanged tables skipped |
| Manual only | Not possible (switched-on tables always sync) | Never run by a pipeline, the watcher or a schedule |
| Effect of a failed sync on the pipeline | None (separate run) | None: the pipeline run succeeds; proven by a test |
| Sync of an unchanged table | Skipped (`DATA-10` fingerprint) | Unchanged |

## Benchmark

Snowflake's dynamic tables (also those stored as Iceberg tables) take a target
lag per table, with one minute as the smallest. Databricks' UniForm writes a
Delta table's Iceberg metadata after each commit, without a setting. Our
"On every update" sits between them: per-table, with a measured p95 instead of
a promised lag.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-66-AC1` Set a Gold table to "On every update"; run its pipeline with new rows; the card says "Up to date" and an outside tool reads the new rows in `gold.<t>` and `gold.<t>_latest` without pressing anything
- `DATA-66-AC2` (operator) Change the same table in ClickHouse outside any pipeline; it is synced without pressing anything
- `DATA-66-AC3` Set a table to "On a schedule", hourly; run its pipeline; nothing is published until the slot
- `DATA-66-AC4` Set a table to "Manual only"; run its pipeline and wait past the watcher; nothing is published until "Publish now"
- `DATA-66-AC5` (operator) Stop the Iceberg catalog; run a pipeline on an "On every update" table: the pipeline succeeds, the dashboard shows the new rows, the card says "Last publish failed"; start the catalog: the table syncs within the retry interval
- `DATA-66-AC6` As a user without `gold:export`, the setting is visible but not changeable, with an explanation
- `DATA-66-AC7` The same for a Silver table

## Not included

- Streaming each row into Iceberg (a sync is still a full copy plus the latest
  table, as `DATA-2` and `DATA-6` define).
- A sync setting on the pipeline page (the pipeline's output table links to
  the catalog page that holds the card).
- Cron expressions for the schedule.

## Asking the assistant

Not part of the dashboards assistant (`AI-1` to `AI-15`). Publishing is not
one of its tools.

## Before anyone builds it

Feature page: [`features/iceberg-sync-per-table.md`](../features/iceberg-sync-per-table.md).
Plan: `docs/superpowers/plans/2026-11-27-data-66-iceberg-sync.md`.
