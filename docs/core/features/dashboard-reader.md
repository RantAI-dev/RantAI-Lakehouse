# The dashboard page: a focused reader with an edit mode

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-61` |
| Kind | Feature |
| Spec | `docs/core/specs/bi-61.md` |
| Status | Building |
| Priority | P1 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | `docs/superpowers/plans/2026-11-14-bi-61-dashboard-page.md` |
| Started | 2026-11-14 |
| Shipped | |
| Evidence | |

## What the user can do when this is done

1. **Read a dashboard on a calm page.**
   - **Header:** a breadcrumb (Dashboards › folder), the title with its star, and one line of
     facts (owner · updated · data from · N comments).
   - **On the right:** Edit (a pencil, for writers), **Share** and ⋯.
2. **Filter without leaving the page.**
   - Tabs and the filters sit on one quiet row under the header.
   - Each filter and parameter is a borderless chip ("Years 2020–2024 ⌄", "Top 5 ⌄") that opens a
     small panel; a changed chip is tinted, and one Reset appears when anything differs from the
     defaults.
3. **Read the tiles.**
   - Tiles are soft-filled with no border and no badge.
   - Hovering a tile shows its ⋯ and a quiet "Data from 12:47" line.
   - The tile ⋯ holds: edit, refresh, view data, download (CSV, XLSX, PNG), alert, lineage, and
     the tile's refresh mode.
4. **Use the grouped ⋯ menu:**
   - **View:** refresh now, full screen, auto-refresh ›, freshness;
   - **Get a copy:** download PDF, print, subscriptions, export file, export YAML;
   - **Manage:** rename, duplicate, theme and font, version history, save as template, delete.
5. **Edit as a mode.** The pencil turns the page into edit mode: Add chart (primary), Done, drag
   handles, "+ Filter" and "+ tab". Done returns to reading.
6. **Use one side panel.** Comments (from the facts line) and Details open in the right side
   panel shared with the AI Copilot, as tabs; opening one replaces what the panel shows.

## Not included

- Mockup B's always-on workspace panel, and mockup C's report-style reading mode.
- New dashboard features; every existing one stays reachable.

## Asking the assistant

No tool changes. The page context the dashboard sends to the assistant stays the same.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | Layout A from the mockups of 2026-11-14 (branch `design/chart-builder-mockups`, `c80a6172`, local) | Owner's choice | Owner, 2026-11-14 |
| D2 | One side panel at a time: Comments and Details share the AI Copilot's right panel as tabs | Owner's rule (DESIGN.md) | Owner, 2026-11-14 |
| D3 | Built on `BI-18` part D (#528), because the ⋯ menu includes "Save as template"; the PR follows #528 | Planner | n/a |

## Acceptance checklist

- [ ] `BI-61-AC1`: in reading mode the header shows Edit, Share and ⋯ only (Edit for writers only).
- [ ] `BI-61-AC2`: the ⋯ menu is grouped as above, with auto-refresh as one submenu, and every old
  entry is still reachable.
- [ ] `BI-61-AC3`: no tile shows a badge in reading mode; the tile ⋯ holds the refresh mode and the
  downloads.
- [ ] `BI-61-AC4`: Comments and the AI Copilot never show as two panels at once.
- [ ] `BI-61-AC5`: at 1190×800, 1366×900 and phone width, in light and dark, nothing overlaps and the
  composer covers no tile.
