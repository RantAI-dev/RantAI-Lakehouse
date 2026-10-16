# `DATA-3` Store Gold in open format first; serve from a fast copy

| | |
| --- | --- |
| Backlog | `DATA-3` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | `DATA-4`, and a new ADR that replaces ADR 0010's direction |

## Why

Today a Gold table is written to ClickHouse, and an open-format copy is made
afterwards (ADR 0010). The open copy is always behind and is a second system
to keep in step. Most competitors make the open table the source of truth
and the fast engine a cache of it (`reference/competitive-comparison.md`).

## What users get

Each Gold table is written once, to open format. Dashboards still read a
fast ClickHouse copy, refreshed from it. Outside tools and dashboards read the
same data.

## Decisions (product owner, 2026-10-14)

- Built, after `DATA-4`.

## Target specs

"Today" is `main` at `99b42812`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Source of truth for Gold | ClickHouse table | Iceberg table in the `gold` namespace |
| Dashboard reads | ClickHouse table | ClickHouse copy, refreshed from Iceberg after each write |
| Copy lag | n/a | The ClickHouse copy is current within 60 seconds *(proposed)* of the Iceberg commit, measured and recorded in a `*-RESULT.md` |
| Dashboard speed | Today's | No slower than today on the BI test set: p95 tile time within 10% *(proposed)* of the measured baseline |
| Choice per table | n/a | Chosen when a Gold table is created; existing tables keep today's path until switched (Snowflake's choice per table) |
| Masking | Applied when published | Applied on every read of the ClickHouse copy and on publish, as today |
| Architecture record | ADR 0010 | A new ADR, accepted before the plan, replacing ADR 0010's direction for these tables |

## Benchmark

Snowflake: managed Iceberg tables chosen when a table is created. StarRocks:
base tables stay in Iceberg, materialized views accelerate them. Databricks:
Delta and Iceberg tables are the source; the engine caches.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-3-AC1` Create a Gold table on the new path; its dashboard shows the same numbers as before
- `DATA-3-AC2` (operator) A second tool reads the Gold table with no publish step
- `DATA-3-AC3` After a pipeline run, the dashboard shows the new numbers within the copy-lag target
- `DATA-3-AC4` Masking applies on the dashboard and on the outside read
- `DATA-3-AC5` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Moving existing Gold tables automatically.
- Silver on this path.

## Asking the assistant

Not part of the dashboards assistant; the dashboard tools read the
ClickHouse copy as they read Gold today.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number), a new ADR, and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
