# `AI-6` Richer charts

| | |
| --- | --- |
| Backlog | `AI-6` in [BACKLOG.md](../BACKLOG.md) |
| Area | Assistant for dashboards |
| Who builds it | AI team |
| When | Next, with each BI phase-2 item |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-6`](bi-6.md) (BI phase 2), [`BI-7`](bi-7.md) (BI phase 2), [`BI-10`](bi-10.md) (BI phase 2), [`BI-11`](bi-11.md) (BI phase 2), [`BI-13`](bi-13.md) (BI phase 2) |
| Status | Spec. Not planned, not built |

## Why

Multi-dimension, joined and combined charts are the requests users find hardest to build.

## What users get

Ask for a complex chart in one sentence.

## A user says

> Visitors by province and month, domestic only, next to hotel occupancy.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Several dimensions and filters | Not possible | As BI-6 |
| Joins | Not possible | As BI-7, choosing join columns from the catalog |
| Bins, groups, hierarchies | Not possible | As BI-10 |
| Parameters | Not possible | As BI-11 |
| Two sources on one chart | Not possible | As BI-13 |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-6, BI-7, BI-10, BI-11, BI-13.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] The quoted request produces the described chart in one go

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
