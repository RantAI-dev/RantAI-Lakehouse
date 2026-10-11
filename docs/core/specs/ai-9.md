# `AI-9` Tabs and templates

| | |
| --- | --- |
| Backlog | `AI-9` in [BACKLOG.md](../BACKLOG.md) |
| Module | AI Copilot |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-18`](bi-18.md) (BI phase 3) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Reorganising a large dashboard is tedious by hand.

## What users get

Reorganise dashboards by asking.

## A user says

> Put the map charts in a separate tab.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Tabs | None | Create, rename, reorder tabs; move charts between them |
| Templates | None | Start a dashboard from a template; save one as a template |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-18 part C.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `AI-9-AC1` The quoted request creates a tab with the map charts

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
