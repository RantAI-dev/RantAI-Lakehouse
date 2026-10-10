# Natural-language queries respect masking and permissions

| | |
| --- | --- |
| Module | Assistant, Query Studio |
| Backlog | `SEC-9` |
| Status | Decisions signed 2026-10-10 |
| Plan | `docs/superpowers/plans/2026-10-10-sec-9-natural-language-queries.md` |

## Problem

The spec was written against endpoints (`routes/agent.rs`, an agentic Ask
and a text-to-SQL route) that ran model-written SQL straight against
`ClickHouse` for any signed-in user. Those endpoints were removed by #78:
the natural-language box in Query Studio now asks the chat engine, whose
`run_sql` tool calls the Query Studio handler.

Re-checked by the planner on `fix/sec-phase-0` on 2026-10-10, on a running
API as Platform Admin and in the code:

| Target | State |
| --- | --- |
| Model-written SQL goes through the masking and row-filter rewrite | Met. `run_sql` calls `routes::query::run`, which calls `rewrite_sql_for_principal`; a test pins that `run_sql` is masked the same way Query Studio is. |
| `query:read` is needed | Met. `/api/ai/chat` and `/api/ai/tool` need only a sign-in, but the tool gate refuses any tool whose `ToolSpec::permission` the caller lacks, and `run_sql` and `run_saved_query` declare `query:read`. |
| Tenant gate as in Query Studio | Met by construction: it is the same handler. |
| Fixed, classified error text | Met by `SEC-11`, with one thing to state plainly: because `run_sql` is the Query Studio handler, it returns the engine's diagnosis of a statement error to the model, as Query Studio does to an author. The caller holds `query:read` and it is their question's statement. |
| A table-function statement is refused before reaching `ClickHouse` (`SEC-9-AC3`) | **Not met.** `SELECT * FROM url('http://169.254.169.254/…')` is refused with "table function not allowed", and nothing is fetched, but the engine's query log shows it received `EXPLAIN AST SELECT * FROM url(…)` first: the tool's engine dry run comes before the policy refusal. |

## What the user can do when this is done

1. Ask in plain language and never see more than they could see in Query Studio (already true).
2. Be refused by the assistant's SQL tools without `query:read` (already true).
3. Have a statement the policy refuses (a table function, a refused system table) stopped before any part of it is sent to the database.

## Not included

- Changes to the model's prompts.
- Widening the list of refused system tables. Observed while testing and recorded as a follow-up: `system.users` is readable through Query Studio and therefore through the assistant; it lists database account names, not credentials.

## Asking the assistant

This item is about the assistant's SQL tools; nothing new to ask.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Close only the remaining gap (order of checks) and pin the met targets with tests; do not rebuild what #78 already replaced | Planner | Owner said to proceed with `SEC-9`, 2026-10-10 |

## Limits to tell a customer

- The assistant can run only what the same person could run in Query Studio.
- When a statement fails, the assistant is told the database's description of the mistake so it can correct the statement; it is the same text the person would see in Query Studio.

## Acceptance checklist

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | `SEC-9-AC1`: as a user with a masking policy on a column, ask the assistant for that column | The masked value | |
| 2 | `SEC-9-AC2`: as a user without `query:read`, ask the assistant to run SQL | Refused: the permission is named | |
| 3 | `SEC-9-AC3`: ask the assistant to run `SELECT * FROM url('http://169.254.169.254/')` | Refused; the database's query log has no statement containing that address (operator) | |
| 4 | `SEC-9-AC4`: make a statement fail | An honest message with a reference id | |
| 5 | Ask a normal question that needs SQL | Answered as before | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
