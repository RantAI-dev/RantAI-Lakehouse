# Joins without writing SQL

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-7`, with the `BI-7` part of `AI-6` (assistant parity) |
| Status | Planner defaults; the owner asked for best practice and confirms at QA |
| Plan | `docs/superpowers/plans/2026-10-11-bi-7-joins.md` |

## Problem

A chart reads one table. Putting visits next to hotel capacity by province
needs a SQL source written by someone who may write SQL.

## What the user can do when this is done

1. In the chart builder, add up to four more tables to the chart's table (five in all) and choose for each: inner, left, right or full join.
2. Match on one or more pairs of columns, with equals or a comparison.
3. Get the matching columns suggested, from equal names and types and from the tables' sorting keys.
4. Use the joined tables' columns like any column: as dimension, breakdown, measure, filter, in a table or a pivot.
5. Trust permissions: masking and row filters of every table in the join apply.
6. Ask the assistant to join two tables.

## Not included

- Joining a SQL source (the base and the joined sides are tables of the serving layer).
- Calculated fields over a joined table's columns: a field belongs to one table (`BI-8`); fields of the chart's base table keep working.
- Unions.
- A saved, reusable joined dataset: the join is part of the chart (`BI-2` is where shared definitions live).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Up to five tables in one chart | Spec *(proposed)* | Owner to confirm at QA |
| 2 | The join belongs to the chart, not to a shared dataset | Planner default (Metabase) | Owner to confirm at QA |
| 3 | A joined column is shown as `table.column`; when a table is joined twice the second gets a number | Planner default | Owner to confirm at QA |
| 4 | A join the database refuses for size is reported as such on the tile; nothing is silently cut | Planner default | Owner to confirm at QA |
| 5 | The builder warns, without blocking, when the chosen pairs do not include a key of the joined table, because rows will multiply | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- Joining on columns that are not unique in the joined table multiplies rows and so sums; the builder warns but cannot know the data.
- Only equals conditions are fast; a join on comparisons alone is refused.
- A join is not remembered between charts.

## Acceptance checklist

- `BI-7-AC1` Joining visits to a second table by province gives the same numbers as the equivalent SQL.
- `BI-7-AC2` A masked column in the joined table stays masked, and a row filter on the joined table still removes its rows.
- `BI-7-AC3` A user without `dashboard:write` cannot save it; a failure shows a plain message.
- `BI-7-AC4` The record list of a clicked bar and the CSV export of a raw table show the joined columns.
- `AI-6-AC1` (the `BI-7` part) "visitors by province next to hotel capacity" is built by the assistant with a join.
