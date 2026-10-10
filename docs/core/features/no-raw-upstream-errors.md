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
5. As the author of a statement in Query Studio, still see the engine's diagnosis of that statement (an unknown column, a syntax error, a type mismatch), without the server's version or any host, and with a reference id.

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
| 4 | Query Studio (run and cost estimate) shows the author the engine's diagnosis of their own statement. Nothing else does: tiles, public links, embeds, SQL sources, the assistant and saved-query runs stay closed. An unreachable or timed-out engine is still the fixed message. | Planner recommended it after the first build hid these errors | Owner, 2026-10-10 |

## Limits to tell a customer

- A failed chart says that it failed and gives a reference id; it does not say why. The reason is in the server log.
- Messages written by the product itself (a bad filter, a missing permission) are unchanged.
- In Query Studio the author sees the database's own description of what is wrong with their statement. That text can name tables and columns the author asked about; it never includes the server version or an address.

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
| 9 | In Query Studio run a statement with a misspelt column | The engine's message naming the column, a reference id, no version and no host | |
| 10 | Put the same misspelt column in a dashboard SQL source chart | The fixed message, not the engine's | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
