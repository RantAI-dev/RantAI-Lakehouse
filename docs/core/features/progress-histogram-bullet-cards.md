# Progress, histogram, bullet, image and web cards

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-16` (part B), with `AI-5` for these types |
| Status | Planner defaults, built at the owner's request to build phase 3 straight through (2026-10-15); the owner confirms or changes each decision at the phase 3 QA |
| Plan | `docs/superpowers/plans/2026-10-15-bi-16b-simple-types.md` |

## Problem

A value cannot be shown against its goal as a bar, the spread of a number
column cannot be shown without building bins by hand, there is no bullet
graph, and a dashboard cannot show a picture or a web page (a logo, a
diagram, a status page).

## What the user can do when this is done

1. Add a progress bar: a value against a goal (a number or another value), with the percentage reached.
2. Add a histogram: pick a number column and see how many rows fall in each range; the ranges are chosen automatically, or the user sets their width or their number.
3. Add a bullet graph: a value, a target marker and up to three bands (for example poor, fair, good), for one value or one per category.
4. Add an image card: an image by its address, with a description for screen readers.
5. Add a web-page card: a page shown inside the dashboard.
6. As an administrator, in Admin > Settings: list the sites images and web pages may come from.
7. Ask the assistant for any of these.

## Not included

- Part C of `BI-16` (Gantt, world maps, uploaded boundaries, spatial analysis, chart plugins).
- Uploading an image file: images come from an address.
- A web page that refuses to be shown inside another page (most sign-in pages and many large sites do); the card then offers "Open in a new tab".

## Decisions

Planner defaults, all confirmed by the product owner on 2026-10-17 (phase 3 QA).

| # | Decision | Default |
| --- | --- | --- |
| 1 | Progress: one value and a goal, either a fixed number or a second value of the same source. Above 100 % the bar is full and the label says the real percentage (for example 112 %). A goal of zero or less shows "No goal to compare with". | Planner, confirmed by the owner 2026-10-17 |
| 2 | Histogram: one number column, counted per range. Automatic ranges use the same rounded widths as `BI-10` bins, aiming at about 20 ranges; the user may set a width or a number of ranges, at most 100. Empty ranges between two filled ones are shown as zero, so the shape is not distorted. | Planner, confirmed by the owner 2026-10-17 |
| 3 | Bullet: one value, an optional target (fixed or a second value) and up to three bands with fixed limits; with a category, one bullet per category, at most 20, the rest summarised as "N more not shown". | Planner, confirmed by the owner 2026-10-17 |
| 4 | Image and web cards take only `https://` addresses whose site is on the administrator's list of allowed content sites (Admin > Settings, at most 50 sites, written `https://host` or `https://host:port`, no wildcards, as for signed-embed sites). An address on another site is refused at save with a plain sentence, and a card whose site is later removed from the list shows "This site is no longer allowed" instead of loading it. | Planner, confirmed by the owner 2026-10-17 |
| 5 | A web page is shown in a sandboxed frame (scripts and pop-ups allowed, no access to the console, no referrer sent). The card always shows "Open in a new tab". | Planner, confirmed by the owner 2026-10-17 |
| 6 | An image card needs a short description (alt text, at most 200 characters). | Planner, confirmed by the owner 2026-10-17 |
| 7 | Image and web cards are layout tiles: they take no filter, like text and headings, and are shown in public links and signed embeds as on the dashboard. | Planner, confirmed by the owner 2026-10-17 |
| 8 | Progress, histogram and bullet take the dashboard's filters like any chart, and use the value formats of `BI-17` part A. | Planner, confirmed by the owner 2026-10-17 |
| 9 | The coverage check is a list in this page and a test: every Metabase chart type maps to one of ours or to a decision that drops it. | Planner, confirmed by the owner 2026-10-17 |
| 10 | Changing these cards needs `dashboard:write`; changing the allowed sites needs `settings:write`. | Planner, confirmed by the owner 2026-10-17 |

## Limits to tell a customer

- Images and web pages come only from sites an administrator lists, and
  only over `https`.
- Many sites refuse to be shown inside another page; the card then stays
  blank and offers "Open in a new tab".
- The allowed-sites list is checked when a card is saved and when it is
  shown. The console does not yet send a Content-Security-Policy that
  would enforce it in the browser as well; that is a separate backlog item.

## Coverage against Metabase

| Metabase | Ours |
| --- | --- |
| Table | Table, raw table (`BI-16` A) |
| Pivot table | Pivot (`BI-16` A) |
| Number | KPI |
| Trend | KPI with comparison and sparkline (`BI-16` A) |
| Detail | Not a tile kind: the one-record view with every field and Previous / Next in the record list that opens from a table, a chart or a KPI (`BI-16` A) |
| Progress | Progress (this part) |
| Gauge | Gauge |
| Line, bar, combo, area, row | Line, bar, combo, area, and the horizontal bar (`hbar`) for Metabase's row chart |
| Waterfall | Waterfall |
| Scatter, bubble | Scatter, bubble |
| Pie, donut | Pie (drawn as a donut; one kind, `pie`) |
| Funnel | Funnel |
| Maps | Region, point and density maps of Indonesia and Jakarta (`geomap`, `pointmap`, `geoheat`); world maps in `BI-16` C |
| Sankey | Sankey |
| Box plot | Box plot |

The developer checks this table against the code and corrects any row that does not hold. `src/lib/chart-coverage.test.ts` reads this table and fails when a row has no kind or decision there, or the code has no such kind.

## Asking the assistant

`create_chart` and `update_chart` accept the new kinds and their fields
(goal, bands, histogram width or count, image or page address and
description). An address outside the allowed sites is refused with the same
sentence as in the console. The standard request set gains "Show a progress
bar of visitors against a goal of 1,000,000" and "Show how hotel prices are
spread". The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-16-AC7` A progress bar shows visitors against a goal, with the percentage, and above the goal says more than 100 %.
- `BI-16-AC8` A histogram of a number column shows about 20 ranges automatically, and 10 when so set.
- `BI-16-AC9` A bullet graph per province shows value, target and three bands.
- `BI-16-AC10` An image card from an allowed site shows; an address on another site is refused; a web-page card from an allowed site shows with "Open in a new tab".
- `BI-16-AC6` A user without `dashboard:write` cannot add these cards, a user without `settings:write` cannot change the allowed sites, and a failure shows a plain sentence.
- `AI-5` The assistant creates a progress bar and a histogram from the two requests above.
