# `BI-6` A more capable chart builder

| | |
| --- | --- |
| Backlog | `BI-6` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The builder takes one dimension, one optional breakdown, no filters, and sorting only by order and limit.

## What users get

Build any question the competitors' builders can, without SQL.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Dimensions | One plus one breakdown | Up to 5 groupings *(proposed)* |
| Measures | Not checked | Several measures on one chart, each with its own aggregation |
| Filters in the chart | None | Any number, on any column, with the filter types of BI-18 A |
| Sort and limit | Order and limit only | Sort on any column or measure, top/bottom N, with 'other' grouping |
| Multi-step | None | Summarise, then filter or summarise the result again (Metabase stages) |
| Preview | Not checked | Live preview of up to 2,000 rows while building |

## Benchmark

Metabase notebook editor: filters, summaries, multiple groupings, joins, custom columns, sort, limit, and further steps on the result. Tableau shelves.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BI-6-AC1` Visitors by province and month, filtered to domestic, top 10 provinces, is built without SQL
- `BI-6-AC2` A second step filters the summarised result to totals over 1,000
- `BI-6-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Drag-and-drop canvas (owner decision)

## Asking the assistant

Ships with [`AI-6`](ai-6.md) (Richer charts), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
