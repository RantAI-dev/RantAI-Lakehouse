# `AI-2` Dashboard filters

| | |
| --- | --- |
| Backlog | `AI-2` in [BACKLOG.md](../BACKLOG.md) |
| Area | Assistant for dashboards |
| Who builds it | AI team |
| When | Next, with BI-18 A |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-18`](bi-18.md) (BI phase 1) |
| Status | Spec. Not planned, not built |

## Why

Without a filter tool the assistant cannot answer 'show only 2025'.

## What users get

Ask the assistant to filter a dashboard.

## A user says

> Show only 2025 and only Bali.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Add, change and clear filters | Not possible | Every filter type of BI-18 A, including relative dates |
| Viewer or default | n/a | Changes the viewer's filters; saving as default only on explicit request |
| Linked filters | n/a | Respects linked filter values |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-18 part A defines the filter types.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] The quoted request sets a year and a province filter
- [ ] The next viewer's dashboard is unchanged

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
