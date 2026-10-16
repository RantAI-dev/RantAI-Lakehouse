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
| 1 | Up to five groupings on a table; a visual chart stays at dimension plus breakdown | Spec *(proposed)*, narrowed for visual charts | Owner to confirm at QA |
| 2 | A chart's own filters are part of the chart: viewers see them listed on the tile but cannot remove them; dashboard filters add to them | Planner default (Metabase) | Owner to confirm at QA |
| 3 | "Other" is computed from the underlying rows, so an average or a distinct count of "Other" is right, not a sum of the hidden bars | Planner default | Owner to confirm at QA |
| 4 | Up to three steps in all (the summary and two more) | Planner default | Owner to confirm at QA |
| 5 | The builder shows up to 2,000 result rows | Spec | Owner to confirm at QA |
| 6 | A click on "Other" offers no drill-down; a click on any other value lists the rows with the chart's own filters applied | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- A second step works on the summarised result, so it can use only the columns and measures of the step before it.
- A chart with steps or an "Other" group is rebuilt from its definition on every read; it cannot be edited as SQL.

## Acceptance checklist

- `BI-6-AC1` Visitors by province and month, filtered to one category, top 10 provinces, is built without SQL.
- `BI-6-AC2` A second step filters the summarised result to totals over 1,000.
- `BI-6-AC3` A user without `dashboard:write` cannot save it; a failure shows a plain message.
- `BI-6-AC4` Top 5 with "Other": the bars add up to the table's own total, and an average measure's "Other" equals the average of the remaining rows.
- `BI-6-AC5` The chart's own filter still applies when the dashboard has a filter on the same column, and the record list of a clicked bar honours both.
- `AI-6-AC1` (the `BI-6` part) "Visitors by province and month, one category only, top 10" is built by the assistant in one go.
