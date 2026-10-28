# Where does this number come from: lineage from a chart to its sources

| | |
| --- | --- |
| Module | Dashboards (also Data: the lineage page and the asset page) |
| Backlog | `BI-14`, with `AI-13` in the same release |
| Status | Draft (2026-10-24). Decision 1 waits for the owner's signature (questions-for-owner Q6); the rest are planner defaults the owner confirms at the phase 6 QA. Not blocked: `DATA-13` (column lineage) is merged (#106, In Acceptance) |
| Plan | `docs/superpowers/plans/2026-10-24-bi-14-chart-lineage.md` |
| Spec | `docs/core/specs/bi-14.md`; assistant `docs/core/specs/ai-13.md` |

## Problem

The lineage page (Governance > Lineage) draws tables, connectors, uploads and
pipelines, and since `DATA-13` their columns, but it starts from a table. No
chart links to it, and a chart, a dashboard, a SQL source or a metric is not a
node on it. Nobody can go from a number on a dashboard to the connector it
came from, or from a Gold table to the dashboards that would change if it
changed.

## What the user can do when this is done

1. From any chart's menu, choose **Where does this come from** and see its path: the chart, its saved SQL source or metric, the Gold table, the Silver and Bronze tables it was built from, and the connector or upload at the start. For each measure and dimension, the columns it comes from, where column lineage was recorded.
2. See on each step when it was last refreshed (a Gold build, a Bronze load, a connector run), and which steps have no record ("not recorded", never a guess).
3. Open the full lineage page focused on any step.
4. From a table's page (Data Explorer) or the lineage page, see **Used by**: the charts and dashboards that read the table, directly or through a SQL source or metric, limited to those the person can see.
5. Ask the assistant: "Where does this number come from?" on a chart, and get the path with the source tables, the connector and the refresh times.

## Not included

- Lineage of a dashboard's filters and parameters (only the charts' data).
- Edges the platform did not record (a table filled outside the platform stays without an incoming edge, as today).
- Impact of a column change on charts by column (charts are listed by table; the column view shows the columns a chart's own fields use).

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | **Who sees it.** The chart's path needs `lineage:read` (Analyst and above today), like the lineage page. A Dashboard Viewer sees the menu item with the sentence "Lineage needs the lineage permission; ask your administrator", not the path. | **Signed by the owner, 2026-10-24** (Q6) |
| 2 | **What a chart's path starts from.** The chart's own record: its mart, its SQL source, its joined tables, its combined questions' sources (`BI-13`), and the metrics its measures use. Each is a node with its own kind (chart, SQL source, metric). | Planner default — owner to confirm at QA |
| 3 | **Refresh times** come from the records already kept: a Gold or Silver table's last build (the statement that last wrote it), a Bronze table's last change (the Iceberg table's last update), a connector's last successful run, an upload's time. A step without a record says "not recorded". | Planner default — owner to confirm at QA |
| 4 | **Columns.** Where `DATA-13` recorded column mappings, each of the chart's fields lists its source columns step by step; a field computed in a formula lists the columns the formula names. | Planner default — owner to confirm at QA |
| 5 | **Used by** lists charts and dashboards the person can see (their own spaces and shared ones; never another person's personal dashboard), up to 200, with the count of those not shown because they cannot see them ("and 3 more you do not have access to"). | Planner default — owner to confirm at QA |
| 6 | **Where it opens.** A side panel on the dashboard (the path as a vertical list, newest refresh time per step), with "Open in lineage" to the full page focused on the chart. The lineage page gains chart, SQL source and metric nodes. | Planner default — owner to confirm at QA |
| 7 | **Embeds and public links** show no lineage (no menu item). | Planner default — owner to confirm at QA |

## Limits to tell a customer

- Only recorded steps are drawn; a table loaded outside the platform has no source shown.
- Build times come from the engine's query log, kept for half a year; an older build shows "not recorded".
- Used by is limited to 200 charts per table.

## Asking the assistant

`AI-13`: a new read tool `get_chart_lineage { chart }` returns the same path
the panel shows (steps, kinds, refresh times, columns per field, gaps), under
the person's permissions; the dashboard page's context already names the
charts, so "Where does this number come from?" on a chart resolves to it.
`get_lineage` gains "used by" in its answer for a table. The prompt tells the
assistant to name the source tables, the connector and the refresh times, and
to say "not recorded" where the path stops. Fixture regenerated; the standard
request set gains the `AI-13-AC1` request. The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-14-AC1` A chart's lineage reaches the connector its data came from, with a refresh time on each step.
- `BI-14-AC2` A Gold table lists the dashboards that use it.
- `BI-14-AC3` A user without `lineage:read` is refused with the sentence, and a failure shows an honest message.
- `BI-14-AC4` *(proposed)* A chart over a SQL source shows the SQL source and the tables it reads, with the columns behind each field.
- `AI-13-AC1` "Where does this number come from?" names the source tables and the connector.
