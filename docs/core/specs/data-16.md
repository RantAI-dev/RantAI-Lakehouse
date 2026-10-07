# `DATA-16` Time travel proven safe

| | |
| --- | --- |
| Backlog | `DATA-16` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

The version picker is built but no test proves a past-version query still applies masking and row filters.

## What users get

Querying a past version of a table is trusted to apply masking.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Masking gate test | None | A gate test proves past-version queries on both engines pass the masking and row-filter rewrite |
| Picker | Built, unproven | Works for ClickHouse and Trino; versions listed with time and operation |
| Retention shown | Not stated | The page states how far back versions exist on this install |

## Benchmark

Supersedes DATA-7. Snowflake: Time Travel 1 day on all editions, up to 90 days on Enterprise.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A masked column stays masked when queried at an older version, on both engines
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
