# `SEC-21` Downloads and big results are safe

| | |
| --- | --- |
| Backlog | `SEC-21` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security, with Query Studio |
| When | Next |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

A re-download re-runs SQL rewritten for the policy at run time, and the API holds the whole result in memory before cutting to 2,000 rows.

## What users get

A download always applies today's masks, and a huge result cannot exhaust the API.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Re-download | Uses the SQL rewritten at run time | Re-applies the current policy to the original SQL |
| Result size | Whole result buffered, then cut | Capped in the engine (max_result_rows / LIMIT) before it reaches the API |

## Benchmark

From the Data audit; not re-checked.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A mask added after a run applies to its re-download
- [ ] SELECT * on a very large table does not raise the API's memory beyond the cap
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
