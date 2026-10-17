# Dashboard layout and organisation

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-18` part C, with `AI-9` (assistant parity: tabs and templates) |
| Status | Planner defaults; the owner asked for best practice and confirms at QA |
| Plan | `docs/superpowers/plans/2026-10-14-bi-18c-layout.md` |

## Problem

A dashboard is one long page. It cannot be split into tabs or sections,
its tiles do not stack on a phone, and copying one drops its filters (and
fails on the built-in board). Nothing can be undone: a changed or deleted
dashboard or chart is gone. Every dashboard starts empty, and finding one
means the browse page.

## What the user can do when this is done

Part 1, layout:

1. Split a dashboard into tabs; each dashboard filter applies to every tab or to one tab.
2. Add headings, text and dividers between tiles, and fold a section away.
3. Read any dashboard on a phone: under 640 px the tiles stack in one column. An editor may arrange a separate phone layout.
4. Duplicate any dashboard, including the built-in one, with its tiles, tabs, filters and layout.

Part 2, safety and finding things:

5. See the last 15 versions of a dashboard with who changed it and when, and go back to any of them.
6. Restore a deleted dashboard or chart from the trash within 30 days.
7. Keep drafts in a personal space nobody else sees.
8. Mark favourites, see recent items, and search dashboards and charts from any page.

Part 3, templates and reports:

9. Save a dashboard as a template and start a new one from it.
10. Write a report: a page of text with live charts in it.

And ask the assistant to put charts in a tab, or to start a dashboard from a template.

## Not included

- Real-time co-editing.
- Version history of a single chart separate from its dashboard (a chart's change is a version of its dashboard; a chart on no dashboard keeps its trash entry only).
- Exporting a report as a document file (`BI-1`).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A dashboard without tabs works as today; adding the first tab puts every existing tile in it | Planner default | Owner to confirm at QA |
| 2 | A dashboard filter applies to every tab unless an editor ties it to one tab | Planner default | Owner to confirm at QA |
| 3 | Below 640 px tiles stack in their reading order; a separate phone layout is optional and replaces the stacking when present | Spec | 2026-10-14 |
| 4 | A version is saved on every saved change of the dashboard or of a chart on it; the last 15 are kept; going back is itself a new version | Spec (Metabase) | 2026-10-14 |
| 5 | Deleting moves to the trash for 30 days; after that it is removed for good. Restoring needs the same permission as deleting | Spec (Metabase) | 2026-10-14 |
| 6 | The personal space is visible to its owner and to administrators only; sharing a draft means moving it to a folder | Planner default | Owner to confirm at QA |
| 7 | A template keeps tiles, tabs, filters and layout but no data snapshot; a dashboard made from it is independent of it | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- Going back to an older version restores the dashboard as it was, including charts that were changed or deleted since (they come out of the trash). A chart that was already removed from the trash for good cannot come back: the step still works, says which chart it left out, and leaves no empty space. Charts added since move to the trash.
- The trash is emptied after 30 days; nothing older can be restored.
- A phone layout is per dashboard, not per device.

## Acceptance checklist

- `BI-18C-AC1` A dashboard with two tabs: a filter tied to one tab leaves the other tab's tiles unchanged; an untied filter changes both.
- `BI-18C-AC2` At 390 px wide the tiles stack in one column in reading order; with a phone layout, that layout is used instead.
- `BI-18C-AC3` Duplicating the built-in board and a board with tabs and filters gives a copy with all of them.
- `BI-18C-AC4` Five edits, then going back three versions, gives the dashboard as it was; the history lists who and when.
- `BI-18C-AC5` A deleted dashboard and a deleted chart come back from the trash; after 30 days they are gone.
- `BI-18C-AC6` A draft in the personal space is not visible to another non-administrator user.
- `BI-18C-AC7` A dashboard started from a template has the template's tiles and is independent of it.
- `BI-18C-AC8` A user without `dashboard:write` cannot change tabs, restore, or save templates; a failure shows a plain message.
- `AI-9-AC1` "Put the map charts in a separate tab." works.
