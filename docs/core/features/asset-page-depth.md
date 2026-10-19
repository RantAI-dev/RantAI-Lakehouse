# Deeper asset page: preview, usage, access, restore, history

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-15` |
| Spec | `docs/core/specs/data-15.md` |
| Kind | Feature |
| Status | Decisions **signed by the product owner (2026-10-14, all six)** — plan is next |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | Not yet |
| Shipped | Not yet |
| Evidence | The Phase 2 pull request (#106) |
| Plan | Not yet written; follows the signature |

## Problem

The asset page previews at most 100 rows, shows 7 days of usage, cannot grant
or revoke access from the table itself, cannot bring a dropped table back, and
its version history does not say who made a version. Read from the code on
2026-10-09 at `main` `c338862` (spec, section 3).

## What the user can do when this is done

1. Preview up to 1,000 rows of a table, with masking and row filters applied as
   today.
2. Read 30 days of usage: queries, the people and queries that run it most,
   tables usually joined with it, and its most-read columns.
3. With the access-approval permission: grant a user access to the table from
   its Access tab, and take a grant away.
4. With the governance permission: restore a dropped Iceberg table within the
   retention window.
5. Read each version's author in the history where the author is recorded.

## Not included

- An entity relationship diagram (spec proposed dropping it — decision 6).
- Restore of ClickHouse tables (they keep no Iceberg metadata to restore from).
- Usage of dashboards counted as table reads (today's counter does not count
  them; that does not change here).

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to
the AI team in `AI-16`.

## Decisions

**Signed 2026-10-14** — the product owner approved every row's Default as
shown below (the *(proposed)* markers are removed; these numbers are now the
decisions). The plan builds to them; a target that cannot be met is
reported, not quietly lowered.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Preview size and paging. | Up to 1,000 rows per request; the page shows them 100 at a time. Masking and row filters apply as today; a preview that cannot be masked still shows no rows. | 2026-10-14 |
| 2 | Usage window and "frequent". | 30 days, from the query history the product already records; dashboards reads stay uncounted. "Frequent" lists the top 10 users, queries and joined tables. Column popularity is built only from what query history already records per query; if columns are not recorded, the section says "not recorded" and is not faked — the plan verifies what exists before building. | 2026-10-14 |
| 3 | Who may grant and revoke. | The holder of `access:approve` — the same permission that decides access requests today (`policy.rs:212`). A direct grant from the page creates the same 30-day permission a decided request creates, is audited like one, and shows in the grantee's own Access view. Revoke removes it at once and is audited with actor and subject. | 2026-10-14 |
| 4 | Who may restore, and what restore does. | `governance:write`. Restore re-registers a dropped Iceberg table from its retained metadata, within 7 days of the drop (the spec's number). The product must not purge a dropped table's metadata before the window ends; if the catalog has already purged it, the page says so with the reason instead of pretending. ClickHouse tables: not restorable, said in the UI. | 2026-10-14 |
| 5 | Version history author. | The author shown is the principal recorded when the write was committed through this product's loader; versions committed by anything else say "unknown" rather than a guess. Writing the principal is a loader change owned by the orchestration team — coordinated at the end of this item (it touches the flow Phase 1's `SRC-11` is rebuilding). | 2026-10-14 |
| 6 | Entity relationship diagram. | Dropped from this item (the spec proposed the drop); `BACKLOG.md` keeps an entry if the owner wants it later. | 2026-10-14 |

## Limits to tell a customer

- A preview is capped at 1,000 rows; it is a sample, not an export.
- Usage counts what the product's own query history records; reads from
  dashboards and from outside tools are not counted.
- A grant expires after 30 days, like a decided request.
- Restore works only for Iceberg tables, only within the retention window.
- A version's author is shown only where the committer was recorded.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Rows follow the spec's four and extend
them with the decisions above.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-15-AC1` | Open a table's preview and page through it | Up to 1,000 rows reachable; masked columns masked at every page | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-15-AC2` | Open a table's usage with 30 days of history | Queries, frequent users, frequent queries, joined tables for 30 days; column popularity as recorded, or "not recorded" | Not run: the QA instance has no 30 days of query history; the card and its figures were seen empty |
| `DATA-15-AC3` | As `access:approve`, grant a user access from the table's Access tab | The grantee can read the table; the grant expires in 30 days; the audit trail names both | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-15-AC4` | Revoke that grant | The grantee loses access at once | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-15-AC5` | Without `access:approve`, open the Access tab | No grant or revoke controls | Not run: needs a user without `access:approve`; covered by `tests/route_auth.rs` in CI |
| `DATA-15-AC6` | Drop an Iceberg table, then restore it from its page within the window | The table is back and readable | Not run in the console: the QA instance has no Lakekeeper admin token; the `g3a` gate step drops and restores a table in CI |
| `DATA-15-AC7` | Try to restore a table dropped before the window, or a ClickHouse table | An honest refusal with the reason | Not run: as AC6 |
| `DATA-15-AC8` | Load a table through the pipeline, then read its history | The new version names the pipeline principal as author; an old version says "unknown" | Not run: no load through the product's own commits on the QA instance |
| `DATA-15-AC9` | As a user without the permission, call the restore route | 403; nothing changes | Not run: covered by `tests/route_auth.rs` in CI |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` sections 2 and 3 updated
- [ ] `BACKLOG.md` status set to **In Acceptance** with the PR; **Released**
      after this checklist passes; Dates given the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
