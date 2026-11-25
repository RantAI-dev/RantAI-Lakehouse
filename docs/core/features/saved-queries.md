# Saved queries you can organise, share and recover

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-3` |
| Kind | Feature |
| Spec | `docs/core/specs/qs-3.md` |
| Status | Released: accepted by the product owner on 2026-11-14 (every row Pass; AC2 with findings; AC7 passed: the refusal part on 2026-11-12, the database part via the API on 2026-11-14; the owner's NUL-byte exception was not needed, because the failure could be triggered and answered `database error` with no Postgres or sqlx text) |
| Priority | P2 |
| Owner | Held in the base. |
| Acceptor | Held in the base. |
| Started | 2026-10-30; also in `BACKLOG.md` Dates |
| Shipped | API 2026-11-03 (#361); console 2026-11-05 (#389); move fix 2026-11-06 (#393); #409 2026-11-07; folder sharing warning 2026-11-07 (#416); Bronze link and the review fixes 2026-11-08 (#437) |
| Evidence | API in #361 (2026-11-03) and its review fixes in #382 (2026-11-05); console in #389 (2026-11-05), with moving a folder needing manage on everything in it in #393 (2026-11-06) and sharing a folder needing the same in #409, and a folder naming who it is shared with before a save or a move in #416: the folder tree with the personal and the shared space, server-side search, the level-gated row and folder actions, the move dialog, the trash with restore at edit, version history with revert behind a confirmation, the share panel for queries and folders at manage, with the grantees' names, and the studio saving back into the query it came from; TypeScript suite 3449 pass / 1 skip / 0 fail on #389's head; every refusal surfaces the server's own sentence through the standard renderer, and the console writes none of its own. |
| Plan | Plan files are not kept in the repository. |

## Problem

A saved query can be created and listed, nothing else (`GET`/`POST /api/query/saved`, `routes/query.rs:2106`,
`:2308`). There is no route to edit, rename, move or delete one, no folder, no version and no sharing. Since
`SEC-20` the list is scoped to the caller's organisation, and inside it every `query:read` holder sees every saved
query (`routes/query.rs:639`, `lakehouse-store/src/queries.rs:304`). The table is `saved_query` in Postgres
(`0005`, `0046`, `0099`); this change gives it the columns and the tables it had none of: the migration
`rust/migrations/0122_saved_query_access.sql` adds `folder_id`, `deleted_at` and `deleted_by` to it and creates
the tables `saved_query_folder`, `saved_query_version` and `saved_query_access_rule`.

Dashboards already have the patterns, in ClickHouse: a 30-day trash (`lakehouse-bi/src/trash.rs:44`), 15 versions
with revert (`versions.rs:45`), folders with a personal space (`folders.rs`) and who may see one (`access.rs`),
and, in open PR #250, per-item access in Postgres with three levels (`dashboard_access_rule`: view, edit, manage).

## What the user can do when this is done

1. Edit a saved query's SQL and title, rename and duplicate it, and move it at manage.
2. Keep queries in folders, in a personal space only they see, or in the organisation's shared space.
3. Share a query or a folder with a person or a role at one of three levels.
4. See the last 15 versions with author and time, and go back to any of them.
5. Delete a query to a trash, and restore it within 30 days.
6. Search saved queries by name and SQL text.

## Not included

- Scheduling a saved query (the page's existing probe stays `supported: false`).
- Sharing outside the organisation.
- Comments on a saved query.

## Asking the assistant

The chat's `list_saved_queries` and `run_saved_query` see only the queries the person may see, and `save_query`
saves into the person's personal space. No new tool: organising and sharing are console actions.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Three levels, the same as dashboards (`BI-19`): view (see and run), edit (change SQL, title and tags; rename a folder), manage (also move, share, and delete a folder). The spec's separate "run" level is dropped: anyone who can see a query's SQL can copy it and run it under their own `query:read`, so view-but-not-run would protect nothing. Sharing or moving a folder needs manage on everything in it. Anyone with edit on a folder sees who it is shared with: the access list answers at edit, every rule in it carries the grantee's name and email, the save sheet and the move dialog name who a chosen folder is shared with, and the share panel shows names instead of ids | 3 levels | Owner, 2026-11-05; changed: Owner, 2026-11-06; changed: Owner, 2026-11-07 |
| 2 | A level is granted to a person or a role of the same organisation, on a query or a folder. A folder's grant covers what is inside it | none | Owner, 2026-10-30 |
| 3 | A new saved query goes to the author's personal space unless they pick a folder. Only the author sees the personal space. The **shared space** is the root of the tree, a query that sits in no folder. Every holder of `query:read` in the organisation sees what is in it, at **view**. The **personal space** is one folder per person, and only its owner sees it | personal | Owner, 2026-10-30 |
| 4 | Every query saved before this release moves to the organisation's shared space with view for everyone in the organisation and manage for its author, so nobody loses a query they see today. A query with no author (seeded, or saved by the assistant before this release) gets manage for administrators only. A grant adds a person or a role to an item in either space. In the shared space it raises the level; in a personal space it is the only way anybody else sees the item. So the acceptance check `QS-3-AC1` (share at view with a second user) is a personal query shared with them | none | Owner, 2026-10-30 |
| 5 | A person with every permission (`*:*`) sees and manages every saved query of the organisation | none | Owner, 2026-10-30 |
| 6 | Delete moves to a trash for 30 days (as dashboards), then the query is removed for good. Edit or manage may delete and restore | 30 days | Owner, 2026-10-30 |
| 7 | A version is kept on every change of SQL, title or parameters, the last 15 (Metabase, and dashboards' `KEEP`). Going back writes a new version "Restored version N", so nothing is lost. A revert restores the SQL and the parameters together (`query-parameters.md`, decision 14) | 15 | Owner, 2026-10-30 |
| 8 | Folders can hold folders, as on the dashboard side | none | Owner, 2026-10-30 |
| 9 | Search matches the name or the SQL text, containing the text typed, case-insensitive, over the queries the person may see | none | Owner, 2026-10-30 |
| 10 | Everything else that lists saved queries uses the same check: the home card, the dashboard click setting, the catalog's "used by" and lineage. A dashboard click target pointing at a query the viewer may not see says "This query is not available to you" | none | Owner, 2026-10-30 |
| 11 | The data lives in Postgres beside `saved_query`, in tables of its own. It copies the shape of `BI-19`'s `dashboard_access_rule`, not the table itself, so this feature does not wait on PR #250's migration | none | Owner, 2026-10-30 |

## Limits to tell a customer

- A shared query shows its SQL to everyone it is shared with; sharing at view level does not hide the SQL.
- A query saved into a folder that is already shared is shared with the same people, because a folder's grant covers what is inside it. The save sheet and the move dialog name who the chosen folder is shared with before the user confirms, so the choice is an informed one.
- Sharing a folder needs manage on the folder and on every sub-folder and query under it, trashed ones included. Otherwise the share is refused with 403 and "Only someone who manages everything in this folder can share it."
- A `PUT` that only removes rules is refused too when the folder holds a query the caller does not manage, so the
  author of such a folder cannot revoke its rules. The query's manager can move the query out, or an administrator
  changes the rules.
- A rule set on an empty folder applies to what is saved or moved into it later. The save sheet and the move dialog
  name who that is, for anyone with edit on the folder.
- Queries saved before this release start in the shared space, visible to the whole organisation, as they were.
- After 30 days in the trash, a query is gone.
- A folder is deleted only when it holds nothing live and nothing in the trash. Otherwise the delete is refused with
  "This folder still holds queries, some of them in the trash. Move or restore them first." No query is ever moved
  silently into a shared place.
- A folder tree is at most four levels deep.
- A move needs manage on everything moved. Moving a query needs manage on it. Moving a folder needs manage on the
  folder and on every sub-folder and query under it, trashed ones included. Either move also needs edit on the target
  folder. Otherwise the move is refused with 403 and "Only someone who manages this can move it." A request that names
  the folder a query is already in is not a move.
- A personal space itself is not shared. Setting access on it is refused with 403 and "A personal space is not shared:
  share a query or a folder inside it."; a rule already stored on one is ignored.
- Search matches a query's title and SQL text. It does not match the owner or the tags.
- Deleting a tenant fails while any of its folders still holds a sub-folder or a query in a folder
  (`saved_query.folder_id` and `saved_query_folder.parent_id` are `ON DELETE RESTRICT` in
  `0122_saved_query_access.sql`; `RESTRICT` is not deferrable, so the database refuses the delete).

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `QS-3-AC1` | Share a query at view with a second user; as that user, run it, then try to save a change | The run works; the change is refused | Pass (2026-11-12, 273d89e6, no case) |
| `QS-3-AC2` | Change a query four times, open its history, go back to version 3 | The SQL is version 3's, and the history shows "Restored version 3" on top | Pass with findings (2026-11-12, 3cdd8332, no case). One save of version 4 failed silently; the history once showed "Query failed (500)" |
| `QS-3-AC3` | Delete a query, open the trash, restore it | It is back in its folder with its history | Pass (2026-11-12, 3cdd8332, no case) |
| `QS-3-AC4` | Save a query without picking a folder; as a second user of the same organisation, search for it | Not found | Pass (2026-11-12, 3cdd8332, no case) |
| `QS-3-AC5` | Share a folder with a role; as a user of that role, open a query inside it | Visible at the folder's level | Pass (2026-11-12, 273d89e6, no case) |
| `QS-3-AC6` | In the chat, as the second user, ask for the saved queries | The private query is not listed | Pass (2026-11-13, 80fb2f68, no case) |
| `QS-3-AC7` | A user without `query:read` opens the saved queries, and an edit fails on the database | Refused; the failure shows a fixed message (principles 2 and 4) | Pass (2026-11-14, 240d2e62, no case). Refusal part (2026-11-12, 3cdd8332): the page says "You don't have access". Database part, through the API: a PATCH with a NUL byte in the title, and separately in the SQL, returned 500 `{"error":"database error"}` both times, with no Postgres or sqlx text; the query stayed at one version; the throwaway query was deleted |
| `QS-3-AC8` | As a user with edit on a shared folder, pick that folder in the save sheet, then in the move dialog | Both name who the folder is shared with, without you, before you confirm | Pass (2026-11-12, 3cdd8332, no case) |

**Acceptor** (not the Owner): product owner (recorded in the base) **Date:** 2026-11-14 **Build:** 240d2e62

Exceptions, each with an owner and a date:

- `QS-3-AC7`, owner 2026-11-14: the auditor tries a NUL byte (`\u0000`) in an edit's SQL or title through the API; if no database failure can be triggered, the code path (`saved_queries.rs:658` to `From<StoreError>`, `lakehouse-store/src/error.rs:169`) and the unit test `internal_message_never_leaks_source_error_text` are accepted as an exception. The exception was not needed: the database failure could be triggered, and the API answered `database error` with no Postgres or sqlx text (see the row above).

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [x] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [x] `BACKLOG.md` Dates has the Shipped date
- [x] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base (done by the auditor)
- [ ] `CHANGELOG.md` entry a customer can read
