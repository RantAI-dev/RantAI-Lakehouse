# Reports with live charts, and dashboard templates

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-18` part D, with `AI-9` (its "templates" half) |
| Kind | Feature |
| Spec | `docs/core/specs/bi-18.md` (rows "Reports with charts" and "Templates") |
| Status | Planned |
| Priority | P2 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md` |
| Started | 2026-11-13 |
| Shipped | |
| Evidence | |

## Problem

A dashboard is a grid of tiles. People also need to **explain** numbers. A written page with
headings and paragraphs, where the charts stay live, is what Tableau Stories and Metabase
Documents give. And each new dashboard starts empty, so a team that has a good layout cannot
reuse it.

## What the user can do when this is done

1. **Reports:**
   - Create a **report**: a page of text (headings, paragraphs, bullet lists, links) with live
     charts placed between the paragraphs. The charts are picked from existing dashboards.
   - Read a report as any viewer. Each chart shows the viewer's own data, with masks and row
     filters applied, exactly as on its dashboard.
   - Keep reports in the same folders as dashboards, under the same access rules.
   - Download a report as a PDF made by the server, the same way as a dashboard PDF.
2. **Templates:**
   - **Save a dashboard as a template** (its layout, charts, filters and text tiles; no data).
   - **Start a new dashboard from a template**, from a "Templates" tab on the Dashboards page.
3. **The assistant:** ask it to "write a report on visitors this quarter with the trend chart",
   "add this chart to the report", or "start a dashboard from the Sales overview template".

## Not included

- Scheduled delivery of a report (`RPT-1` covers dashboards; reports come later).
- Built-in starter templates.
- Templates shared between deployments (a dashboard file, `BI-19` D, already moves a dashboard
  across).
- Comments on reports.
- Version history for reports.

## Asking the assistant

The assistant can create a report, add text and add a chart by reference, list templates, and
start a dashboard from a template. These are low-risk writes behind `dashboard:write`. Deleting
a report or a template needs approval, as deleting a dashboard does (assistant parity,
AGENTS.md).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| R1 | A report is text blocks (heading, paragraph, list, link) and chart blocks. A chart block is a reference to an existing chart, so it stays live and follows the viewer's policy | Planner (Metabase Documents) | Owner, 2026-11-13 |
| R2 | Permissions as dashboards: `dashboard:read` to view and `dashboard:write` to edit. Reports live in dashboard folders, with folder access | Planner | Owner, 2026-11-13 |
| R3 | A report downloads as a server-made PDF through the existing print path. No scheduled sends in this phase | Planner | Owner, 2026-11-13 |
| R4 | At most 30 chart blocks per report | Planner | Owner, 2026-11-13 |
| R5 | The assistant creates and edits reports (tools listed in the plan) | AGENTS.md assistant parity | Owner, 2026-11-13 |
| T1 | A template is a saved copy of a dashboard's definition (layout, charts, filters, text tiles), without data or snapshots | Planner (Tableau, Metabase) | Owner, 2026-11-13 |
| T2 | A dashboard made from a template is an independent copy, in this deployment. A chart whose table is missing shows the board's honest "source not available" sentence, never an empty chart | Planner | Owner, 2026-11-13 |
| T3 | No built-in templates in this phase | Planner | Owner, 2026-11-13 |
| T4 | Saving a template and starting from one need `dashboard:write`. Templates show on a "Templates" tab of the Dashboards page | Planner | Owner, 2026-11-13 |
| T5 | The assistant lists templates and starts a dashboard from one | AGENTS.md assistant parity | Owner, 2026-11-13 |

## Limits to tell a customer

- A report holds up to 30 charts.
- Reports have no schedule and no version history yet.
- Templates stay inside one deployment.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `BI-18D-AC1` | Create a report with a heading, two paragraphs, a list, a link and two charts from different dashboards | The report saves and reloads with the same blocks in order, and the charts draw live | Not run |
| `BI-18D-AC2` | Open the report as a role with a mask and a row filter on one chart's table | That chart shows the masked, filtered data, as on its dashboard | Not run |
| `BI-18D-AC3` | Open a report in a folder the person cannot read | Refused with the board's not-found sentence | Not run |
| `BI-18D-AC4` | Try to add a 31st chart | Refused with a sentence that names the limit | Not run |
| `BI-18D-AC5` | Download the report as a PDF | Text and charts are in the PDF in order | Not run |
| `BI-18D-AC6` | Save a dashboard with tabs, filters and a text tile as a template, then start a new dashboard from it | The new dashboard is an independent copy with the same layout, charts and filters | Not run |
| `BI-18D-AC7` | Start from a template whose chart table was dropped | That tile shows "source not available", and the rest draws | Not run |
| `BI-18D-AC8` | As a viewer with only `dashboard:read` | No "Save as template" and no "New report"; the server refuses both with 403 | Not run |
| `BI-18D-AC9` | Ask the assistant to write a short report with one chart, then to start a dashboard from a template | Both are done; the chat names what it made | Not run |

## After acceptance

`BI-18` becomes In Acceptance. `PRODUCT.md` lists reports and templates.
