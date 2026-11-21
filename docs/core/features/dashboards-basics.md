# Dashboards, the basics: create, browse, every chart type, threshold alerts

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-56` (and `BI-66`, the timing now in section 5 of `docs/plans/BI-4-RESULT.md`) |
| Kind | Feature |
| Spec | none: the row came from the feature-row audit of 2026-11-13, which found the base of the module without an acceptance run. The other `BI-` rows cover only additions |
| Status | Released (accepted by the product owner on 2026-11-21) |
| Priority | P2 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | This page; the run is an acceptance run, not a build |
| Started | 2026-11-21 |
| Shipped | Before 2026-11-13 (the base of the module) |
| Evidence | Acceptance run of 2026-11-21 on the QA stack, built from the QA integration branch `qa/integration-2026-11-21` (tip `3fd675ee9`), by an agent (API and headless Chrome; gallery board with one tile per kind the builder offers — 33 kinds, `src/features/dashboards/chart-kinds.ts:21`); the owner looked at the gallery and found it tidy. The run's report and screenshots are held outside this repository |

## What the user can do

1. **Make and keep dashboards.** Create a dashboard, rename it, give it a description,
   duplicate it, move it to a folder, and move it to the trash and back.
2. **Find dashboards.** The Dashboards page lists them; search finds one by name; a
   favourite is marked and found again; the list switches between Table and Gallery
   (`BI-59` covers the page's own layout).
3. **Draw every chart type with real data.** The builder offers 33 kinds, from bars and lines
   to maps, tables and text tiles. Each one draws from a mart or a SQL source on the QA stack,
   its numbers match the same question asked in Query Studio, and hovering it shows a tooltip
   with the value.
4. **Be told when a number crosses a line.** A threshold alert on a metric is created,
   tested from the Alerts page, fires when the value is over the threshold, does not fire
   when it is under, and its e-mail arrives.
5. **See without changing.** A Dashboard Viewer opens a shared dashboard and reads every
   tile, and has no way to edit, move or delete it (no control shown; the API refuses).
6. **Read it on a phone.** A dashboard at 390 px wide stacks its tiles, every tile is
   readable, and the page never scrolls sideways.

## Not included

- Anything an addition row owns (filters `BI-18`, formatting `BI-17`, embedding `BI-26`,
  reports `RPT-1`, digests `BI-19`, the Dashboards page layout `BI-59`). They have their own
  checklists.
- Import from other BI tools (`BI-3`) and the rest of the BI matrix (`BI-20`): the owner
  deferred both on 2026-11-13.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | The acceptance run is done by an agent on the QA stack (API and headless Chrome, screenshots in the report); the owner looks at one gallery dashboard with every chart type and at the report | Planner | Owner, 2026-11-21 ("okeay" to the checklist) |
| D2 | "Every chart type" means every kind the chart builder offers on the run's commit (the QA integration branch in Evidence), listed in the report with its result; a kind that cannot draw is a finding, not a skip | Planner | Owner, 2026-11-21 |
| D3 | `BI-4-AC3` timing (`BI-66`): a real dashboard of 12 tiles, opened 5 times cold (no remembered result) and 5 times with remembered results, median and range written to `docs/plans/BI-4-RESULT.md` with the method | Planner | Owner, 2026-11-21 |

## Acceptance checklist

- [x] `BI-56-AC1`: create, rename, describe, duplicate, move to a folder, move to the trash
  and restore a dashboard; each step shows on the Dashboards page.
- [x] `BI-56-AC2`: search finds a dashboard by part of its name; a favourite is marked and
  listed; Table and Gallery both show it.
- [x] `BI-56-AC3`: every chart kind the builder offers draws on one gallery dashboard with
  real data (the builder offers 33 kinds on the run's build, `src/features/dashboards/chart-kinds.ts:21`;
  the run's per-kind list is in the report, held outside the repo), and the owner finds the
  gallery tidy.
- [x] `BI-56-AC4`: for at least five kinds (bar, line, pie, table, KPI), the numbers equal
  the same question run in Query Studio.
- [x] `BI-56-AC5`: hovering each drawn chart shows a tooltip with its value. 25 of the 26
  charts the run checked did (the 33 tiles include non-chart types); the density map has
  none by design, and gets one in `BI-73` with the other findings of the run (owner,
  2026-11-21).
- [x] `BI-56-AC6`: a threshold alert fires over its threshold and not under it, from
  "Test run now", and its e-mail arrives in the QA mailbox. Pass through the API route
  behind the button; the button itself was not clicked (see Result).
- [x] `BI-56-AC7`: a Dashboard Viewer reads a shared dashboard and sees no edit, move or
  delete control; the API refuses an edit from that account.
- [x] `BI-56-AC8`: at 390 px wide the gallery dashboard stacks, every tile is readable, and
  the page does not scroll sideways.
- [x] `BI-66` (`BI-4-AC3`): the 12-tile cold and remembered timing (`BI-56` D3) is in section 5
  of `docs/plans/BI-4-RESULT.md`.

**Acceptor** (not the Owner): the product owner (recorded in the base) **Date:** 2026-11-21
**Build:** QA integration branch `qa/integration-2026-11-21`, tip `3fd675ee9`

## Result (2026-11-21)

Seven items passed outright; AC3 passed on the owner's look at the gallery; AC5 passed with
the density map's missing tooltip moved to `BI-73`. No item failed. Checked through the API
rather than the button: trash and restore, and "Test run now" (the same routes). Phone
widths were emulated (390 px), not a real phone.

Findings the owner chose to fix, all in `BI-73`: category labels dropped on a narrow tile,
rotated labels clipped at the left, smoothed lines over unordered categories, the Gantt
"Today" label over the axis, the alert e-mail's number format and emoji, and the density
map's tooltip. Left as they are (owner): a long Sankey label and the pivot's in-tile scroll
on a phone, maps filling half their tile, and the Version history, Subscriptions and Export
items a Dashboard Viewer sees in the settings menu (no edit item is among them).
