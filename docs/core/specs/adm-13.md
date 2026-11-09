# `ADM-13` Each role sees only the pages it may use

| | |
| --- | --- |
| Backlog | `ADM-13` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code at `dcaae8a5` on 2026-11-09. Feature page [`pages-per-role.md`](../features/pages-per-role.md) |

## Why

Every signed-in person sees every page in the sidebar and the command palette and then hits errors on the pages they may not use (`app-sidebar.tsx` uses the full `NAV_GROUPS`; there is no route guard). Every signed-in person may use the assistant (`/api/ai/*` is `RequiresAuth`). The System status, Alerts, Audit Log and Data Quality lists are readable by any login (`/api/ops/{kind}`, `/api/alerts`, `/api/governance/{kind}` are `RequiresAuth`), so a "dashboards only" person cannot exist.

## What users get

A person sees only the pages their roles allow. A page they may not use is not in the sidebar or the palette, and opening its address shows "You don't have access". Two new ready-made roles: Dashboard Viewer as "dashboards only", and Assistant only, which can ask the assistant about the data and dashboards it may read but cannot change anything.

## Target specs

"Today" is `main` at `dcaae8a5`, read from the code.

| Capability | Today | Target |
| --- | --- | --- |
| Sidebar and command palette | Every page for everyone | Only pages the person's permissions open |
| Opening a page by address | Renders, then API errors | "You don't have access" panel, no data request made |
| Pages with no entry in the access map | n/a | Refused (fail closed); a test fails the build if a page has no entry |
| Assistant | Any login | `assistant:use` (full, as today) or `assistant:ask` (read-only, Ask mode forced) |
| System status, Alerts, Audit Log and Data Quality lists | Any login | `ops:read`, `alert:read` (or `alert:write`), `governance:read`, enforced by the API |
| Dashboard Viewer role | `dashboard:read`, and the assistant through `RequiresAuth` | `dashboard:read` only: Dashboards and nothing else |
| Assistant only role | Does not exist | `assistant:ask, dashboard:read`: the assistant, its history, and dashboards, all read-only |
| Upgrade | n/a | Every existing role except Dashboard Viewer gets `assistant:use, ops:read, alert:read, governance:read`, so nobody else loses a page |

## Benchmark

| Product | Per-role page visibility |
| --- | --- |
| Metabase | Hides the admin and data sections from people without the permission. Collection permissions decide which dashboards are listed |
| Power BI | Viewer workspace role sees content only. Building and the dataset pages need Contributor or more |
| Superset | Menu entries are permission-driven (`menu_access`); a page without the permission is refused |

Target: match Superset (menu and page both follow the permission) and Power BI's viewer-only experience.

## Acceptance checklist

The full list is on the feature page, [`pages-per-role.md`](../features/pages-per-role.md).

## Not included

- Switching modules off for the whole installation (`ADM-5`). This item keeps the nav map ready for it (`module` field) but builds no switch.
- Narrowing which tables a role reads (`GOV-7`).
- A tenant-scoped administrator role.

## Asking the assistant

An Assistant only user asks questions and gets answers and charts from the data and dashboards they may read. Write tools are hidden from them and refused if called. This is not a BI feature, so `AI-15` does not apply.
