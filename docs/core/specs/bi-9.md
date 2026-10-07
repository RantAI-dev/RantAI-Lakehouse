# `BI-9` Group by day, week, month, quarter, year

| | |
| --- | --- |
| Backlog | `BI-9` in [BACKLOG.md](../BACKLOG.md) |
| Area | Dashboards |
| Who builds it | Dashboards |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

A date column can only be used as it is; there is no time grain in the builder or on the dashboard.

## What users get

Any chart can group dates by minute up to year, and viewers can switch the grain.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Grains | None | Minute, hour, day, week, month, quarter, year (Metabase's list) |
| Date parts | None | Hour of day, day of week, day of month, week of year, month of year, quarter of year |
| Week start | n/a | Admin setting, Monday by default *(proposed)* |
| Dashboard switch | None | A time-grain filter that changes the grain of every chart linked to it |
| Time zone | Not checked | Grouping follows the report time zone setting |

## Benchmark

Metabase: minute, hour, day, week, month, quarter, year, plus hour-of-day, day-of-week, month-of-year, quarter-of-year. Tableau: date parts and truncations.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A line chart of daily rows switches to monthly from the dashboard
- [ ] Day-of-week shows seven bars Monday to Sunday
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Ships with [`AI-3`](ai-3.md) (Time grain), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
