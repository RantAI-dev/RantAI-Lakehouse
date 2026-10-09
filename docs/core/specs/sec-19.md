# `SEC-19` The query cost estimate runs only safe SQL

| | |
| --- | --- |
| Backlog | `SEC-19` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| When | Next |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

POST /api/query/estimate runs EXPLAIN ESTIMATE on raw SQL without the read-only check or the table-function block, and returns database error text.

## What users get

Estimating a query cannot make the engine fetch outside URLs or leak errors.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Checks before EXPLAIN | None | The same read-only check and sql_rewrite::enforce as a run |
| Error text | First 240 characters returned | Classified message |

## Benchmark

From the Data audit; not re-checked.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SEC-19-AC1` An estimate of SELECT * FROM url('http://...') is refused without contacting the URL
- `SEC-19-AC2` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
