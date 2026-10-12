# More load modes

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-10` |
| Spec | `docs/core/specs/src-10.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec". The spec has no *(proposed)* numbers |
| Plan | `docs/superpowers/plans/2026-10-12-src-10-load-modes.md` |

Priority and status are in [BACKLOG.md](../BACKLOG.md). Decisions signed
2026-10-12.

## Problem

From the spec: loads either append or replace; there is no way to keep one
current row per key, or a history of changes. Checked in the code and by
measurement on 2026-10-12:

- The spec's "Today" is out of date by one mode. A table can be loaded with
  `replace`, `append` or `incremental` (new rows by a cursor column, SQL
  sources only). Three of Airbyte's five modes exist; the two with
  deduplication do not.
- No table has a key. Nothing merges.
- A run always loads every table of the connector. One table cannot be
  refreshed or cleared by itself.
- Measured, not assumed (`docs/plans/SRC-10-RESULT.md`): the loader can merge
  into an Iceberg table on a key, keeping one current row per key. It cannot
  keep history by itself: the "valid from / valid to" strategy is refused for
  Iceberg tables by the loader version this product ships, so history has to
  be built on top of the merge it does have. And a batch that holds the
  same key twice makes the merge fail, so duplicates must be removed before
  loading.

## What the user can do when this is done

One row per row of the spec's Target table.

| Spec row | Spec target | What the user can do |
| --- | --- | --- |
| Sync modes | All five Airbyte modes, chosen per table | 1. Choose, per table, one of five modes: replace, append, replace and deduplicate, incremental, incremental and deduplicate. |
| Deduplicate on primary key (SCD type 1) | Merge on a declared or detected primary key with a cursor column | 2. Keep one current row per key. The key is read from the source for a SQL table and can be changed; for other sources the user names it. When a key appears more than once in a load, the row with the highest cursor value wins. |
| History (SCD type 2) | Valid-from, valid-to and current-flag columns, as Databricks | 3. Choose a sixth mode, "history": every version of a row is kept, each with the time it became valid, the time it stopped being valid, and a mark on the current one. An update at the source closes the old row and adds a new one. |
| Refresh one table, or clear it | Per table: refresh keeping or removing old rows, and clear (Airbyte refresh/clear) | 4. Refresh one table now, either on top of its rows or after removing them. 5. Clear one table: empty it and forget where its incremental load had got to. Neither touches the connector's other tables. |

## Not included

From the spec: anything not in the Target table. In particular:

- Change-capture (CDC) connectors: they already apply inserts, updates and
  deletes as they arrive (`SRC-4`).
- Removing a row from the table when it is deleted at the source. A batch
  load cannot see a deletion; measured, the loader's delete marker also fails
  on Iceberg.

## Asking the assistant

From the spec: not part of the dashboards assistant. The assistant's
`set_ingest_spec` tool sends the same table list, so the AI team reviews the
tool schema when the mode list grows.

## Decisions

Signed by the product owner on 2026-10-12 with "run it as the spec says": decision 1 follows the spec's row instead of the planner's default; decisions 2 to 8 are the defaults, which do not depart from the spec.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec "History (SCD type 2)". The loader refuses it for Iceberg tables (measured), so it is built here as our own merge: read the current row of every key in the load, close the ones that changed, add the new versions, in one commit. The planner proposed to defer it; the product owner chose the spec ("run it as the spec says", 2026-10-12). | Built in this work, as the spec says: columns `_valid_from`, `_valid_to` and `_is_current`. It ships only if the measurement of plan task 1 shows the merge is one commit and repeatable; if it does not, the developer stops and the product owner is asked again. | 2026-10-12 (the spec, not the planner's default) |
| 2 | Spec "five Airbyte modes": which sources get which. | Replace, append, replace and deduplicate: every batch source. Incremental and incremental and deduplicate: SQL sources only, as incremental is today. | 2026-10-12 |
| 3 | Spec "declared or detected primary key". | SQL sources: the key the source declares is offered and can be changed. Other sources: the user names the key columns. A deduplicating mode cannot be saved without a key. | 2026-10-12 |
| 4 | Spec "with a cursor column": which row wins when a key repeats in one load. | The row with the highest value of the cursor column. With no cursor column (replace and deduplicate on a source without one), the last row read. | 2026-10-12 |
| 5 | Spec "refresh keeping or removing old rows, and clear". | Refresh, keeping: load the whole table again now, by its mode's rule (a deduplicating mode merges, append adds). Refresh, removing: empty the table, then load it whole. Clear: empty the table and forget its cursor, load nothing. Each asks for confirmation; "removing" and "clear" say that the rows are gone. | 2026-10-12 |
| 6 | Beyond the spec: the key changes or disappears at the source. | `SRC-8` already makes the table wait when the primary key changes. A deduplicating table whose key column is removed waits too. | 2026-10-12 |
| 7 | Beyond the spec: who may refresh or clear. | A user with `connector:manage` on a connector of their own tenant, as for "Run now". | 2026-10-12 |
| 8 | Beyond the spec: a merge rewrites files of the Bronze table. | Kept as the loader does it. Before the mode ships, the gate proves that the query engine reads a merged table correctly (one row per key, right count). | 2026-10-12 |
| 9 | Beyond the spec, follows from decision 1: what "valid from" is, and what happens to a row that disappears from the source. | Valid from is the time of the load that first saw the version. A row that is no longer at the source stays current: a batch load cannot tell a deletion from a row it was not given. | not signed: new on 2026-10-12, the default is built until it is |

## Limits to tell a customer

- History keeps the time a load saw a change, not the time the change happened at the source, and a load that runs once a day sees at most one version a day.
- A row deleted at the source stays in the table, in every mode.
- Deduplication needs a key; incremental modes need a SQL source.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
Rows 1 to 4 are the spec's checklist, in its order.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec: every Target row above works as written | Rows 5 to 12 pass | |
| 2 | Spec: set a table to "incremental and deduplicate"; update a row at the source; run | One current row for that key, with the new values. Set another table to "history" and do the same: a closed row with its valid-to time and a new current row | |
| 3 | Spec: refresh one table of a connector that has two | The other table's rows and its last load time do not change | |
| 4 | Spec: as a user without `connector:manage`, change a mode, refresh, clear; and make a load fail | Refused each time; the failure shows an honest message with no source error text | |
| 5 | Open a table's mode choice on a SQL connector and on a file connector | Six modes on the first; four on the second, with the reason the other two are missing | |
| 6 | On a SQL table, open the key | The source's own key is filled in and can be changed | |
| 7 | Choose a deduplicating mode with no key | Cannot be saved; the message says why | |
| 8 | Load a table in "replace and deduplicate" from a source that holds the same key twice | One row per key: the one with the highest cursor value | |
| 9 | Refresh a table "removing old rows", then "keeping" | First: the table holds exactly the source's rows. Second: by its mode, merged or added | |
| 10 | Clear a table | It is empty; the next incremental run loads everything again | |
| 11 | Query a deduplicated table from Query Studio after two merges | One row per key and a correct count | |

| 12 | In "history" mode, run twice with nothing changed at the source, then query the table | No new rows; one current row per key; closed rows keep their times | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item set to Released; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
