# No raw upstream errors in responses

| | |
| --- | --- |
| Module | Platform (API) |
| Backlog | `SEC-11` |
| Status | Decisions signed 2026-10-10 |
| Plan | `docs/superpowers/plans/2026-10-10-sec-11-no-raw-upstream-errors.md` |

## Problem

Dashboard tiles, including on public and embed links, show the database's
own error text when a query fails (seen on 2026-10-07: a tile printed
`Code: 184. DB::Exception: …` with the server version). The assistant's
tools and several catalog, connector and alert routes do the same. That
text names tables, columns, versions and hosts to whoever is looking,
signed in or not. Principle 4 in `AGENTS.md` forbids it.

## What the user can do when this is done

1. See a short, fixed message on a failed tile ("This chart could not be loaded"), on the console, on a public link and in an embed.
2. Quote a reference id from that message to an administrator.
3. As an operator, find the full error in the server log by that id.
4. Keep seeing our own messages unchanged: validation errors, "not found", permission refusals and policy refusals still say what is wrong.
5. As the author of a statement in Query Studio, see what kind of failure it is (a missing table or database, a missing column, a syntax error, a wrong type, too little memory, or that the engine could not run the statement) in a fixed sentence, with a reference id and no engine text.

## Not included

- Changing HTTP status codes of existing routes, except where a status was derived from upstream text.
- A screen for browsing errors by id.
- Rewriting history or logs already written.

## Asking the assistant

The assistant's tools return the same fixed messages, so it can tell the
user a step failed and give the reference id, and cannot repeat database
text it was never given.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Scope is every route under `routes/`, not only the files the audit named | Planner default | Owner said to proceed with phase 0, 2026-10-10 |
| 2 | The raw text goes to the log only, keyed by a reference id shown to the user | Spec | 2026-10-10 |
| 3 | A guard test fails the build when a route builds a response from an upstream error's text | Spec `SEC-11-AC1` | 2026-10-10 |
| 4 | Query Studio (run and cost estimate) shows a fixed sentence for the statement's cause, followed by ` Reference: <id>`, never the engine's own text. The raw text goes to the log under that id. Nothing else shows engine text: tiles, public links, embeds, SQL sources and saved-query runs stay closed. The assistant's `run_sql` tool is the Query Studio handler, called for a signed-in caller holding `query:read`, and it still returns the engine's trimmed diagnosis to the model so the model can correct its SQL (seen on a running API while checking `SEC-9`); the assistant's other tools stay closed. An unreachable, timed-out or stopped engine keeps its fixed message. | Planner recommended it after the first build hid these errors | Owner, 2026-10-10; changed: Owner, 2026-11-12, LH-QS-2-08 |
| 5 | `POST /api/pipelines/{id}/trigger` answers `Dagster`'s `PipelineNotFoundError` with 503, `Retry-After` and `orchestrator_job_not_loaded`, not `#80`'s 404. An unknown or out-of-scope pipeline id is refused before any `Dagster` call (`authored_pipelines::in_scope`, `authored_launch_target`), so a not-found after that is a job the product knows that the orchestrator has not loaded: a code location reloading (retry) or a broken one (an operator problem). Neither is the caller's 404. The console shows it as unavailable with the fixed sentence, which says to try again | Planner, PR #105 review | Owner waived the review on 2026-10-15; confirm at QA |
| 6 | `orchestrator_unreachable` (the code the gates retry) is given only to a transport failure and an HTTP status of 500 or more from `Dagster`. A GraphQL error, an answer that does not parse, or a 4xx is a 502 with the fixed message and no `code`, so a protocol bug is not retried as an outage | Planner, PR #105 review | 2026-10-15 |

## Limits to tell a customer

- A failed chart says that it failed and gives a reference id; it does not say why. The reason is in the server log.
- Messages written by the product itself (a bad filter, a missing permission) are unchanged.
- A condition a client must act on is told by a stable `code` field beside `error`, never by the message text. Today: `orchestrator_job_not_loaded` and `orchestrator_unreachable` (HTTP 503 with `Retry-After`, from `POST /api/connectors/{id}/ingest/run`; the first also from `POST /api/pipelines/{id}/trigger`). The acceptance gates retry on these codes only (PR #105 CI fix).
- In Query Studio the author sees a fixed sentence for the kind of failure and a reference id, not the database's own description. The description is in the server log under that id. The assistant's `run_sql` tool still gives the model the database's description, so it can correct the statement (decision 4).

## Acceptance checklist

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | `SEC-11-AC1`: run the guard test | Passes; adding `err.to_string()` of an upstream error to a route makes it fail | |
| 2 | `SEC-11-AC2`: force a `ClickHouse` error on a tile (a chart over a dropped table) and open the dashboard's public link | The fixed message and a reference id; no database text, no version | |
| 3 | Same dashboard in the console and in an embed | Same fixed message | |
| 4 | Search the API log for the reference id (operator) | One line with the full upstream error | |
| 5 | Send a malformed dashboard filter | Our own message, unchanged (400) | |
| 6 | `SEC-11-AC3`: as a role without `dashboard:read`, open a dashboard | Refused with the usual permission message | |
| 7 | Ask the assistant to run a query against a table that does not exist | It reports a failure with a reference id, not database text | |
| 8 | Stop `ClickHouse` and open a dashboard (operator) | Fixed "unavailable" message; nothing upstream | |
| 9 | In Query Studio run a statement with a misspelt column | "This query names a column that does not exist. Reference: …", with no `Code:`, no engine text, no version and no host | |
| 10 | Put the same misspelt column in a dashboard SQL source chart | The fixed message, not the engine's | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
