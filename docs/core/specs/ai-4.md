# `AI-4` Calculated fields

| | |
| --- | --- |
| Backlog | `AI-4` in [BACKLOG.md](../BACKLOG.md) |
| Module | AI Copilot |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`BI-8`](bi-8.md) (BI phase 1) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Formulas are where users most need help.

## What users get

Describe a calculation; the assistant writes the formula.

## A user says

> Add profit as revenue minus cost.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Write formulas | None | In the product's formula language, never raw SQL |
| Validation | n/a | The formula is validated before saving; errors are fixed or explained |
| Explain | n/a | Explains an existing formula in plain words |

For every capability: The assistant uses the same API and permissions as the user; anything that deletes, shares or publishes asks for approval first.

## Benchmark

BI-8.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `AI-4-AC1` The quoted request adds a working profit field
- `AI-4-AC2` An impossible request is explained, not faked

## Not included

- Anything not in the Target table.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
