# Audit trail covers every console change

| | |
| --- | --- |
| Module | Governance |
| Backlog | `GOV-1` |
| Kind | Feature |
| Spec | `docs/core/specs/gov-1.md` |
| Status | Planned |
| Priority | P1 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | Engineering plan kept by the planner |
| Started | 2026-11-14 |
| Shipped | |
| Evidence | |

## Problem

About 110 of the 218 routes that change something leave no audit row, among them every dashboard, saved query, alert and report subscription change. The audit page shows only the newest 200 events and cannot export.

## What the user can do when this is done

1. Change anything in the console and find the change on the audit page: who, what, on what, when, and the outcome.
2. Find a refused change (no permission) on the audit page, with outcome Refused.
3. Filter the audit page by person, action, object and time range and get events of any age, page by page.
4. Export the filtered events as CSV.
5. See, for a role, policy, table-access or grant change, the old and new value; for any other change, the names of the fields that changed.
6. Trust that nobody, not even an administrator through the database account the console uses, can edit or delete an audit row.

## Not included

Reads and downloads (as today), retention settings, sending events to another system, personal UI state.

## Asking the assistant

No new assistant work. Its tool calls are already recorded.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Excluded from the audit (each listed in the guard test with this reason): read-like POSTs (preview, validate, estimate, compile, lag-check, `check`, `compare`), connection and email "test" buttons, the public data and frame routes, machine callbacks from ingestion jobs, multipart upload parts, and personal UI state (favourites, recent, home layout, chat session create/rename/delete). Everything else is recorded | as written | Owner, 2026-11-14 |
| 2 | A refused change is recorded (outcome `refused`) for every covered route | yes | Owner, 2026-11-14 |
| 3 | Old and new values only for roles, policies, table access and grants; other changes record the names of changed fields only; never a secret | as written | Owner, 2026-11-14 |
| 4 | A failed audit write does not undo the change (as today); it is logged and counted on the Operations health page | as written | Owner, 2026-11-14 |
| 5 | CSV export limit per file | 100,000 rows | Owner, 2026-11-14 |
| 6 | The database refuses `UPDATE` and `DELETE` on `audit_event` (trigger, in a migration) | yes | Owner, 2026-11-14 |
| 7 | Retention: keep forever, no setting in this feature | yes | Owner, 2026-11-14 |
| 8 | Delivery in two parts: A, coverage plus the guard test (Decisions 1 to 4, 6); B, server-side search, paging and export (Decision 5) | yes | Owner, 2026-11-14 |
| 9 | Who sees the audit page and export | `audit:read` only (Governance Admin, and `*:*`). Today the page needs only `governance:read`, which migration `0133` gives every role; this feature narrows it (owner, 2026-11-14) | Owner, 2026-11-14 |

## Limits to tell a customer

- Reads and downloads are recorded as today, not every page view.
- Events are kept forever; there is no retention setting yet.
- Events cannot be sent to an outside system yet; export is CSV from the audit page.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `GOV-1-AC1` | Save, rename, move, share and delete a saved query; open the audit page | Five rows, each with you, the action, the query, the time and Executed | Not run |
| `GOV-1-AC2` | Create a dashboard, add a chart, change a filter, delete the dashboard | One row per change | Not run |
| `GOV-1-AC3` | Create, edit and delete an alert and a report subscription | One row per change | Not run |
| `GOV-1-AC4` | Change the query limits and the reporting settings | One row each, naming the changed fields, no values | Not run |
| `GOV-1-AC5` | Create a role, change its permissions | Rows with the old and new permissions | Not run |
| `GOV-1-AC6` | As a user without the permission, try to delete someone else's dashboard | Refused in the console; a row with outcome Refused | Not run |
| `GOV-1-AC7` | Ask the assistant to change a chart | A row with the assistant as actor (as today) | Not run |
| `GOV-1-AC8` | Filter the audit page to one person and a date range older than the newest 200 events | The matching events of that range, paged | Not run |
| `GOV-1-AC9` | Export the filtered result | A CSV with the same rows and columns | Not run |
| `GOV-1-AC10` | As an Analyst (holds `governance:read`, not `audit:read`), open the audit page and the export link | Refused; the audit page is not in the menu | Not run |
| `GOV-1-AC11` | (operator) Try `UPDATE audit_event ...` and `DELETE FROM audit_event ...` with the console's database account | Both refused by the database | Not run |
| `GOV-1-AC12` | (operator) Look at the CI run of the pull request | The audit coverage test passes, and it lists every excluded route with its reason | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo; the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read

