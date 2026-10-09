# Trusted and deprecated tables, and tags with agreed values

| | |
| --- | --- |
| Module | Data (Catalog) |
| Backlog | `DATA-12` |
| Spec | `docs/core/specs/data-12.md` (no *(proposed)* numbers) |
| Status | Decisions signed 2026-10-09. Part 1 in build |
| Plan | Part 1: `docs/superpowers/plans/2026-10-09-data-12-part-1-certification.md`. Parts 2 and 3 get their own plans |

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

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | As a user with `governance:write`, open a table and mark it certified | The page shows "Certified"; the change history says who and when | |
| 2 | Search the table in the ⌘K box and in the Data Explorer | Both show the mark | |
| 3 | In the Data Explorer, filter by certification: certified | Only certified tables | |
| 4 | Mark another table deprecated, with a note and a replacement table | Its page shows the note and a link that opens the replacement | |
| 5 | Give a replacement that does not exist, then the table itself | Each is refused with a plain message; nothing is saved | |
| 6 | Take the mark off | The mark, the note and the link are gone everywhere; the history says who | |
| 7 | As a user with `catalog:write` but not `governance:write`, open a table | No control to set a mark; the description and tags can still be edited, and editing them does not change the mark | |
| 8 | (operator) As that user, call the certification route | 403; nothing is saved | |
| 9 | As a user without `catalog:read`, search | No tables and no marks | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
