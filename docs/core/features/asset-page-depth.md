# Deeper asset page: preview, usage, access, restore, history

<!-- PR #106 review R1 SHOULD-FIX DATA-15 3 -->

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-15` |
| Spec | `docs/core/specs/data-15.md` |
| Kind | Feature |
| Status | Released (accepted 2026-10-23; Phase 2 PR #106) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-14 |
| Shipped | 2026-10-21 |
| Evidence | The Phase 2 pull request (#106) |
| Plan | `docs/superpowers/plans/2026-10-16-data-15-asset-page-depth.md` |

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
6. With the governance permission *(proposed, not yet signed)*: drop a table
   that a file upload made, from the table's page, after a confirmation that
   names the table and its row count, and restore it from the same page
   within 7 days (`DATA-55`, `DEC-21`).

## Not included

- An entity relationship diagram (spec proposed dropping it — decision 6).
- Restore of ClickHouse tables (they keep no Iceberg metadata to restore from).
- Usage of dashboards counted as table reads (today's counter does not count
  them; that does not change here).
- Dropping a table made by a connector or a pipeline (`DEC-21`).

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to
the AI team in `AI-16`. The assistant has no tool that drops or restores a
table (`DATA-55`).

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
| 7 | Which tables can be dropped from the console. | A table made by a file upload, with the 7-day restore. Tables made by a connector or a pipeline are out of scope. | 2026-11-12 (`DEC-21`) |
| 8 | Who may drop. | `governance:write`, the permission that restores (decision 4), so whoever can drop can also restore. The person must belong to the tenant whose upload made the table, and their roles must be allowed to read it. | *(proposed, not yet signed)* |
| 9 | Where. | The table's page, Activity tab, Snapshots card: Drop table while the table exists, Restore once it is dropped. The Catalog lists the dropped tables with the days each has left; the list links to the page and does not restore. | *(proposed, not yet signed)* |
| 10 | What the confirmation says. | The table's name and its row count, counted when the dialog opens. When the rows cannot be counted the dialog says so; it never shows 0 for an unknown count. It also says the table can be restored for 7 days and that what reads it will fail until then. | *(proposed, not yet signed)* |
| 11 | What a drop leaves alone. | Everything but the table: its uploaded files, its entry in the Catalog, and the pipelines, quality rules, saved queries, dashboards, marks and grants that name it. Those that read it fail until it is restored. | *(proposed, not yet signed)* |
| 12 | The table's name during the 7 days. | Closed: a new load into it is refused with a sentence that says to restore the table or choose another name. After the 7 days the name takes a new load. | *(proposed, not yet signed)* |
| 13 | The audit trail. | A drop and a restore are each written before they happen (`allowed`) and again when they end (`executed` or `failed`), with who, which table and, for a drop, its row count. If the first entry cannot be written, nothing is dropped or restored. | *(proposed, not yet signed)* |

**Added 2026-11-12 (`DATA-55`).** Row 7 is the product owner's decision `DEC-21`; rows 8 to 13 are proposed and not yet signed.

## Limits to tell a customer

- A preview is capped at 1,000 rows; it is a sample, not an export.
- Usage counts what the product's own query history records; reads from
  dashboards and from outside tools are not counted.
- A grant expires after 30 days, like a decided request.
- A `query:read` grant made from a table's page also opens that table's page for the grantee, without `catalog:read` (product owner 2026-11-12, `DEC-20`). It opens that table only: not the catalog list, not search, not another table's page. Masks, row filters and the role's table access apply as everywhere.
- Shown with a table grant: related tables and usage are hidden. A page opened by a grant carries that table's own facts only — no table it is built from or built into, no joined tables, no frequent users or queries, no dependents, and no replacement's id or name — and the page says so rather than drawing empty panels as if nothing existed. *(proposed, not yet signed)*
- Restore works only for Iceberg tables, only within the retention window.
- A version's author is shown only where the committer was recorded.
- A version written by a file upload names the person who started the load, as "<name> (file upload)", with the name they had then; a version written by a connector's replace or append names the connector's run. A merge load and every version written before 2026-11-12 say "unknown" (`DATA-52`).
- Frequent users are shown by name. A person who no longer exists is "Unknown user", and all such people share one row. A person of another tenant is "Unknown user" too: a name is only shown to someone who shares a tenant with that person. A caller with a service token is shown by the service identity's own name, in its own row.
- A table whose Iceberg table is no longer there shows no row count (`DATA-54`).
- Only a table that a file upload made can be dropped from the console, and only by someone of the tenant whose upload made it. A connector's table and a pipeline's table cannot (`DATA-55`, `DEC-21`).
- A dropped table can be restored for 7 days from its page. After that it cannot come back. Whether the catalog then removes its files from storage was not measured.
- Dropping a table deletes nothing else. Its uploaded files stay, and so does its entry in the Catalog, also after the 7 days. Pipelines, quality rules, saved queries and dashboards that read it fail until it is restored.
- While a dropped table can still be restored, a file cannot be loaded into its name.
- A table with a load in progress cannot be dropped until the load ends.
- The row count in the confirmation is counted at that moment and can take up to 30 seconds on a large table; when it cannot be counted the dialog says so.
- Dropping needs the restore to be configured on the install (the catalog's admin token). Without it nothing can be dropped.

## Spec rows

| Spec row | Status |
| --- | --- |
| Preview up to 1,000 rows | **Met** |
| Usage: 30 days | **Met in code** |
| Access: grant and revoke from the asset | **Met in code** |
| Restore: dropped table within 7 days | **Partly met** (soft delete on every warehouse and undrop route built; only drops after soft-delete profile applied; Bronze Iceberg only) |
| History: version time, operation and author | **Partly met** (author is read and shown; a merge load, and versions written before 2026-11-12, say "unknown") |
| Drop: a table made by an upload, from its page | **Built, not proven against the catalog** (`DATA-55`; the proof is the G9 gate) |

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Rows follow the spec's four and extend
them with the decisions above.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-15-AC1` | Open a table's preview and page through it | Up to 1,000 rows reachable; masked columns masked at every page | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-15-AC2` | Open a table's usage with 30 days of history | Queries, frequent users, frequent queries, joined tables for 30 days; column popularity as recorded, or "not recorded" | Pass 2026-10-23 (product owner) |
| `DATA-15-AC3` | As `access:approve`, grant a user access from the table's Access tab | The grantee can read the table; the grant expires in 30 days; the audit trail names both | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-15-AC4` | Revoke that grant | The grantee loses access at once | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-15-AC5` | Without `access:approve`, open the Access tab | No grant or revoke controls | Pass 2026-10-23 (product owner) |
| `DATA-15-AC6` | Drop an Iceberg table, then restore it from its page within the window | The table is back and readable | Pass 2026-10-23 (product owner) |
| `DATA-15-AC7` | Try to restore a table dropped before the window, or a ClickHouse table | An honest refusal with the reason | Pass 2026-10-23 (product owner) |
| `DATA-15-AC8` | Load a table through the pipeline, then read its history | The new version names the pipeline principal as author; an old version says "unknown" | Pass 2026-10-23 (product owner) |
| `DATA-15-AC9` | As a user without the permission, call the restore route | 403; nothing changes | Pass 2026-10-23 (product owner) |
| `DATA-15-AC10` | As `governance:write`, open the page of a table an upload made, Activity tab, and press Drop table | A dialog names the table and its row count (or says the rows could not be counted) and says it can be restored for 7 days | |
| `DATA-15-AC11` | Confirm the drop | The page says the table was dropped and until when it can be restored; the Catalog lists it under Dropped tables with the days left; the audit trail names who dropped which table and its row count | |
| `DATA-15-AC12` | Press Restore on the same page | The table is back with the same row count and can be queried; it is gone from Dropped tables | |
| `DATA-15-AC13` | Open the page of a table a connector made, and of a Silver table | No Drop table; calling the drop route answers "Only a table made by a file upload can be dropped here." | |
| `DATA-15-AC14` | Without `governance:write`, open an uploaded table's page and call the drop route | No Drop table; 403; nothing changes | |
| `DATA-15-AC15` | While the table is dropped, load a file into its name | Refused with the sentence that says to restore it or choose another name | |
| `DATA-15-AC16` | Start a load into an uploaded table and try to drop it while it loads | Refused: a file is being loaded into this table | |

**Acceptor** (not the Owner): product owner (held in the base) **Date:** 2026-10-23 **Build:** main 0e5498d2

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` sections 2 and 3 updated
- [x] `BACKLOG.md` status set to **In Acceptance** with the PR; **Released**
      after this checklist passes; Dates given the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
