# `AI-7` Named metrics first

| | |
| --- | --- |
| Backlog | `AI-7` in [BACKLOG.md](../BACKLOG.md) |
| Module | AI Copilot |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-2`](bi-2.md) (BI phase 2) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

If the assistant writes its own SQL for 'revenue', two people can get two numbers.

## What users get

One question, one number, everywhere.

## A user says

> What was revenue last quarter?

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| List and use metrics | None | Tools to list, describe and use metrics |
| Preference | Writes SQL | Uses a named metric when one matches; says which one; writes SQL only when none does, and says so |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-2.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `AI-7-AC1` The quoted request uses the Revenue metric and names it in the answer

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
