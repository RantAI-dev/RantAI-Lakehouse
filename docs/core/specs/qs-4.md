# `QS-4` History and query profile

| | |
| --- | --- |
| Backlog | `QS-4` in [BACKLOG.md](../BACKLOG.md) |
| Module | Query Studio |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

History is the last 200 runs in a side panel, and the query profile always reads 'Not measured'.

## What users get

Find any past query and see why a slow one is slow.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| History | Last 200 in a side panel | A history page covering 14 days (Snowflake), filtered by user, status, duration, engine, table and time |
| Profile | Always 'Not measured' | Operator tree from the engine's plan with time, rows and bytes per step, for ClickHouse and Trino |
| Hints | None | Plain hints for the common causes: full scan, no partition pruning, large join, spill |
| Admin view | Not checked | Admins see everyone's queries and can cancel them |

## Benchmark

Snowflake Query History: 14 days, many filters; Query Profile with operator statistics. Databricks: query history with filters, query profile with insights.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `QS-4-AC1` A query run 10 days ago is found by filtering on its table
- `QS-4-AC2` A slow query's profile names the step that took the most time
- `QS-4-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Help inside the SQL editor is [`AI-12`](ai-12.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
