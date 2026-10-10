# Calculated fields

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-8`, with `AI-4` (assistant parity) |
| Status | Planner defaults throughout; the owner confirms at QA (asked to move fast, 2026-10-11) |
| Plan | `docs/superpowers/plans/2026-10-11-bi-8-calculated-fields.md` |

## Problem

A chart can use a column only as it is stored. "Profit = revenue - cost",
"share of total" or "running total" need a SQL source written by someone
who may write SQL, and cannot be reused by the next chart.

## What the user can do when this is done

Part 1:

1. Write a formula on a table or SQL source, for example `[revenue] - [cost]` or `Sum([revenue]) / CountDistinct([customer])`, and save it as a named field.
2. Get suggestions for columns and functions while typing, with a line of help per function.
3. See a mistake marked at its position, and be unable to save a formula that does not check out.
4. Pick a saved field in any chart on that source, like a column; others with access to the source can pick it too.
5. Ask the assistant: "add profit as revenue minus cost"; it writes the formula, never SQL, and can explain an existing one.

Part 2:

6. Table calculations over a chart's result: running total, running count, previous value, percent of total, rank, moving average.
7. Compare with the previous period and with the same period last year, on a chart grouped by a time grain (`BI-9`).
8. Fixed-level aggregation: a value computed at chosen columns whatever the chart groups by, for ratios and cohorts.

## Not included

- Raw SQL inside a formula.
- Formulas across two sources (joins are `BI-1`).
- Governed metrics with owners and certification (`BI-2`); a saved field is the plain version.
- Formulas in dashboard filters.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | The product has its own small formula language: `[Column]` references, numbers, quoted text, operators, and a fixed list of functions. It is parsed and compiled by the server; nothing the user types is ever placed in SQL as it is | Spec | 2026-10-11 |
| 2 | A field belongs to one table or SQL source and is visible to everyone who can read that source; creating, changing and deleting need `dashboard:write` | Planner default | Owner to confirm at QA |
| 3 | A field is row-level (usable as a dimension or inside an aggregation) or aggregate (usable as a measure); the server works out which from the formula | Planner default | Owner to confirm at QA |
| 4 | A field that a chart uses cannot be deleted; the refusal lists the charts | Planner default | Owner to confirm at QA |
| 5 | Dividing by zero gives an empty value, not an error | Planner default | Owner to confirm at QA |
| 6 | Column permissions hold: a formula over a masked column sees the masked value, and over a denied column fails for that user | Security posture | 2026-10-11 |
| 7 | The function list is the Metabase set named in the spec; the exact names are in the plan | Spec | 2026-10-11 |

## Limits to tell a customer

- A field works on the source it was written for only.
- Table calculations follow the chart's own order (its dimension).
- "Same period last year" needs a chart grouped by month, quarter or year, or by day or week with a full year of data behind it.

## Acceptance checklist

- `BI-8-AC1` Profit as revenue minus cost appears as a field and charts correctly.
- `BI-8-AC2` A running total line matches the cumulative sum. (Part 2)
- `BI-8-AC3` A formula with a typo shows the error at its position and cannot be saved.
- `BI-8-AC4` A user without `dashboard:write` cannot create a field; a failure shows a plain message.
- `BI-8-AC5` A formula over a masked column shows the masked value to a masked role.
- `AI-4-AC1` "Add profit as revenue minus cost." creates the field through the formula language.
