# Connector passwords cannot be stolen by re-pointing a connector

Backlog `SEC-14`. Spec: [`../specs/sec-14.md`](../specs/sec-14.md).
Plan: `docs/superpowers/plans/2026-10-08-sec-14-connector-repoint.md`.

## What the user can rely on

Changing where a connector points never sends its stored password to the
new place. To point a connector somewhere else, the person changing it
gives the credentials again in the same step.

## Decisions

The product owner asked on 2026-10-08 for the Phase 0 security items to be
fixed as their specs say. The spec's Target table is taken as signed, and
the plan's proposed decisions are taken as proposed on that instruction;
any of them can still be changed before the part that implements it merges.

| # | Decision | Choice |
| --- | --- | --- |
| D1 | What counts as re-pointing | A change to the connector's target identity: the fields in the plan's section 3 (for a database: driver, host, port, database). |
| D2 | Without credentials | Refused with 409 and a fixed message; nothing is written and nothing is dialled. |
| D3 | Which credentials | Every slot the connector has after the change. |
| D4 | Changing `host` through the general edit route | Follows the same rule. Not in the spec's table; added because it is the same path. |
| D5 | PostgreSQL TLS | `require` at least; `verify-full` when a CA file is configured. |
| D6 | Where the CA comes from | One setting, `CONNECTOR_TLS_CA_FILE`, for PostgreSQL and SQL Server. |
| D7 | The compose PostgreSQL | Gets TLS, and the API is given its CA. |
| D8 | No clear-text authentication | The connection test uses a small client of its own that accepts SCRAM-SHA-256 only. |
| D9 | MySQL | Not in this item. |
| D10 | The orchestrator's own connections | Not in this item; the re-point rule covers what they are given. |

## Limits

- A connector whose password may already have been captured is not
  repaired by this: its password has to be changed at the source.
- With `require` and no CA configured, the connection is encrypted but
  the server's identity is not checked.

## Acceptance checklist (product owner, on a running console)

- [ ] Every Target row of the spec works as written
- [ ] Re-pointing the seeded connector without new credentials returns 409
- [ ] With new credentials in the same step, the change is saved and the test runs against the new place
- [ ] Editing a connector's host in the console asks for the credentials again
- [ ] A test against a server that asks for clear-text password authentication never sends the password
- [ ] A user without the permission is refused
