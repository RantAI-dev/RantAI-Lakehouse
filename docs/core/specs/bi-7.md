# `BI-7` Joins without writing SQL

| | |
| --- | --- |
| Backlog | `BI-7` in [BACKLOG.md](../BACKLOG.md) |
| Area | Dashboards |
| Who builds it | Dashboards |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Combining two tables needs hand-written SQL.

## What users get

Join tables in the builder by picking them and the columns to match.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Join types | Only by writing SQL | Inner, left, right, full |
| Joins per question | n/a | Up to 5 tables *(proposed)* |
| Conditions | n/a | Several column pairs; equals and comparison operators |
| Suggestions | n/a | Suggest the matching columns from names and declared keys |
| Permissions | n/a | Masking and row filters apply to every joined table |

## Benchmark

Metabase joins: inner, left, right, full, several joins and conditions, all plans. Tableau relationships and joins.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] Joining visits to hotels by province gives the same numbers as the equivalent SQL
- [ ] A masked column in the joined table stays masked
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Ships with [`AI-6`](ai-6.md) (Richer charts), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
