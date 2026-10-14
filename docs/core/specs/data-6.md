# `DATA-6` A "latest copy only" table for outside readers

| | |
| --- | --- |
| Backlog | `DATA-6` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Waits for | `DATA-2` |

## Why

A published Gold table holds several copies, one per publish (`DATA-2` keeps
the last N). An outside tool that does not filter on the newest export time
counts every copy and reports wrong numbers.

## What users get

Next to each published table, a second open-format table that always holds
only the newest copy, so any outside tool reads correct numbers with no filter.

## Decisions (product owner, 2026-10-14)

- Built, alongside `DATA-2`'s "keep N copies".

## Target specs

"Today" is `main` at `99b42812`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Latest-only table | None | `gold.<table>_latest` *(proposed name)*, replaced with the newest copy on each successful publish |
| Form | n/a | A plain Iceberg table, not an Iceberg view: every reader that reads Iceberg tables reads it; Iceberg views are not read the same way by every engine |
| Consistency | n/a | Replaced in one Iceberg commit: a reader sees the old copy or the new one, never a mix |
| Shown | n/a | The table's name on the publish page, with a copy button |
| Switch off | n/a | Switching publishing off stops updates to both tables; both stay |

## Benchmark

Snowflake and Databricks expose a table that always holds the current
contents; history is reached through time travel, not by filtering.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-6-AC1` (operator) After a publish, `SELECT count(*)` on the latest-only table from a second tool, with no filter, equals the console's row count
- `DATA-6-AC2` (operator) Reading during a publish never returns a mix of two copies
- `DATA-6-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Iceberg views.

## Asking the assistant

Not part of the dashboards assistant.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
