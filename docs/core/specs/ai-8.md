# `AI-8` Chart formatting

| | |
| --- | --- |
| Backlog | `AI-8` in [BACKLOG.md](../BACKLOG.md) |
| Area | Assistant for dashboards |
| Who builds it | AI team |
| When | Next, with BI-17 |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-17`](bi-17.md) (BI phase 3, 6) |
| Status | Spec. Not planned, not built |

## Why

Formatting is many small settings; asking is faster.

## What users get

Format a chart by asking.

## A user says

> Show values in rupiah, make the bars green, add a target line at 1,000.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Format fields in chart tools | None | Every setting of BI-17 A: number formats, colours, axes, labels, reference lines, themes |
| Forecast and clustering | None | As BI-17 B |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-17.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] The quoted request applies all three changes

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
