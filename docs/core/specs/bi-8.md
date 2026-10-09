# `BI-8` Calculated fields

| | |
| --- | --- |
| Backlog | `BI-8` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Charts can only use plain column names; there are no formulas.

## What users get

Write formulas for new columns and measures, checked before saving.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Functions | Only plain column names | At least the Metabase set: maths, text, dates, conditions (case, if, coalesce), type conversion, aggregations (sum, count, distinct, average, median, percentile, min, max, standard deviation, conditional aggregations) |
| Window functions | None | Running total, running count, offset (previous row), percent of total, rank, moving average |
| Period comparisons | None | Compare with the previous period and the same period last year |
| Level of detail | None | Fixed-level aggregation (Tableau FIXED) for ratios and cohorts |
| Editor | n/a | Autocomplete for columns and functions, inline help per function |
| Validation | n/a | Errors shown with position before saving; formulas compile to bound SQL, never raw text spliced in |
| Reuse | n/a | A saved formula is a field others can pick (ties to BI-2 metrics) |

## Benchmark

Metabase custom expressions: about 100 functions (maths, text, date, logic, aggregations, cumulative and offset). Tableau: row-level, aggregate, table calculations and LOD expressions.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BI-8-AC1` Profit as revenue minus cost appears as a field and charts correctly
- `BI-8-AC2` A running total line matches the cumulative sum
- `BI-8-AC3` A formula with a typo shows the error and cannot be saved
- `BI-8-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Ships with [`AI-4`](ai-4.md) (Calculated fields), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
