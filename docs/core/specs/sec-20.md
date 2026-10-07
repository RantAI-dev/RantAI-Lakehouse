# `SEC-20` Queries are scoped to the tenant

| | |
| --- | --- |
| Backlog | `SEC-20` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security, with Query Studio |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Query Studio runs have no tenant gate and saved queries are listed for everyone.

## What users get

A user in one organisation cannot query another's tables or see its saved queries.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Tenant gate on POST /api/query/run | None | The catalog tenant gate applies to every table a query reads |
| Saved queries list | Global | Per tenant |

## Benchmark

From the Data audit; not re-checked.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A user in tenant B cannot run SQL on tenant A's uploaded table
- [ ] A user in tenant B does not see tenant A's saved queries
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
