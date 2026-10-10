# Tables and KPI cards

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-16` (part A), with `AI-5` for these types |
| Status | Planner defaults throughout; the owner confirms at QA (asked to move fast, 2026-10-11) |
| Plan | `docs/superpowers/plans/2026-10-11-bi-16a-tables-kpi.md` |

## Problem

A table tile is always a grouped summary of at most 100 rows: the rows
themselves cannot be shown, columns cannot be formatted, hidden or
reordered. There is no pivot table. A KPI tile is one big number with
nothing to compare it to. A row in the drill-down list cannot be opened.

## What the user can do when this is done

1. Make a table of raw rows: choose the columns, page through the whole result, sort by a column.
2. Set each column's label, format (number, percent, currency, date, link, image), width, wrapping, and hide or reorder columns.
3. Make a pivot table: up to three row fields, two column fields and five values, with totals and subtotals, and collapse a group.
4. Give a KPI a comparison: with the previous period (day, week, month, quarter, year on a date column) or with a goal, shown as an amount and a percent, coloured for better or worse, with a small trend line.
5. Open one record from a record list and step to the next or previous one.
6. Ask the assistant for any of these.

## Not included

- Parts B and C of `BI-16` (progress, histogram, bullet, image and web cards, Gantt, more maps).
- Editing data from a table.
- Conditional colouring of table cells.
- Paging a raw table inside an embed or public link: they show the first page (`BI-26`).
- Calculated columns (`BI-8`).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A raw table pages 50 rows at a time with the total, like the record list | Planner default | Owner to confirm at QA |
| 2 | A pivot shows at most 10,000 cells; beyond that it shows the first ones and says it was cut | Spec *(proposed)* | Owner to confirm at QA |
| 3 | Totals and subtotals are computed by the database over the underlying rows, never by adding up the cells (an average of averages would be wrong) | Planner default | Owner to confirm at QA |
| 4 | "Previous period" compares the latest period that has data with the one before it, in the report time zone of `BI-9` | Planner default (Metabase trend) | Owner to confirm at QA |
| 5 | The editor says whether up is good or down is good; the default is up | Planner default | Owner to confirm at QA |
| 6 | A link cell opens `http`/`https` in a new tab; an image cell loads `https` only and sends no referrer. Anything else is shown as text | Planner default | Owner to confirm at QA |
| 7 | Currency is shown with the browser's Indonesian formatting (`Rp`), no conversion | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- A pivot beyond 10,000 cells is cut, not paged.
- An image cell makes the viewer's browser fetch the image from wherever the data says.
- A comparison needs a date or timestamp column; without one, only a goal can be compared.

## Acceptance checklist

- `BI-16A-AC1` A raw table of a 237-row table pages to the last row and its total says 237; sorting by a column reorders across pages.
- `BI-16A-AC2` A column set to currency, one hidden and two swapped stay that way after reload.
- `BI-16A-AC3` A pivot by province and month with totals: the grand total equals the table's own sum.
- `BI-16A-AC4` A KPI comparing months shows the change as amount and percent, coloured, with a trend line; with a goal it shows the distance to the goal.
- `BI-16A-AC5` A record opens from the list and next/previous walk the list.
- `BI-16A-AC6` A user without `dashboard:write` cannot save any of it; a failure shows a plain message.
- `AI-5-AC1` "Show this as a pivot table with totals." works.
