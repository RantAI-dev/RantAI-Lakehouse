# Audit trail covers every console change

| | |
| --- | --- |
| Module | Governance |
| Backlog | `GOV-1` |
| Kind | Feature |
| Spec | `docs/core/specs/gov-1.md` |
| Status | Released |
| Priority | P1 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | Engineering plan kept by the planner |
| Started | 2026-11-14 |
| Shipped | 2026-11-15 |
| Evidence | PR #546 (part A: coverage, central writer, append-only trail, audit:read), PR #549 (part B: server-side search, paging, CSV export), PR #608 (part C: the acceptance follow-ups) |

## Problem

The guard test counts every route that changes something: 224, of which 182 are recorded (the route's handler or the central writer) and 42 are excluded, each with a listed reason. Before this feature, every dashboard, saved query, alert and report subscription change left no audit row, and the audit page showed only the newest 200 events and could not export.

## What the user can do when this is done

1. Change anything in the console and find the change on the audit page: who, what, on what, when, and the outcome.
2. Find a refused change (no permission) on the audit page, with outcome Refused.
3. Filter the audit page by person, action, object and time range and get events of any age, page by page.
4. Export the filtered events as CSV.
5. See, for a role, policy, table-access or grant change, the old and new value; for any other change, the names of the fields that changed.
6. Trust the database to refuse every edit and delete of an audit row: the append-only trigger binds an account that neither owns `audit_event` nor is a superuser. The table's owner or a superuser can still skip it, with `ALTER TABLE audit_event DISABLE TRIGGER` or with `SET session_replication_role = replica`, which skips the trigger without changing the table. The console connects as `lakehouse_app`, which neither owns `audit_event` nor is a superuser, so it cannot turn the trigger off; only the migration owner can.

## Not included

Reads and downloads (as today), retention settings, sending events to another system, personal UI state.

## Asking the assistant

No new assistant work. Its tool calls are already recorded.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Excluded from the audit (each listed in the guard test with this reason): read-like POSTs (preview, validate, estimate, compile, lag-check, `check`, `compare`), connection and email "test" buttons, the alert test button (`POST /api/alerts/test`), the public data and frame routes, machine callbacks from ingestion jobs, the alert scheduler's tick (`POST /api/alerts/run`), multipart upload parts, and personal UI state (favourites, recent, home layout, chat session create/rename/delete, and the read marks of mentions, `POST /api/dashboard/comments/mentions/read`; owner, 2026-11-15). Everything else is recorded | as written | Owner, 2026-11-14 |
| 2 | A refused change is recorded (outcome `refused`) for every covered route | yes | Owner, 2026-11-14 |
| 3 | Old and new values only for roles, policies, table access and grants; other changes record the names of changed fields only; never a secret | as written | Owner, 2026-11-14 |
| 4 | A failed audit write does not undo the change (as today); it is logged and counted on the Operations health page | as written | Owner, 2026-11-14 |
| 5 | CSV export limit per file | 100,000 rows | Owner, 2026-11-14 |
| 6 | The database refuses `UPDATE` and `DELETE` on `audit_event` (trigger, in a migration), except clearing `run_id` or `approval_id` when the run or approval they point to is deleted | yes | Owner, 2026-11-14 |
| 7 | Retention: keep forever, no setting in this feature | yes | Owner, 2026-11-14 |
| 8 | Delivery in two parts: A, coverage plus the guard test (Decisions 1 to 4, 6); B, server-side search, paging and export (Decision 5) | yes | Owner, 2026-11-14 |
| 9 | Who sees the audit page and export | `audit:read` only (Governance Admin, and `*:*`). Today the page needs only `governance:read`, which migration `0133` gives every role; this feature narrows it (owner, 2026-11-14) | Owner, 2026-11-14 |

## Limits to tell a customer

- Reads and downloads are recorded as today, not every page view.
- Events are kept forever; there is no retention setting yet (`GOV-21`).
- Events cannot be sent to an outside system yet; export is CSV from the audit page.
- The database owner or a superuser can still skip the trigger that makes the trail append-only, with `ALTER TABLE audit_event DISABLE TRIGGER ...` or with `SET session_replication_role = replica` (which skips the trigger without changing the table). Nothing in the trail records that. The console connects as `lakehouse_app`, a role that owns no table and is not a superuser (`SEC-78`), so it cannot turn the trigger off; only the migration owner can.
- The database allows an `UPDATE` that only clears a row's `run_id` or `approval_id`, because it clears those links when an agent run or an approval is deleted. Every other change to a row is refused.
- A request that the service's timeout cuts off writes no row.
- A change made through a create route does not name the new object when its id is only in the response. The row then names the kind and the action, not the object.
- The row names the fields a change touched and never their values. The exceptions are the old and new values that the identity, policy, table-access and grant rows record.
- Neither the scheduler's alert tick (`POST /api/alerts/run`) nor the alert test button (`POST /api/alerts/test`) writes an audit row (Decision 1; the reason is in the guard test). The tick is the alert scheduler running the rule on its own, with no person behind it, and the test button sends a probe and changes no configuration.
- Writes through the Iceberg REST door are refused by that door and are not in the trail.
- The assistant's audit history tool (`get_audit_history`) needs `audit:read`, like the audit page (#585).
- The Overview page's activity feed (`POST /api/overview`) shows the 20 newest audit rows of everyone only to a person with `audit:read`. Any other person sees only the 20 newest rows of their own actions, with the action, the object and the outcome.
- An asset page's change history shows up to 50 audit rows about that asset to anyone who can open the asset. `audit:read` does not narrow it either.
- Machine callbacks are not audited. The pipeline run events, the schema and capture observations and the connector run reports accept a service identity or an unrestricted administrator account. A person who calls one of these routes by hand with an unrestricted account leaves no row. A pipeline run started from the console is audited as `pipeline.trigger`. The orchestrator's lag call (`POST /api/pipelines/runnable/{id}/lag-check`) is also open to an unrestricted administrator account. It advances the pipeline's lag streak and can send the behind-target alert, and it writes no audit row.
- Marking a dashboard comment mention as read is not recorded. It counts as personal UI state, like favourites.
- Each failed sign-in that reaches the password check adds one row with outcome `refused`. The try that makes an identifier locked adds two rows: the `auth.login_locked` row and the refused row. The row holds no email, no password and no identifier. `auth.login_locked` rows written before this change hold the first 12 hex characters of the identifier's SHA-256, and they stay, because the trail is append-only. An unknown email adds the same row, so a visitor who tries many different identifiers adds a row for each try. A try refused because that identifier is locked adds no row. Nothing removes these rows and there is no retention setting yet, so the growth is permanent. The follow-ups are `SEC-75` (a rate limit for failed sign-ins) and `GOV-21` (retention).
- The CSV export holds the newest 100,000 matching events per file; the file starts with the event id and carries the same outcome words as the page, with the stored outcome beside them and the stored detail column as `reason`. When more match, the audit page shows a notice, and the response carries two headers, `x-audit-export-truncated` and `x-audit-export-total`. To get the rest, narrow the time range and export again.
- Dates in the filters: the API takes RFC 3339 times. The console sends the UTC start of the From day and the UTC start of the day after the To day, so the To day is included in full.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `GOV-1-AC1` | Save, rename, move, share and delete a saved query; open the audit page | Five rows, each with you, the action, the query, the time and Executed | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC2` | Create a dashboard, add a chart, change a filter, delete the dashboard | One row per change | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC3` | Create, edit and delete an alert and a report subscription | One row per change | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC4` | Change the query limits and the reporting settings | One row each, naming the changed fields, no values | Pass (staging `6da7012a`, Auditor 2026-11-16, finding fixed by D3) |
| `GOV-1-AC5` | Create a role, change its permissions | Rows with the old and new permissions | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC6` | As a user without the permission, try to delete someone else's dashboard | Refused in the console; a row with outcome Refused | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC7` | Ask the assistant to change a chart | A row with the assistant as actor (as today) | Pass (staging `8fe62e349`, Auditor 2026-11-18, retest after #573) |
| `GOV-1-AC8` | Filter the audit page to one person and a date range older than the newest 200 events | The matching events of that range, paged | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC9` | Export the filtered result | A CSV with the same rows and columns | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC10` | As an Analyst (holds `governance:read`, not `audit:read`), open the audit page and the export link | Refused; the audit page is not in the menu | Pass (staging `6da7012a`, Auditor 2026-11-16) |
| `GOV-1-AC11` | (operator) Try `UPDATE audit_event ...` and `DELETE FROM audit_event ...` with the console's database account | Both refused by the database | Pass (CI of #546/#549, `lakehouse-store/tests/audit.rs` as superuser; staging console account from deploy config; Auditor 2026-11-16) |
| `GOV-1-AC12` | (operator) Look at the CI run of the pull request | The audit coverage test passes, and it lists every excluded route with its reason | Pass (staging `6da7012a`, Auditor 2026-11-16) |

**GOV-1-AC11 note.** The Auditor's check (2026-11-16) found the console account on staging was `lakehouse`, the image's superuser and the owner of `audit_event`, so it could still turn off the trigger (`DISABLE TRIGGER`, `session_replication_role = replica`); since `SEC-78` the console connects as `lakehouse_app`, which neither owns `audit_event` nor is a superuser, so only the migration owner can turn it off.

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [x] `PRODUCT.md` section 2 and 3 updated
- [x] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [x] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo; the Acceptor is recorded in the base
- [x] `CHANGELOG.md` entry a customer can read

