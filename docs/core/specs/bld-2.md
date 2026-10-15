# `BLD-2` Tables that refresh themselves within a target lag

| | |
| --- | --- |
| Backlog | `BLD-2` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open (was: BLD-3, DATA-10, shipped) |

## Why

A pipeline runs on a schedule or after its upstream pipeline succeeds. A
table built from a Bronze table that a connector refreshes, or from an
uploaded table, is only as fresh as its schedule, and a schedule set too
often wastes runs when nothing changed.

## What users get

The user says how fresh a pipeline's output must be ("at most 10 minutes
behind its inputs"). The product runs the pipeline when an input has changed
and the lag would otherwise be exceeded, and shows whether the target is met.

## Decisions (product owner, 2026-10-14)

- Approach: a **target lag per pipeline**, as Snowflake Dynamic Tables do,
  built on the existing pipelines and their Dagster sensors. Not ClickHouse
  materialized views.
- Smallest target lag a user may set: **1 minute**, Snowflake's minimum.
- The assistant sets and reads the target lag in the same PR.

## Target specs

"Today" is `main` at `99b42812`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Freshness setting | Cron schedule or "after upstream" | A target lag per pipeline, 1 minute to 7 days (Snowflake's range) |
| Downstream lag | None | "Downstream": refresh only when a pipeline that reads this one needs it (Snowflake's `DOWNSTREAM`) |
| Change detection | None | Runs only when an input changed: a Bronze input's Iceberg snapshot ID, a ClickHouse input's content fingerprint (`DATA-10`) |
| No change, no run | n/a | A pipeline whose inputs did not change does not run, and its lag is still shown as met |
| Lag shown | None | Current lag, target, and "met" or "behind" on the pipeline page and in the list |
| Behind target | None | An alert when a pipeline stays behind its target for 3 checks in a row *(proposed)*, through the existing alert rules |
| Measured lag | None | Measured on the compose stack with the target set to 1 minute and recorded in a `*-RESULT.md`; if 1 minute cannot be met, the measured floor is reported and the minimum raised, never claimed |

## Benchmark

Snowflake Dynamic Tables: `TARGET_LAG` from 1 minute to 7 days, or
`DOWNSTREAM`; refresh skipped when inputs did not change; lag shown per
table. Databricks Lakeflow materialized views: refresh on a schedule or when
upstream data changes.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BLD-2-AC1` Set a target lag of 5 minutes on a Bronze-to-Silver pipeline; load new rows into the Bronze table; the Silver table has them within 5 minutes
- `BLD-2-AC2` With no new input for an hour, the pipeline does not run and shows "met"
- `BLD-2-AC3` A downstream pipeline set to "Downstream" refreshes its upstream only when it needs it
- `BLD-2-AC4` A pipeline kept behind its target raises an alert
- `BLD-2-AC5` The assistant sets a target lag from a plain-language request
- `BLD-2-AC6` A user without `pipeline:write` is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Incremental refresh (only the changed rows). Each refresh rebuilds the
  output, as pipelines do today.
- ClickHouse materialized views.

## Asking the assistant

The assistant change is part of this item (decision above). The AI team
reviews that part of the plan and the diff.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
