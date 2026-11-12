# Query history page

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-4` (history part) |
| Kind | Feature |
| Spec | `docs/core/specs/qs-4.md` (no *(proposed)* number; the 14 days are Snowflake's) |
| Status | In Acceptance |
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
2. Filter by status (completed, failed, stopped, refused), engine, minimum duration, a table the run read, and a
   time range within the 14 days.
3. See failed, stopped and refused runs as well as completed ones, each with a plain sentence that says why.
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
| 3 | Every run that reaches the server's answer is recorded: completed, failed, stopped, and refused (`blocked`: a policy, the tenant gate or the read-only guard said no). A run ended with Stop is recorded as **Stopped**; a run the time limit ended stays failed, with its own sentence. The stored message is a fixed sentence, never the engine's own text, `Code:` or catalog names (principle 4). A run that fails in the engine keeps the sentence for its cause (a missing table or database, a missing column, a syntax error, a wrong type, too little memory, or "The engine could not run this statement.") followed by `Reference: <id>`, the id in the log. The result tab and the run endpoint's 422 body still show the engine's diagnosis (`QS-1-AC5`) | none | Owner, 2026-10-24; changed: Owner, 2026-11-05 |
| 4 | Each run records the tables it read, from the parser the policy step already uses (`sql_rewrite::referenced_tables`). A statement that does not parse records none | none | Owner, 2026-10-24 |
| 5 | The filters run on the server, extending the existing `GET /api/query/history` with query parameters; no new route. Pages are numbered, 50 rows per page, newest first: the request takes `page` (1-based) and the answer is `{ runs, total, page, pageSize }`. The page shows Previous, numbered buttons, Next and "Page N of M"; there is no "Load more" | 50 per page | Owner, 2026-10-24; changed: Owner, 2026-11-05 |
| 6 | The side panel stays as it is (the last runs, now including failed ones) with a "View all" link to the page | none | Owner, 2026-10-24 |
| 7 | The table filter matches a table name containing the text typed, case-insensitive | none | Owner, 2026-10-24 |
| 8 | Bronze tables in the editor (`QS-1` review fix): a Bronze table has two names. The ClickHouse name the server sends (`queryTarget`) stays for the catalog page's policy and the stand-in. Its Trino name, `iceberg.bronze.<table>`, is built from the registry table name the catalog list carries (`tableKey`), so it is known before the table's detail is opened. A Trino tab inserts the Trino name from the schema browser and offers it in the autocomplete only for a Bronze table whose Iceberg table a read has proven to exist (one read of the `bronze` namespace's table list on the first Trino tab, or the table's own read when its entry is expanded) — completion offers only proven Bronze tables (Owner, 2026-11-10) — and after `FROM iceberg.bronze.<table> b` the alias `b.` offers that table's columns. A ClickHouse tab inserts and offers no Bronze table; the table's row says "Bronze tables are read with Trino: switch this tab to Trino to insert it.", also before its detail has loaded. The catalog's "Open in Query Studio" and snapshot links for a Bronze table open a Trino tab for `iceberg.bronze.<table>` only when that Bronze table's Iceberg table exists, which the catalog page knows from the table's detail it loads, and the ClickHouse link otherwise; a Bronze table's starter SQL uses that name on Trino (#409). An asset with no readable table (no `Iceberg` table, no `queryTarget`, no stand-in) offers no `Query Studio` link, no starter SQL and no Copy button, and shows one sentence instead: "No readable table for this asset yet." The sentence appears only once the `Iceberg` load has finished, so it does not flash | none | Owner, 2026-10-24; changed: Owner, 2026-11-05; changed: Owner, 2026-11-06; changed: Owner, 2026-11-08; changed: Owner, 2026-11-09; changed: Owner, 2026-11-10; changed 2026-11-12 (`QS-7`), *(proposed, not yet signed)* |

## Limits to tell a customer

- History shows your own runs only. An administrator's view of everyone's runs is not built yet.
- A statement the Trino engine rejects shows "The engine could not run this statement." plus a reference id, whatever the cause: the Trino client does not keep the error name, so History cannot tell a missing table from a syntax error.
- A failed Trino run recorded before the classification keeps its old message, which can hold the engine's first line. Failed ClickHouse runs from before it show the same default sentence, not their cause. Both age out of the 14-day window.
- History covers 14 days. Runs from before this release have no table list, so the table filter misses them.
- A `DESCRIBE` or `SHOW COLUMNS FROM` statement records the table it names. A `SHOW TABLES` statement records no table.
- On a Trino tab, an alias on `iceberg.bronze.<table>` offers the table's columns one request after the statement first names the table; until then, the list says it is loading them.
- An alias on a quoted Bronze table name, one whose table name is not a plain identifier (for example
  `iceberg.bronze."order lines" b`), does not resolve its columns. The name is still inserted.
- History is per user, not per tenant: a user who changes tenant still sees their own earlier runs from the
  other tenant.

A run of a statement with `{{name}}` parameters (`QS-5`, `docs/core/features/query-parameters.md`)
is recorded as the statement that ran, with the parameter values printed in it as literals, not as the text with
`{{name}}`. Clicking the run or downloading it works without the values.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `QS-4-AC1` | (operator) Have a run on a table recorded 10 days ago; open History and filter on that table | The run is listed | Not run |
| `QS-4-AC2` | Run SQL that fails, then SQL a policy refuses; open History | Two rows, Failed and Refused, each with a sentence that says why; the failed row ends with a reference id and holds no `Code:` or catalog name | Not run |
| `QS-4-AC3` | Sign in as a user without `query:read` and open History | Refused with a permission message | Not run |
| `QS-4-AC4` | Sign in as a second user and open History | None of the first user's runs is listed | Not run |
| `QS-4-AC5` | (operator) Have a run recorded 15 days ago | It is not listed | Not run |
| `QS-4-AC6` | Filter on engine Trino and a minimum of 5 seconds | Only Trino runs of 5 seconds or more | Not run |
| `QS-4-AC7` | Click a run | Its SQL opens in a new Query Studio tab with its engine | Not run |
| `QS-4-AC8` | In the editor on a Trino tab, click a Bronze table in the schema browser without expanding it; type its alias and a dot | The name is inserted and its columns are suggested | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status updated (history part), with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read

## QA-1 decisions (product owner, 2026-11-02)

- A run stopped with Stop is listed in History with the status **Stopped**.
- When a page states a rule but not its exact message, QA checks that the action is refused (or shown) and that the message names the rule. The wording itself is not tested. Defaults the planner wrote on a page count as decided unless the owner changes them.
