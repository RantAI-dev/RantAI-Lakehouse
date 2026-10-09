# `AI-3` Time grain

| | |
| --- | --- |
| Backlog | `AI-3` in [BACKLOG.md](../BACKLOG.md) |
| Module | AI Copilot |
| When | Next, with BI-9 |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-9`](bi-9.md) (BI phase 1) |
| Status | Spec. Not planned, not built |

## Why

Users ask for monthly or weekly views constantly.

## What users get

Change a chart's time grain by asking.

## A user says

> Make this monthly.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Time grain in chart tools | None | Every grain and date part of BI-9 |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-9.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `AI-3-AC1` The quoted request switches a daily chart to monthly

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
