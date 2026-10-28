# Query history page

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-4` (history part) |
| Kind | Feature |
| Spec | `docs/core/specs/qs-4.md` (no *(proposed)* number; the 14 days are Snowflake's) |
| Status | Building |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-24; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | |
| Plan | Plan files are not kept in the repository. |

## Problem

History is a side panel of the caller's last 200 completed runs (`lakehouse-store/src/queries.rs:414`, `:441`),
with no filter and no page. A failed, refused or stopped run is not recorded at all: the run route writes
`"completed"` only (`routes/query.rs:966-991`), so the database's `failed`, `blocked` and `cancelled` statuses have no
producer. The tables a run read are not recorded either, so nobody can find "the query on `orders` last week".

## What the user can do when this is done

1. Open Query Studio > History and see their own runs of the last 14 days, newest first, 50 per page.
2. Filter by status (completed, failed, refused), engine, minimum duration, a table the run read, and a
   time range within the 14 days.
3. See failed and refused runs as well as completed ones, each with the message they saw at the time.
4. Click a run to open its SQL in a new Query Studio tab.
5. In the Query Studio editor, insert and complete Bronze tables too (`QS-1` review fix).

## Not included

- Other users' runs, and cancelling them (the spec's admin view). It needs a new route and a decision on the
  permission, and `policy.rs` is held by open PRs. Later part.
- The query profile and the hints. Later part.
- Deleting old rows. Rows older than 14 days stay in the database and are not shown; a retention job is a separate
  decision.
- Runs from before this change carry no table list, so the table filter does not find them.

## Asking the assistant

No. History is the person's own trail in the console. The chat's `run_saved_query` goes through the run route and is
recorded the same way; the chat's `run_sql` is not recorded, as today.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | The page is `/query-studio/history`, a third tab beside Studio and Saved Queries; it shows the caller's own runs only, as the side panel does | none | Owner, 2026-10-24 |
| 2 | 14 days are shown (Snowflake). Older rows stay in the database, unseen | 14 days | Owner, 2026-10-24 |
| 3 | Every run that reaches the server's answer is recorded: completed, failed, and refused (`blocked`: a policy, the tenant gate or the read-only guard said no). Each keeps the message the person saw, which is already the fixed, classified text; the engine's own text is never stored (principle 4). A run stopped with Stop is not recorded: Stop only closes the browser's request today, so the server never learns of it. It is recorded once `QS-2` stops the query on the server | none | Owner, 2026-10-24; the stopped case corrected by the planner the same day |
| 4 | Each run records the tables it read, from the parser the policy step already uses (`sql_rewrite::referenced_tables`). A statement that does not parse records none | none | Owner, 2026-10-24 |
| 5 | The filters run on the server, extending the existing `GET /api/query/history` with query parameters; no new route. 50 rows per page, newest first | 50 per page | Owner, 2026-10-24 |
| 6 | The side panel stays as it is (the last runs, now including failed ones) with a "View all" link to the page | none | Owner, 2026-10-24 |
| 7 | The table filter matches a table name containing the text typed, case-insensitive | none | Owner, 2026-10-24 |
| 8 | Bronze tables in the editor (`QS-1` review fix): their SQL name comes from the existing `assetQueryTarget` once the table's detail is loaded (on expand, or when the statement names it) | none | Owner, 2026-10-24 |

## Limits to tell a customer

- History shows your own runs only. An administrator's view of everyone's runs is not built yet.
- A run stopped with Stop is not in History until the server can stop a query (`QS-2`).
- History covers 14 days. Runs from before this release have no table list, so the table filter misses them.
- A Bronze table's name is known once its detail is loaded, so the first suggestion for a new Bronze table may wait
  for that request.
- History is per user, not per tenant: a user who changes tenant still sees their own earlier runs from the
  other tenant.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `QS-4-AC1` | (operator) Have a run on a table recorded 10 days ago; open History and filter on that table | The run is listed | Not run |
| `QS-4-AC2` | Run SQL that fails, then SQL a policy refuses; open History | Two rows, failed and refused, each with the message shown at the time | Not run |
| `QS-4-AC3` | Sign in as a user without `query:read` and open History | Refused with a permission message | Not run |
| `QS-4-AC4` | Sign in as a second user and open History | None of the first user's runs is listed | Not run |
| `QS-4-AC5` | (operator) Have a run recorded 15 days ago | It is not listed | Not run |
| `QS-4-AC6` | Filter on engine Trino and a minimum of 5 seconds | Only Trino runs of 5 seconds or more | Not run |
| `QS-4-AC7` | Click a run | Its SQL opens in a new Query Studio tab with its engine | Not run |
| `QS-4-AC8` | In the editor, expand a Bronze table in the schema browser and click it; type its alias and a dot | The name is inserted and its columns are suggested | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status updated (history part), with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
