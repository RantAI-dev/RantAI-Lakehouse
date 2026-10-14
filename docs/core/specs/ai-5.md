# `AI-5` Every new chart type

| | |
| --- | --- |
| Backlog | `AI-5` in [BACKLOG.md](../BACKLOG.md) |
| Module | AI Copilot |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-16`](bi-16.md) (BI phase 1, 3, 6) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

A chart type the assistant cannot make is a broken promise.

## What users get

Ask for any chart type the product has.

## A user says

> Show this as a pivot table with totals.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Chart tools schema | Current types only | Each new type added in the same change that adds it (assistant parity) |
| Choosing a type | Not checked | Suggests a suitable type when the user does not name one |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-16.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `AI-5-AC1` The quoted request makes a pivot with totals
- `AI-5-AC2` Each BI-16 part's PR includes the assistant schema change

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
