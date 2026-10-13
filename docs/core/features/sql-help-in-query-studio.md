# SQL help in Query Studio

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `AI-12` |
| Kind | Feature |
| Spec | `docs/core/specs/ai-12.md` (no *(proposed)* number; the spec's one acceptance row is `AI-12-AC1`) |
| Status | Building |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-13; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | |
| Plan | Plan files are not kept in the repository. |

## Problem

A query that fails in Query Studio shows its error and nothing else. The person reads the engine's message and fixes the statement by hand, or copies it into the chat and asks there, which leaves the statement, the engine and the error apart. Reading a statement someone else wrote is the same problem without an error to start from.

## What the user can do when this is done

1. Press Explain in the SQL tab and read a plain-words explanation of the statement, on ClickHouse and on Trino.
2. Run a statement that fails on ClickHouse, press Fix with AI, and read one sentence on what was wrong with a line diff of the corrected statement.
3. Press Accept and get the proposal in the editor, with nothing run.
4. Press Reject and keep the editor as it was.

## Not included

- Variables in the statement. **Waits for `QS-5`.**
- An optimise action. The spec names one against Databricks and Snowflake; it is not built here.
- Autocomplete by AI (the spec lists it as later).
- Fix with AI on Trino. It is a later backlog item, after another stream repairs Trino's catalog access on staging; until then the button is disabled on Trino and says why.

## Asking the assistant

This feature is the assistant working inside Query Studio. The user asks with one fixed sentence: "Why does this query fail? Fix it." with the engine, the SQL and the error the run raised, or "Explain what this query does." with the engine and the SQL. For an explanation the assistant writes plain words and calls no tool; the server offers it none in that turn. For a fix it proposes the whole corrected statement through `propose_sql`, a checked, read-only tool that never runs it. A proposal that fails the server's check comes back to the model as an error, so the model can correct it in the same turn; only a checked proposal reaches the person.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | The buttons sit in Query Studio's SQL tab, and the explanation and the proposal show on that page | none | Owner, 2026-10-13 |
| 2 | The fix is a line diff, written in this repository, with no new dependency | none | Owner, 2026-10-13 |
| 3 | This pull request holds Explain for both engines and Fix with AI for ClickHouse; Fix for Trino is a later backlog item | none | Owner, 2026-10-13 |
| 4 | One new read-only tool, `propose_sql`, which validates the statement and echoes it back | none | Planner's choice, 2026-10-13 |
| 5 | The server checks a proposal without running it: the read-only guard, the policy rewrite as the caller, then `EXPLAIN PLAN` on ClickHouse | none | Planner's choice, 2026-10-13 |
| 6 | A proposal that fails the check returns to the model as an error, so the model can correct it in the same turn | none | Planner's choice, 2026-10-13 |
| 7 | One switch, `AI_SQL_HELP`, on when unset or empty | on | Planner's choice, 2026-10-13 |
| 8 | The diff is computed in the console, line by line, so the server never sees the editor's text in a second form | none | Planner's choice, 2026-10-13 |

## Limits to tell a customer

- A proposal is checked for the caller's permissions and for the names the engine resolves, and it is never run. A statement that plans can still return rows other than the ones the person meant.
- The console refuses to send a statement of more than 20,000 characters to be fixed; it says the query is too long for the assistant to fix.
- The assistant sees the tables the chat's data map lists, and learns of other tables only from the error text.
- The diff is against the editor as it was when the proposal came back. Typing in the editor closes the proposal, and so does running the query.
- In an Explain turn the assistant is offered no tool, so it can neither propose a fix nor run a query.
- A chat the console opened with a tool list refuses any tool outside that list.
- `AI_SQL_HELP=false` takes the `propose_sql` tool and the rules that tell the model when to use it out of the chat. The Explain and Fix with AI buttons stay in Query Studio's SQL tab: Fix with AI then shows the assistant's answer and "No fix to apply was proposed.", never a diff.
- `AI_SQL_HELP` set to anything other than `true` or `false` stops the API at start. Unset or empty means on.

## Acceptance checklist

Run on a running deployment. Mark each Pass, Fail, or Not run with the reason. A step not performed is never Pass. Nothing has to be set first: the switch is on when unset. Rows 1 to 5 use the seed data.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | In Query Studio's SQL tab, run on ClickHouse a statement against a `silver` table that fails on a misspelled column, then press "Fix with AI". | The page shows one sentence on what was wrong and a line diff that names the right column. Nothing runs, so the editor holds the statement that failed until Accept. | |
| 2 | With a ClickHouse statement in the editor, press "Explain". | The page explains the statement in plain words, and no query runs. | |
| 3 | With a Trino statement in the editor, press "Explain". | The page explains the statement in plain words, and no query runs. | |
| 4 | After a proposal is shown, press Reject. | The proposal closes and the editor holds the statement it held before. | |
| 5 | After a proposal is shown, press Accept. | The editor holds the proposed statement and nothing runs; rows come only when the person presses "Run query". | |
| 6 | Sign in as a user without `query:read` and ask for the same fix on the statement from row 1. | No proposal is returned and no diff is shown. | |
| 7 | Set `AI_SQL_HELP=false` and restart the API (operator). Run the statement from row 1 and press "Fix with AI". | No diff is offered. The card shows the assistant's answer and "No fix to apply was proposed." The buttons themselves stay in the SQL tab. | |
| 8 | Set `AI_SQL_HELP=maybe` and restart the API (operator). | The API stops with a `ConfigError` naming the accepted values. | |

The proposal is checked on the server and the diff is drawn in the console, so rows 1 to 5 are what a person sees. Row 6 is the permission the tool carries, `query:read`, the same one running the statement in Query Studio asks for.

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read