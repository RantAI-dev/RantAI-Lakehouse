# Combine two sources on one chart

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-13`, with the `BI-13` part of `AI-6` (assistant parity) |
| Status | Planner defaults; the owner asked for best practice and confirms at the end-of-phase QA |
| Plan | `docs/superpowers/plans/2026-10-11-bi-13-combined-charts.md` |

## Problem

A chart reads one table or one SQL source. Visitors next to hotel occupancy
by month, from two tables that should not be joined row by row, cannot be
drawn on one chart.

## What the user can do when this is done

1. Add up to four more questions to a bar, line, area or combo chart (five in all), each with its own source, measures and filters, matched on the chart's dimension.
2. Draw each as bars or a line and put it on the left or the right value axis.
3. See a second value axis when one is used, each with its own scale.
4. Click a point to list the rows behind it, from the source that point came from.
5. Ask the assistant for "visitors next to occupancy".

## Not included

- Matching on more than one dimension, or on dimensions of different kinds (a date with a text).
- A breakdown on a combined chart: each question is one or more plain series.
- Combining on pies, maps, tables, pivots and KPI cards.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Up to five questions on one chart | Spec *(proposed)* | Owner to confirm at QA |
| 2 | Questions are matched on the values of the dimension: dates by the chart's time grain, text by exact spelling. A value one question lacks is a gap for that question, not a zero | Planner default | Owner to confirm at QA |
| 3 | Each question is asked on its own, with its own permissions; one that fails shows its reason in the legend and the others still draw | Planner default | Owner to confirm at QA |
| 4 | A dashboard filter applies to every question that has the filtered column | Planner default (as tiles today) | Owner to confirm at QA |
| 5 | The chart's sort and limit follow the first question; the others supply values for the kept dimension values | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- Two questions line up only where their dimension values are spelled the same.
- A combined chart costs one query per question.

## Acceptance checklist

- `BI-13-AC1` Visitors (bars) and a rate from a second table (line, right axis) by month on one chart; each series equals its own single chart.
- `BI-13-AC2` A user without `dashboard:write` cannot save it; a question the viewer may not read shows its reason while the others draw.
- `BI-13-AC3` Clicking a bar lists rows from the first table; clicking a point of the line lists rows from the second.
- `AI-6-AC1` (the `BI-13` part) "visitors by month next to rooms" is built by the assistant.
