# `WEB-7` Side-panel forms become full pages

| | |
| --- | --- |
| Backlog | `WEB-7` in [BACKLOG.md](../BACKLOG.md) |
| Module | Platform |
| Size | M |
| Spec checked | Against the code at `dcaae8a5` on 2026-11-09 |

## Why

Create and edit forms open in a narrow side panel (`components/patterns/create-sheet.tsx`, `sm:max-w-md`), and the two largest forms are pop-ups: the chart builder (`chart-builder.tsx:1773`) and the alert rule form (`alerts-page.tsx:479`). Long forms scroll inside a small box, cannot be linked to, and are lost on a reload. The submit button reads "Creating…" even when editing (`create-sheet.tsx:66`).

## What users get

Each of these forms is its own page with an address, a back link, and Cancel and Save buttons: Invite user, Create service identity, Create role, Edit role, Create tenant, Tag keys (new and edit), Add quality rule, Save query, Save as SQL source, New chart, Edit chart, New alert rule, Edit alert rule. Small dashboard pop-ups stay as they are, so the dashboard stays in view.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Forms listed above | Side panel or pop-up | Full page with its own URL |
| Reload on a form page | Form gone | Edit pages reload with the saved values; new pages reload empty |
| Save query and Save as SQL source | Panel over the editor | Page; the SQL comes over in the browser session, and Cancel or Save returns to the editor with its text intact |
| Chart drafted by the assistant | Opens the builder pop-up | Opens the New chart page with the draft filled in |
| Submit label | "Creating…" everywhere | "Creating…" on new, "Saving…" on edit |
| Page access | n/a | Every new page has an `ADM-13` access-map entry |
| `CreateSheet` | 9 users | Deleted |

## Benchmark

Metabase, Superset and Power BI all build charts and alerts on full pages or full-screen editors. Their user and role administration uses full pages or wide modals.

## Acceptance checklist

- `WEB-7-AC1` Each listed form opens at its own address, and Save creates or changes the object as before
- `WEB-7-AC2` Reloading an Edit page shows the saved values
- `WEB-7-AC3` Save query from Query Studio, then Back: the editor still holds the SQL
- `WEB-7-AC4` Ask the assistant for a chart, then open the draft: the New chart page has the draft
- `WEB-7-AC5` A person without the permission who opens a form's address sees "You don't have access"
- `WEB-7-AC6` Dashboard pop-ups (tabs, filters, share, download, version history) still open over the dashboard

## Not included

- Form pickers (`WEB-8`).
- A warning about unsaved changes when leaving a page.

## Asking the assistant

Not changed. The assistant's chart drafts open the new page (above).
