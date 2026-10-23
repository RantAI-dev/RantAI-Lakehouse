# Forecasts on a time series and clusters on a scatter chart

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-17` part B, with `AI-8` (its "forecast lines" half) |
| Status | Draft (2026-10-24). Decision 1 waits for the owner's signature (questions-for-owner Q5); the rest are planner defaults the owner confirms at the phase 6 QA |
| Plan | `docs/superpowers/plans/2026-10-24-bi-17b-forecast-clustering.md` |
| Spec | `docs/core/specs/bi-17.md` part B |

## Problem

A chart over time can show a straight trend line (`BI-17` A) but cannot say
where the series is heading with its seasons, nor how sure that is. A scatter
chart shows points but not the groups they fall into.

## What the user can do when this is done

1. On a line, area or bar chart over dates, add a **Forecast** in the chart's Reference lines: choose how many periods ahead (for example 6 months), and see the forecast drawn after the last actual period with a shaded 95 % band.
2. Read in the tooltip and the chart's note which model was used ("trend and yearly seasons"), and the band's range for each future period.
3. See an honest sentence instead of a line when the data cannot carry a forecast ("A forecast needs at least 5 periods; this chart has 3").
4. On a scatter or bubble chart, switch on **Clusters**: points are coloured by group, with the number of groups chosen automatically or set by the user (2 to 10); the legend names each group with its size and centre.
5. Ask the assistant: "Add a 6-month forecast to this chart" or "Group these points into clusters".

## Not included

- Forecasting from columns other than the chart's own measure (no external drivers, no regression on other columns).
- Forecast numbers in downloads (Excel, CSV): downloads carry the actual data only, as today. The PDF shows the forecast as drawn.
- Saving cluster membership back to a table.
- Forecasts on charts without a date axis, on pivots and tables.

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | **Method.** Exponential smoothing, chosen automatically like Tableau: the product tries level only, level and trend, level and seasons, and level, trend and seasons (additive), and keeps the one that fits best by a standard score (AICc). Seasons follow the chart's period: 12 for months, 4 for quarters, 7 for days, 52 for weeks, 24 for hours; none for years. The band is the model's own 95 % prediction interval. | **Owner to sign before building** (Q5) |
| 2 | **Honesty thresholds.** No forecast below 5 periods; seasons only with at least 2 full seasons of history; more than 10 % of periods missing → no forecast; up to 10 % missing are filled in a straight line and the note says how many. Each refusal is a sentence on the chart, never a line. | Planner default — owner to confirm at QA |
| 3 | **The current period is left out** when it is not finished (it contains today in the report time zone), and the note says so, because a half month looks like a fall. | Planner default — owner to confirm at QA |
| 4 | **Horizon.** 1 to 60 periods, and never more than the length of the history. | Planner default — owner to confirm at QA |
| 5 | **Where it is computed.** In the browser, from the rows the chart already has, like the trend line: no new query, the same numbers on screen and in the PDF. | Planner default — owner to confirm at QA |
| 6 | **Clusters.** k-means on the chart's two (or, for bubbles, three) measures, each scaled to the same range first; k chosen automatically from 2 to 10 by the Calinski–Harabasz score (Tableau's rule), or set by the user. Same points, same groups every time (a fixed start). At least 3 points per group, else fewer groups or a sentence. | Planner default — owner to confirm at QA |
| 7 | **Clusters use the points shown** (the chart's own rows, within its row limit); the note says "Clusters of the N points shown". | Planner default — owner to confirm at QA |
| 8 | **Saved with the chart**, edited in the chart builder (a Forecast entry in Reference lines; a Clusters section under Colours), and visible to everyone who can see the chart. No new permission: saving needs `dashboard:write` as for every chart change. | Planner default — owner to confirm at QA |

## Limits to tell a customer

- One forecast per chart, on one measure; 60 periods ahead at most.
- Additive seasons only: a series whose swings grow with its level is forecast less well (multiplicative models are not offered).
- Clusters are computed on the points on screen, not on the whole table.

## Asking the assistant

`AI-8`: `format_chart` already sets `referenceLines` and the style fields; its
schema gains the `forecast` line kind with `horizon`, and the `clusters`
style field (`k: "auto" | 2..10`). The assistant reads back the model and any
refusal sentence from the chart's note so it never claims a forecast that was
not drawn. Fixture regenerated; the standard request set gains "Add a 6-month
forecast to this chart" and "Group these points into clusters". The AI team
reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-17-AC4` A 24-month series shows a 6-month forecast with a band.
- `BI-17-AC5` A scatter chart colours points by cluster.
- `BI-17-AC6` A user without `dashboard:write` cannot save a forecast or clusters, and a failure shows a plain sentence.
- `BI-17-AC7` *(proposed)* A 3-month series shows the sentence "A forecast needs at least 5 periods", not a line.
- `AI-8-AC2` *(proposed)* "Add a 6-month forecast to this chart" adds it and the assistant names the model used.
