# Trusted and deprecated tables, and tags with agreed values

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-12` |
| Spec | `docs/core/specs/data-12.md` (no *(proposed)* numbers) |
| Kind | Feature |
| Status | Released (accepted 2026-10-23; Phase 2 PR #106) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-09; also in `BACKLOG.md` Dates |
| Shipped | 2026-10-21 |
| Evidence | The pull request of each part |
| Plan | Part 1: `docs/superpowers/plans/2026-10-09-data-12-part-1-certification.md`. Parts 2 and 3: `docs/superpowers/plans/2026-10-14-data-12-parts-2-3-governed-tags-and-picker-marks.md` |

## Problem

Nothing tells a reader which of two similar tables is the one to trust, or
that a table should no longer be used. Tags are any word anyone with edit
rights types, so `finance`, `fin` and `keuangan` can all mean the same
thing. Read from `main` at `bc048e0` on 2026-10-09; evidence is in the plan,
section 2.

## What the user can do when this is done

Part 1, marks in the catalog:

1. With `governance:write`: mark a table certified, or deprecated with a
   note and, if there is one, the table to use instead; and take a mark off.
2. Anyone: see the mark on the table in search results, in the Data
   Explorer and on its page, and filter the Data Explorer by it.
3. On a deprecated table's page, read the note and follow a link to the
   replacement.
4. See who set a mark and when, in the table's change history.

Part 2, governed tags:

5. With `governance:write`: define a tag key and the values it may take.
6. With `catalog:write`: give a table a value for a key, chosen from the
   allowed values. A value outside the list is refused.
7. Anyone: find and filter tables by a governed tag. Free one-word tags
   keep working beside them.

Part 3, marks where tables are picked:

8. In the chart builder, see the mark on a Gold mart in the data-source
   picker, and a warning when a deprecated one is chosen.
9. In Query Studio, see the mark on each table a ClickHouse query reads,
   and a warning when one is deprecated.

## Not included

- A tag on a namespace applying to its tables: `DATA-22` (decision 3).
- A data-owner role. Marks are set with `governance:write` (decision 1).
- Marks on a dashboard SQL source: its tables are not recorded.
- Marks in the SQL editor's autocomplete, and on a Trino query's sources.
- Tag-based masking.
- Ranking certified tables higher in search.

## Asking the assistant

Not part of the dashboards assistant for parts 1 and 2; AI work for the
Data module is handed to the AI team in `AI-16`. Part 3 changes the chart
builder, which is a dashboards feature: its plan names the assistant change
(the chart tools say when a mart is deprecated), and the AI team reviews it.

## Decisions

Signed by the product owner on 2026-10-09.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Who sets a mark. The spec says "a data owner or admin"; the product has no owner role, only a free-text owner field. | `governance:write` (Governance Admin, Platform Admin), through its own route, so `catalog:write` alone cannot certify. The spec's "owner" is not built. | 2026-10-09 |
| 2 | How governed tags sit beside today's free tags. | Added beside them. An admin defines keys and allowed values; a table gets `key = value` pairs that are checked. Free one-word tags stay and keep working. | 2026-10-09 |
| 3 | Tag inheritance from a schema. A schema here is one of four fixed namespaces, with no stored record and no page to edit. | Not built by this task; backlog `DATA-22`. The spec's inheritance target is not met by `DATA-12`. | 2026-10-09 |
| 4 | Where marks show outside the catalog. | In parts. Part 1: search, Data Explorer, asset page. Part 3: Gold marts in the chart builder's picker, and the sources Query Studio lists for a ClickHouse query. SQL sources and autocomplete are not marked. | 2026-10-09 |

## Limits to tell a customer

- A mark says what a person decided; nothing checks a certified table.
- Only a user with `governance:write` sets or removes a mark.
- A deprecated table still works. It warns; it does not block.
- Tags do not pass from a namespace to its tables.
- A dashboard SQL source does not show the marks of the tables it reads.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Steps needing a terminal are marked
(operator). Rows 1 to 9 are part 1; the later parts add theirs.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-12-AC1` | As a user with `governance:write`, open a table and mark it certified | The page shows "Certified"; the change history says who and when | Pass 2026-10-11 |
| `DATA-12-AC2` | Search the table in the ⌘K box and in the Data Explorer | Both show the mark | Pass 2026-10-11 |
| `DATA-12-AC3` | In the Data Explorer, filter by certification: certified | Only certified tables | Pass 2026-10-11 |
| `DATA-12-AC4` | Mark another table deprecated, with a note and a replacement table | Its page shows the note and a link that opens the replacement | Pass 2026-10-11 |
| `DATA-12-AC5` | Give a replacement that does not exist, then the table itself | Each is refused with a plain message; nothing is saved | Pass 2026-10-11 |
| `DATA-12-AC6` | Take the mark off | The mark, the note and the link are gone everywhere; the history says who | Pass 2026-10-11 |
| `DATA-12-AC7` | As a user with `catalog:write` but not `governance:write`, open a table | No control to set a mark; the description and tags can still be edited, and editing them does not change the mark | Pass 2026-10-23 (product owner) |
| `DATA-12-AC8` | (operator) As that user, call the certification route | 403; nothing is saved | Pass 2026-10-23 (product owner) |
| `DATA-12-AC9` | As a user without `catalog:read`, search | No tables and no marks | Pass 2026-10-23 (product owner) |
| `DATA-12-AC10` | As a user with `governance:write`, define a tag key `data_domain` with allowed values `finance` and `ops` | The key appears on the Tag keys page with its values | Pass 2026-10-19 (product owner, QA instance of #106) |
| `DATA-12-AC11` | As a user with `catalog:write` but not `governance:write`, give a table `data_domain: finance` from its page | Saved; the About card shows `data_domain: finance` beside the free tags | Pass 2026-10-23 (product owner) |
| `DATA-12-AC12` | Give that table `data_domain: revenue`, which is not an allowed value | Refused with a plain sentence naming the allowed values; nothing is saved | Pass 2026-10-19: the editor offers only allowed values; the API refusal of a value outside the list was run by the planner on 2026-10-16 |
| `DATA-12-AC13` | Search `finance` in the ⌘K box and in the Data Explorer; also filter the Tags column by it | The table is found (reason line says tag `data_domain: finance`); the column shows the pair; the filter finds it | Pass 2026-10-23 (product owner) |
| `DATA-12-AC14` | On the same table, edit the free tags | Free tags still work beside the governed one; the governed pair is unchanged | Pass 2026-10-23 (product owner) |
| `DATA-12-AC15` | As a user with `governance:write`, delete the key `data_domain` while a table still uses it | Refused with the number of tables using it; after the value is removed from the table, the delete succeeds | Pass 2026-10-23 (product owner) |
| `DATA-12-AC16` | (operator) Without `governance:write`, call `POST /api/governance/tag-keys`; without `catalog:write`, send `governedTags` on the annotation route | Both 403; nothing is saved | Pass 2026-10-23 (product owner) |
| `DATA-12-AC17` | In the chart builder's picker, a certified Gold mart shows its mark, and choosing a deprecated one warns with the note and a link to its replacement; in Query Studio, a ClickHouse query reading a deprecated table marks it in its sources list | As described | Pass 2026-10-19 (product owner, QA instance of #106) |

**Acceptor** (not the Owner): product owner (held in the base) **Date:** 2026-10-23 **Build:** main 0e5498d2

The product owner checked `DATA-12-AC1` to `AC6` on 2026-10-11 on a separate
instance of this branch and reported them as expected. `AC7` to `AC9` need a
user without `governance:write`; the planner ran `AC8` through the API (403).
Rows `AC10` to `AC17` are parts 2 and 3 (planned
2026-10-14). The product owner accepted the whole checklist on 2026-10-23;
every row that was not yet Pass is marked Pass on that date.

Exceptions, each with an owner and a date:

## After acceptance

- [x] `PRODUCT.md` section 2 and 3 updated
- [x] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [x] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
