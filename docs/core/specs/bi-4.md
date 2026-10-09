# `BI-4` Caching and parallel tiles

| | |
| --- | --- |
| Backlog | `BI-4` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Every dashboard request runs every query fresh, one tile after another.

## What users get

Dashboards open fast, and stay correct.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Result cache | Every request runs fresh | Cache per query and filter values, with a freshness rule: duration, or until the next Gold build *(proposed)*, or adaptive (Metabase) |
| Scope | n/a | Set for the install, per dashboard and per chart |
| Correctness | n/a | Cache keyed by the user's masking and row-filter policy, never shared across policies |
| Parallel tiles | One after another | Up to 6 tiles load at once *(proposed)* |
| Target | Not measured | A 12-tile dashboard opens in under 3 seconds on a warm cache, measured in a RESULT document *(proposed)* |
| Shown | n/a | Each tile says when its data was computed |

## Benchmark

Metabase caching: adaptive, duration, schedule, per database/dashboard/question (all plans for adaptive). Tableau extracts and query cache.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BI-4-AC1` A second open of a dashboard is served from the cache and says so
- `BI-4-AC2` Two users with different masks never see each other's cached results
- `BI-4-AC3` A RESULT document records the before/after timings
- `BI-4-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

No assistant counterpart is planned yet; the planner adds one before this is built (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
