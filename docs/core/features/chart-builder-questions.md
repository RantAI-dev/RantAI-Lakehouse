# A more capable chart builder

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-6`, with the `BI-6` part of `AI-6` (assistant parity) |
| Status | Planner defaults; the owner asked for best practice and confirms at QA |
| Plan | `docs/superpowers/plans/2026-10-11-bi-6-chart-builder.md` |

## Problem

A chart takes one dimension, one optional breakdown, and measures that all
share one aggregation. It cannot be filtered on its own (only the whole
dashboard can), cannot be sorted by anything but its dimension or first
measure, has no "top 10 and the rest", and cannot ask a second question of
its own result ("only provinces whose total is over 1,000"). Anything more
needs a SQL source written by someone who may write SQL.

## What the user can do when this is done

Part 1:

1. Give each measure its own aggregation (sum of visitors next to the average stay).
2. Filter one chart on any column, with the same filter types as dashboard filters; the chart's own filters always apply, whatever the dashboard's filters are.
3. Sort by any column or measure, ascending or descending.
4. Keep the top or bottom N and, when wanted, gather the rest under "Other", computed from the underlying rows.
5. Group a table by up to five columns (a visual chart keeps one dimension and one breakdown; a pivot keeps its three plus two).

Part 2:

6. Add steps after the summary: filter the summarised result, or summarise it again, up to three steps in all.
7. See up to 2,000 rows of the result while building, beside the chart.
8. Ask the assistant for all of it in one sentence.

## Not included

- A drag-and-drop canvas (owner decision in the spec).
- Joins (`BI-7`), bins, groups and hierarchies (`BI-10`), parameters (`BI-11`), two sources on one chart (`BI-13`): the next items of this phase.
- More than two groupings on a visual chart.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Up to five groupings on a table; a visual chart stays at dimension plus breakdown | Spec *(proposed)*, narrowed for visual charts | Owner, 2026-11-13 |
| 2 | A chart's own filters are part of the chart: viewers see them listed on the tile but cannot remove them; dashboard filters add to them | Planner default (Metabase) | Owner, 2026-11-13 |
| 3 | "Other" is computed from the underlying rows, so an average or a distinct count of "Other" is right, not a sum of the hidden bars | Planner default | Owner, 2026-11-13 |
| 4 | Up to three steps in all (the summary and two more) | Planner default | Owner, 2026-11-13 |
| 5 | The builder shows up to 2,000 result rows | Spec | Owner, 2026-11-13 |
| 6 | A click on "Other" offers no drill-down; a click on any other value lists the rows with the chart's own filters applied | Planner default | Owner, 2026-11-13 |
| 7 | Top N of the breakdown on a bar, hbar, line or area chart: the limit keeps the N breakdown values with the largest total of the measure over the chart's filtered rows; the others are left out. A dimension with a time grain keeps every period | Owner, QA round 2026-11-12 (`BI-46`) | Accepted by the owner, 2026-11-12 (`BI-46`) |
| 8 | A dimension without a grain keeps its own top N, counted over the rows of the kept breakdown values | Planner default (open question 1) | *(proposed, not yet signed)* |
| 9 | A date breakdown can be grouped by day, week, month, quarter or year | Planner default (`BI-46`) | *(proposed, not yet signed)* |
| 10 | A radar chart takes one to six measures, each with its own aggregation, and draws one shape per measure on one shared scale; it has no breakdown | Planner default (QA round, `BI-49`) | changed 2026-11-12 (`BI-49`), *(proposed, not yet signed)* |

## Limits to tell a customer

- A sum is a whole number. An average, a minimum and a maximum keep up to six decimals; with no number format a value shows about four significant digits (1,466; 146.6; 14.66), and with decimals set it shows exactly those. A chart saved before this rule, with an average, a minimum or a maximum for the whole chart, shows whole numbers until it is saved again, unless a dashboard filter applies to it. *(proposed, not yet signed; `BI-55`)*
- A second step works on the summarised result, so it can use only the columns and measures of the step before it.
- A chart with steps or an "Other" group is rebuilt from its definition on every read; it cannot be edited as SQL.
- A chart with a time grain and a breakdown shows every period (up to the chart row limit); to keep only recent periods, add a relative date filter to the chart.
- The value list of a chart filter reads the chart's own table or SQL source only; while a SQL source is new or edited and not saved, no list is offered.
- On a radar chart every measure shares one scale, so a measure with much smaller numbers is drawn near the centre.

## Acceptance checklist

- `BI-6-AC1` Visitors by province and month, filtered to one category, top 10 provinces, is built without SQL.
- `BI-6-AC2` A second step filters the summarised result to totals over 1,000.
- `BI-6-AC3` A user without `dashboard:write` cannot save it; a failure shows a plain message.
- `BI-6-AC4` Top 5 with "Other": the bars add up to the table's own total, and an average measure's "Other" equals the average of the remaining rows.
- `BI-6-AC5` The chart's own filter still applies when the dashboard has a filter on the same column, and the record list of a clicked bar honours both.
- `AI-6-AC1` (the `BI-6` part) "Visitors by province and month, one category only, top 10" is built by the assistant in one go.
