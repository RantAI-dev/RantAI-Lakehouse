# Query Studio editor basics

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-1` |
| Kind | Feature |
| Spec | `docs/core/specs/qs-1.md` (one *(proposed)* number, the 200 ms suggestion time, is decision 2) |
| Status | Building |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-23; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | |
| Plan | Plan files are not kept in the repository. |

## Problem

Autocomplete in the SQL tab suggests table names and nothing else. The hook
fills the editor's schema with an empty column list for every table
(`src/features/queries/use-query-studio.ts:75-87`), because the columns sit on
each table's detail and the console never fetches them. The editor holds one
statement in one buffer: the server runs exactly one statement per request
(`rust/crates/lakehouse-api/src/routes/query.rs:267`), and the console has no
way to run several or only the selected part. There is one draft, not several
tabs. The formatter puts clauses on new lines and uppercases keywords, with
no settings (`src/lib/sql-format.ts`). Nobody has checked find and replace,
comment toggling, bracket matching or folding in this editor.

## What the user can do when this is done

1. Type a table alias and a dot, and pick from that table's columns.
2. Get suggestions for keywords, schemas, tables, columns and aliases, with
   the ones that fit the cursor's place in the statement first.
3. Open a schema browser beside the editor, expand a layer, a table and its
   columns with their types, and click a name to insert it at the cursor.
4. Write several statements in one tab and press Run all: each statement gets
   its own result tab.
5. Select part of the text and run only the selection.
6. Open several query tabs, rename and close them, and find them all again
   after closing and reopening the browser.
7. Choose keyword case and indentation for Format, and have the choice kept.
8. Use find and replace, toggle a comment, see the matching bracket, fold a
   block, and open a list of the keyboard shortcuts.
9. Reload the page and find unsaved text where it was.

## Not included

- Real-time co-editing and git *(proposed drop in the spec)*.
- Tabs on another device or browser. Tabs live in this browser only (decision 5).
- Running several statements in one transaction. Run all sends them one after
  another (decision 3).
- Autocomplete by AI, and help from the assistant inside the editor (`AI-12`).

## Asking the assistant

No. `QS-1` changes the editor, not what the assistant can do. Help inside the
SQL editor is `AI-12`. The page context the assistant already reads (the SQL
and the last result) keeps working, and follows the active query tab.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Columns for autocomplete and the schema browser come from the catalog's existing table detail (`GET /api/catalog/{id}`), fetched the first time a table is named or expanded and kept for the session. No new server route: `policy.rs` and `routes/mod.rs` are held by open PRs. Each first fetch also reads a 5-row governed sample on the engine, as opening the table's catalog page does (`routes/catalog.rs:1987`); a columns-only route is a follow-up once those PRs merge | Fetch on first use, keep for the session | Owner, 2026-10-23 |
| 2 | Suggestions show within 200 ms of a keystroke once the table's columns are loaded *(proposed in the spec)*. The first time a table is named, its columns may take one request longer, and the list says it is loading them | 200 ms | Owner, 2026-10-23 |
| 3 | Run all sends the statements one after another, as separate requests, in the order written. It stops at the first failure, and the statements after it show "Not run" in their tabs. The server keeps its one-statement rule | Stop at the first failure | Owner, 2026-10-23 |
| 4 | Run all runs at most 20 statements; more is refused with a message before anything runs *(proposed)* | 20 | Owner, 2026-10-23 |
| 5 | Query tabs are stored in this browser (`localStorage`), not on the server: each tab keeps its name, SQL and engine, not its result. At most 20 tabs *(proposed)* | Browser only, 20 tabs | Owner, 2026-10-23 |
| 6 | ⌘/Ctrl+Enter runs the selection when there is one, and otherwise the whole tab (Run all). Today it runs the whole buffer | Selection, else all | Owner, 2026-10-23 |
| 7 | Format keeps this repository's small formatter and adds two settings, kept in this browser: keyword case (upper, lower, as typed) and indent (none, 2 or 4 spaces). Indent puts each clause's body on the line after its keyword, indented by that width; none is today's output. No new dependency | Upper case, no indent (today's output) | Owner, 2026-10-23; indent default corrected by the planner the same day, because today's formatter does not indent and a 2-space default would change everyone's output |
| 8 | Find and replace, comment toggling, bracket matching and folding come from the editor component already installed; the work is to check each one and add the shortcut list | none | Owner, 2026-10-23 |
| 9 | The schema browser sits beside the editor and can be closed; it lists only the tables the catalog shows this user | Open on wide screens, closed on narrow ones | Owner, 2026-10-23 |
| 10 | The Natural language and SQL tabs stay. Query tabs live inside the SQL tab | none | Owner, 2026-10-23 |
| 11 | No on/off switch. This is the editor itself, and every part degrades on its own: without column data the editor still runs SQL | none | Owner, 2026-10-23 |
| 12 | Explain, Fix with AI, Estimate, Save query and Save as SQL source work on one statement: the selection when there is one, else the whole tab when it holds one statement. With several statements and no selection they are disabled, and the button's title says to select one statement. A saved query or SQL source with several statements would fail where it is run later (the server runs one statement per request) | none | Planner's choice, 2026-10-23 |
| 13 | Fix with AI is offered on a failed statement's result tab and proposes a fix for that statement; Accept replaces that statement's text in the editor and nothing else | none | Planner's choice, 2026-10-23 |

## Limits to tell a customer

- Query tabs are kept in the browser they were opened in. Clearing the
  browser's site data removes them; saved queries are not affected.
- Run all is not a transaction. A statement that ran before a failure is not
  undone (the editor runs read-only statements, so nothing was written).
- Autocomplete and the schema browser know only the tables the catalog shows
  you. A table you can query but the catalog does not list gets no column
  suggestions. A Bronze table's SQL name is known once its detail is loaded:
  expand it in the schema browser, or write its `namespace.name` in the
  statement. A name only Trino serves is offered on a Trino tab only.
- Run all takes at most 20 statements, and a browser keeps at most 20 tabs.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never
a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `QS-1-AC1` | In the SQL tab, type `SELECT o. FROM <a Silver or Gold table, or a Bronze one whose detail is loaded> o` and put the cursor after `o.` | The list shows that table's columns | Not run |
| `QS-1-AC2` | Write three statements separated by `;` and press Run all | Three result tabs, one per statement, in order | Not run |
| `QS-1-AC3` | Open three query tabs with different SQL, close the browser, open Query Studio again | The three tabs are back with their names and SQL | Not run |
| `QS-1-AC4` | Sign in as a user without `query:read`, open Query Studio and press Run all | Every statement is refused with a permission message, and nothing runs on the engine (`POST /api/query/run` needs `query:read`, `policy.rs:412`) | Not run |
| `QS-1-AC5` | Write three statements where the second fails, press Run all | Tab 1 shows rows, tab 2 shows the engine's error as the console reports it today, tab 3 says "Not run" | Not run |
| `QS-1-AC6` | Select the second of three statements and press ⌘/Ctrl+Enter | Only that statement runs | Not run |
| `QS-1-AC7` | In the schema browser, expand a layer, a table, and click a column | The column name is inserted at the cursor; each column shows its type | Not run |
| `QS-1-AC8` | Set keyword case to lower and indent to 4, press Format, reload, press Format again | Both times keywords are lower case and indented by 4 | Not run |
| `QS-1-AC9` | Use find and replace, toggle a comment, fold a block, and open the shortcut list | Each works, and the shortcut list names the keys used | Not run |
| `QS-1-AC10` | (operator) Make the catalog's table detail fail for one table, then type its alias and a dot | The list says the columns could not be loaded; the query still runs | Not run |
| `QS-1-AC11` | Sign in as a user without `catalog:read` but with `query:read` | The schema browser says it needs catalog access; the editor still runs SQL | Not run |
| `QS-1-AC12` | Paste 21 statements and press Run all | Refused before anything runs, with a message naming the limit of 20 | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
