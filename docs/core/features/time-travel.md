# Query a table as it was at an earlier version, safely

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-16` |
| Spec | `docs/core/specs/data-16.md` (no *(proposed)* numbers) |
| Kind | Task (it has an acceptance checklist because it changes what a user sees) |
| Status | In Progress (the pull request is open; `BACKLOG.md` holds the status) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-09; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | PR #100, with the first run of the `g8-time-travel` gate |
| Plan | `docs/superpowers/plans/2026-10-09-data-16-time-travel.md` |

## Problem

A raw (Iceberg) table keeps a version per load, and the console offers to
query an earlier one. Run on 2026-10-09: on ClickHouse this works, but no
test proves that masking and row filters still apply to the earlier version.
On Trino the API refuses the query with "policy cannot be evaluated", and
Query Studio's version picker writes only the Trino form, whichever engine
is chosen, so the button it offers produces a query that cannot run.
Evidence is in the plan, section 2.

## What the user can do when this is done

1. On an asset's Activity tab, press "Query this version" and get the table
   as it was after that load, on ClickHouse, as today.
2. In Query Studio with ClickHouse chosen, pick a table and a version, listed
   with its time and operation, and have the query pinned to it.
3. Trust that a masked column is still masked, and a row filter still
   applied, at an earlier version: a gate test in CI proves it on every
   change.
4. With Trino chosen, see the version picker switched off with the reason,
   and get a plain message if a past-version query is typed by hand.
5. Read how many versions a table has and how far back they go, where the
   versions are listed and where one is chosen.

## Not included

- Past versions on Trino, and masking on Trino at all: `DATA-21`.
- Two tables at two different versions in one query. On ClickHouse one
  version id applies to every Iceberg table in the query.
- Querying by date and time instead of by version.
- Versions of Silver and Gold tables: the engine keeps none.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in `AI-16`.

## Decisions

Signed by the product owner on 2026-10-09.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Trino. The spec says "works for ClickHouse and Trino". On Trino the query is refused today, and by reading the code masking on Trino does not work either; fixing both is larger than this task and cannot be tried on the build machine. | Split. This task proves ClickHouse and refuses Trino with a plain message. Trino becomes `DATA-21`, with its own gate. The spec's Trino target is not met by this task. | 2026-10-09 |
| 2 | What "retention shown" says. Version clean-up does not run on this ClickHouse version, so a promise in days would be invented. | State what is there: "N versions, the oldest from <date>". | |
| 3 | Two tables in one query at a pinned version fail on ClickHouse. | State it as a limit beside the picker; not fixed. | |

## Limits to tell a customer

- A past version can be queried on ClickHouse, for raw (Iceberg) tables
  only.
- One version applies to the whole query; a query that joins two raw tables
  cannot be pinned to a version.
- Past versions cannot be queried on Trino.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Steps needing a terminal are marked
(operator).

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-16-AC1` | Open a raw table loaded at least twice; Activity tab; "Query this version" on the oldest | Query Studio opens on ClickHouse and returns the rows of that load | Not run |
| `DATA-16-AC2` | In Query Studio (ClickHouse), write `SELECT count() FROM` a raw table, pick the table and an older version in the picker, run | The count of that version; the picker listed each version with its time and operation | Not run |
| `DATA-16-AC3` | Pick another version for the same query | The earlier pin is replaced, not added twice | Not run |
| `DATA-16-AC4` | As a user whose role masks a column of that table, repeat step 1 | The column is masked at the older version | Not run |
| `DATA-16-AC5` | Switch the engine to Trino | The picker is switched off and says past versions are available on ClickHouse | Not run |
| `DATA-16-AC6` | On Trino, type a query with `FOR VERSION AS OF` and run | A plain message saying past versions are available on ClickHouse; not "policy cannot be evaluated" | Not run |
| `DATA-16-AC7` | Look at the Snapshots card and the picker | Each states how many versions the table has and the date of the oldest | Not run |
| `DATA-16-AC8` | Read the picker's note | It says one version applies to every raw table in the query | Not run |
| `DATA-16-AC9` | As a user without `query:read`, open the Activity tab | No "Query this version" that runs; the page says access is missing | Not run |
| `DATA-16-AC10` | (operator) Open the CI run of the pull request | The time-travel gate passed | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
