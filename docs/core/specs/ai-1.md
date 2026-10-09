# `AI-1` Manage the dashboards that exist today

| | |
| --- | --- |
| Backlog | `AI-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | AI Copilot |
| When | Next, ready now |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

The assistant can build charts but cannot rename, delete, arrange or move them, so users fall back to doing it by hand.

## What users get

Ask the assistant to tidy a dashboard.

## A user says

> Rename this dashboard to 'Q3 Tourism' and move the Bali chart to the top.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Rename a dashboard or chart | Not possible | A tool, effective at once |
| Delete | Not possible | A tool that always asks for approval |
| Arrange and resize tiles | Not possible | Move, resize and order tiles by plain instruction ('top', 'next to', 'half width') |
| Move a chart to another dashboard | Not possible | A tool |
| Undo | n/a | Every change can be undone in one step |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

Assistant parity rule; BI-18 C will add tabs (AI-9).

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `AI-1-AC1` The quoted request does exactly that
- `AI-1-AC2` Deleting asks for approval and does nothing on 'no'
- `AI-1-AC3` A user without edit rights is told they cannot

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
