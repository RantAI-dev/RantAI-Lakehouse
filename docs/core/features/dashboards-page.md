# The Dashboards page: recent work, one list of everything, and a gallery view

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-59` |
| Kind | Feature |
| Spec | `docs/core/specs/bi-59.md` |
| Status | Building |
| Priority | P1 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | `docs/superpowers/plans/2026-11-14-bi-59-dashboards-page.md` |
| Started | 2026-11-14 |
| Shipped | |
| Evidence | |

## What the user can do when this is done

1. Open **Dashboards** and see:
   - a large search;
   - one **New** button: dashboard, report, from a template (each template with a small
     picture), import;
   - one ⋯ menu for metrics, folders and trash.
2. See **Recent**: the four items opened last, each as a small picture of its layout.
3. See **one list** of dashboards, reports and templates. Filter it with **All / Dashboards /
   Reports / Templates** and a folder picker. The columns are name, type, folder, owner,
   updated and sharing. Every column sorts, and the ⋯ menu is in the same place on every row,
   built-in Main included.
4. Switch the list between **Table** and **Gallery**. The gallery shows each item as a picture
   of its layout (each tile's chart kind in its place), grouped by folder, and the page
   remembers the choice.
5. Tick rows to **move to a folder**, favourite, or **move to trash** in bulk.
6. See a useful fact where "No description." used to be: "Made from the template QA BI", a
   report's first sentence, or nothing.

## Not included

- The calm-library rail (mockup A) and the "Continue where you left off" heading (mockup B).
  The Recent row covers it.
- Changes to the dashboard page itself, or to sharing, folders' access rules or trash.

## Asking the assistant

No tool changes: the list reads the same routes. The assistant can already list boards,
reports (`list_reports`) and templates (`list_templates`).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| P1 | Layout C (home and table) from the mockups of 2026-11-14 (branch `design/chart-builder-mockups`, `df0f38bb`, local) | Owner's choice | Owner, 2026-11-14 |
| P2 | B's picture cards are the Gallery view of the same list; Table is the default; the choice is remembered per browser | Owner's choice (gallery as a second view); default view by the planner | Owner, 2026-11-14; default to confirm at QA |
| P3 | A picture is drawn from the board's chart definitions (each tile's kind and position, in the chart colours, with sample shapes, no numbers), read once for the whole list (`GET /api/dashboard/specs`), so the gallery runs no query. The dashboard read cannot be narrowed to a few tiles (`DashboardQuery::only` is `serde(skip)`), and running every board's tiles for a list is too heavy. Real-number pictures are `BI-60` | Planner | Owner to confirm at QA |
| P4 | Badge meanings and colours: built-in grey, public link amber, embedded sky, restricted rose; soft fill, no border (DESIGN.md) | Planner (mockup) | Owner to confirm at QA |
| P5 | Built on `BI-18` part D (#528), which adds reports and templates to this page; the PR follows #528 | Planner | n/a |

## Acceptance checklist

- [ ] `BI-59-AC1`: the header and toolbar show New, search, the type filter, the folder picker,
  the view switch and one ⋯ menu, and no other button.
- [ ] `BI-59-AC2`: one list holds dashboards, reports and templates; each type filter shows only
  its kind; sorting works on every column.
- [ ] `BI-59-AC3`: the Gallery view shows a picture for each dashboard on screen, and the view
  choice survives a reload.
- [ ] `BI-59-AC4`: bulk move to folder, favourite and move to trash work, and trash can still be
  restored.
- [ ] `BI-59-AC5`: no "No description." text anywhere; the badges match P4.
- [ ] `BI-59-AC6`: at 1190×800, 1366×900 and phone width, in light and dark, nothing overlaps and
  the assistant bar covers no row.
