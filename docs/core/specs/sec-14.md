# `SEC-14` Connector passwords cannot be stolen by re-pointing a connector

| | |
| --- | --- |
| Backlog | `SEC-14` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security, with the Data stream |
| When | Now |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Saving a connector's ingest spec can change its host while keeping its stored password; the next connection test sends that password to the new server, with a TLS mode the server can decline. Someone allowed to manage connectors can capture the console's own database password.

## What users get

Changing where a connector points can never send its stored password anywhere new.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Host, port, database or adapter change on PUT ingest-spec | Keeps the stored password | Requires new credentials in the same request, otherwise 409 |
| Dial override on PUT credential | Same gap | Same rule for the slot not being changed |
| PostgreSQL test TLS | Prefer (server may decline) | sslmode=require at least; verify-full when a CA is configured; never clear-text password authentication |
| SQL Server test TLS | Encryption optional | Encryption required |

## Benchmark

Re-checked by the planner: lakehouse-store connectors.rs set_ingest_spec keeps secret_ref; connector_probe.rs uses PgSslMode::Prefer.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] Re-pointing the seeded connector without new credentials returns 409
- [ ] A test against a server that asks for clear-text password authentication never sends the password
- [ ] A regression test reproduces the attack and now fails
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
