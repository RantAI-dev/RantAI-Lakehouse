# Dashboard filters

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-18` (part A: filters). The backlog row is on the unmerged docs branch `claude/gracious-albattani-vw5wsb`; it is not in `BACKLOG.md` on this branch yet. |
| Status | Decisions signed 2026-10-07 |
| Plan | `docs/superpowers/plans/2026-10-07-dashboard-filters.md` |

## Problem

A dashboard can only be filtered by "column is one of these values", on
columns that happen to be a chart's dimension. There is no date range, no
number or text filter, the value list stops at 200 with no search, and the
year filter only exists for a column literally named `tahun`. Worse, any
editor who changes a filter saves it to the dashboard at once
(`dashboard-page.tsx`, `applyFilters`), so a passing look becomes the
default everyone sees, including public-link and embed viewers.

## What the user can do when this is done

1. Filter on any column of the tables and SQL sources the dashboard's charts read, not only chart dimensions.
2. Filter a date column by a from–to range, or relative to today (last N days, weeks, months, quarters, years; this or previous period).
3. Filter a number column by a range (either end open) or by a list of values.
4. Filter a text column by a list of values, or by contains, starts with, ends with.
5. Search inside a long value list, and be told when the list was cut short.
6. See a value list narrowed by the other active filters (choose a province, the city list shrinks).
7. Pick filter values that come from a chart built on a SQL source.
8. Change filters without changing the dashboard for anyone else; the filter state is in the page address, so a copied link opens the same view.
9. As an editor, press **Save as default** to make the current filters the dashboard's default. Anyone can press **Reset** to return to the default.
10. Filter the year like any other number column; the special Year chip is gone.
11. See on a tile when an active filter does not apply to it because the tile's data has no such column.

Added on 2026-10-10 to reach the target in `docs/core/specs/bi-18.md` (part A, second round):

12. Filter a date column by one specific date, before a date, after a date, or by the next N days, weeks, months, quarters or years.
13. Filter a date column by a month of a year or a quarter of a year, picked as such rather than typed as two dates.
14. Filter a number column by "equal to", "not equal to", "greater than" and "less than", named as such.
15. Filter a text column by "does not contain".
16. As an editor, mark a saved default filter as required: a viewer can change its value but cannot remove it, and clearing it returns to the default value.

## Not included

- Filter controls for public-link and embed viewers. They keep seeing the saved default; an embed token's locked `params` still cannot be overridden (`BI-26`).
- Parameters and SQL variables (`BI-11`); time grain (`BI-9`).
- Filters that reach a SQL-source chart through named parameters: the spec ties this to `QS-5`, which does not exist yet. A filter still applies to a SQL-source chart when the source returns a column of that name.
- Locking a filter in an embed beyond today's signed `params`: `BI-26`.
- Per-tile filters, and wiring one filter to differently named columns.
- Saving a default on the built-in "Main" dashboard (its filters were never stored).

## Asking the assistant

Not in this change. The assistant already receives the active filters as
page context and that summary is updated to describe the new kinds. It
cannot set or save dashboard filters; that needs a tool of its own.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Filter changes are temporary and mirrored in the URL; saving a default is an explicit action for editors | — | Owner, 2026-10-07 |
| 2 | All of part A ships as one PR | — | Owner, 2026-10-07 |
| 3 | The Year chip is folded into ordinary filters | — | Owner, 2026-10-07 |
| 4 | "Main" cannot save a default | Planner default, not objected | 2026-10-07 |
| 5 | A filter whose column a tile lacks is skipped for that tile and the tile says so | Planner default, not objected | 2026-10-07 |
| 6 | "Required" belongs to a saved default filter. It cannot be removed by a viewer; clearing it restores the default value. A required filter always has a value, so a dashboard never opens unfiltered by it | Planner default (the spec says only "a filter can be required") | Owner to confirm at QA |
| 7 | "Next N" counts from tomorrow and excludes today; "last N" keeps including today, as built | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- Relative dates ("today", "this month") follow the database server's clock and time zone, not the viewer's.
- A value list shows at most 200 values; use search to reach the rest.
- A filter applies to every tile whose data has a column of that name, and to no other tile.
- Public links and embeds show the saved default filters and cannot be changed by the viewer.

## Acceptance checklist

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Open a dashboard, add a filter on a text column, pick two values | Tiles with that column narrow; the address bar changes | |
| 2 | Copy the address into a new tab | Same filters applied | |
| 3 | Reload the dashboard without the query string, as another user | Saved default, not the filter from step 1 | |
| 4 | Add a date filter "last 30 days", then a from–to range | Tiles narrow accordingly; chip reads the range in words | |
| 5 | Add a number filter with only a lower bound | Applies; chip reads "≥ n" | |
| 6 | Add a text filter "contains" with a `%` in the text | Matches the literal `%`, not everything | |
| 7 | Open a value list with more than 200 values | Says the list is cut; search finds a value beyond it | |
| 8 | Filter province, then open the city list | Only cities of that province | |
| 9 | On a dashboard with a SQL-source chart, add a filter on a column only that source returns | Values listed; tile narrows | |
| 10 | As editor, press Save as default; open the public link | Public view shows the new default | |
| 11 | Press Reset after changing filters | Back to the default; query string cleared | |
| 12 | As a role without `dashboard:write` | No Save as default; filtering still works | |
| 13 | As a role with a masking policy on a column, open its value list | Masked values, never the clear ones | |
| 14 | Filter a column one tile lacks | That tile is unchanged and says the filter does not apply | |
| 15 | Open a dashboard saved before this change with filters and a year | Renders as before; old `{column, values}` filters still work | |
| 16 | Stop ClickHouse and open a value list (operator) | A plain "could not load values", no database text | |
| 17 | Date filter: "on" one date, "before", "after", "next 7 days" | Each narrows as named; the chip reads it in words | |
| 18 | Date filter: pick a month of a year, then a quarter of a year | The whole month or quarter, inclusive | |
| 19 | Number filter: equal to, not equal to, greater than, less than | Each narrows as named | |
| 20 | Text filter: does not contain | Rows with the text are gone; rows with an empty value stay | |
| 21 | Mark a saved default filter as required, save, reopen as a viewer and try to remove it | It cannot be removed; changing its value works; Reset returns the default | |
| 22 | Open a link saved before this round (`?f=…` with only the older filter kinds) | Same view as before | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
