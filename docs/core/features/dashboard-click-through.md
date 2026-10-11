# Click-through and drill-down everywhere

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-18` (part B) |
| Status | Decisions signed 2026-10-10 |
| Plan | `docs/superpowers/plans/2026-10-10-bi-18b-click-through.md` |

## Problem

Clicking a value on a chart can list the rows behind it only for category
charts over a Gold table: not for maps, calendar, sankey, sunburst, box
plot, KPI, gauge or table tiles, and not for any chart on a SQL source.
The list stops at 100 rows and ignores the dashboard's active filters, so
its count can disagree with the number just clicked. A click cannot lead
anywhere else. Auto-refresh is remembered only by the browser tab, and
"full screen" is an overlay inside the page.

Already there, though the spec's "Today" column says otherwise:
cross-filtering (click a value, choose Filter).

## What the user can do when this is done

1. Click a value on any chart kind and see the rows behind it: a region on a map, a point on a point map, a day on a calendar, a node of a sankey or sunburst, a box of a box plot. KPI, gauge and table tiles offer the rows behind the whole tile.
2. Do the same on a chart built on a SQL source.
3. Page through all the rows, 50 at a time, with the total shown; the rows respect the dashboard's active filters.
4. As an editor, choose per chart what a click does: the drill menu (default), open another dashboard with the clicked value as its filter, open a saved query, or open a URL with the clicked value inserted.
5. As an editor, save an auto-refresh interval for a dashboard (off, 1, 5, 10, 15, 30 or 60 minutes); a viewer can change it for their own session.
6. Put a dashboard in real full screen, with a dark option for a wall display.
7. Keep cross-filtering from the click menu, as today.

## Not included

- Layout, tabs, phone layout, version history, trash: part C.
- Click destinations in embeds and public links: they keep the drill menu off, as today (`BI-26`).
- Passing more than the one clicked value to a destination.
- A scheduler that refreshes data in the background when nobody is looking.

## Asking the assistant

The assistant's chart tools gain the click setting, so "make a click on
this chart open the Sales dashboard filtered by region" can be asked. It
does not click for the user.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A click URL is `http`, `https` or a path inside the console; the clicked value is always URL-encoded; an external URL opens in a new tab. No domain allowlist: only someone who may edit the dashboard writes it | — | Owner, 2026-10-10 |
| 2 | The saved auto-refresh is the dashboard's default, set by an editor; a viewer may change it for their session | — | Owner, 2026-10-10 |
| 3 | The record list applies the dashboard's active filters | — | Owner, 2026-10-10 |
| 4 | Intervals are Metabase's: 1, 5, 10, 15, 30, 60 minutes | Spec | 2026-10-10 |
| 5 | 50 rows per page | Planner default | Owner to confirm at QA |
| 6 | The dark option in full screen is remembered per browser, not saved on the dashboard | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- A click passes one value. The destination dashboard must have a column of the name chosen for it, or the filter is shown as not applying.
- Rows behind a SQL-source chart come from that source's SQL, at most the source's own row limit.
- A text tile has nothing to click.
- Real full screen needs the browser to allow it; where it does not, the in-page view is used and says so.

## Acceptance checklist

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Click a region on a choropleth, a point on a point map, a day on a calendar, a sankey node, a sunburst ring, a box | The click menu, then the rows behind that value | |
| 2 | On a KPI, a gauge and a table tile, open the tile's menu | "View records" lists the rows behind the tile | |
| 3 | Click a value on a SQL-source chart | Rows from the source's SQL for that value | |
| 4 | Open a value with more than 50 rows | Total shown; next and previous page work; the last page is short | |
| 5 | Set a dashboard filter, then drill | The row count agrees with the value clicked | |
| 6 | `BI-18-AC4`: set a chart's click to open another dashboard on a column, click a bar | That dashboard opens filtered to the clicked value | |
| 7 | Set a click to a URL with the value placeholder, click a value containing a space and `&` | A new tab with the value encoded | |
| 8 | Try to save a click URL starting `javascript:` | Refused with a plain message | |
| 9 | Set a click to open a saved query | Query Studio opens that query | |
| 10 | Save auto-refresh 1 minute, reload as another user | The tiles refresh after a minute; the setting is shown | |
| 11 | Full screen, dark on, reload | Real full screen was entered; the dark choice is remembered | |
| 12 | `BI-18-AC7`: as a role without `dashboard:write`, try to change a chart's click or the saved refresh | Refused; clicking and viewing still work | |
| 13 | As a role with a masking policy on a column, drill | Masked values in the row list | |
| 14 | Open a dashboard saved before this change | Clicks behave as before (drill menu) | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
