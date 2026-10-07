# `SEC-11` No raw database errors on screen

| | |
| --- | --- |
| Backlog | `SEC-11` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security, shared with the AI team for the agent endpoints |
| When | Now |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Dashboard tiles, including on public and embed links, and several catalog, connector and agent routes return raw upstream error text. Principle 4 forbids it.

## What users get

Viewers see a clear, fixed message; internal details stay in the server log.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Tile errors (also public and embed) | Raw ClickHouse message | Classified fixed message; the raw text only in the log with a correlation id |
| Agent endpoints | Raw model and ClickHouse errors | Classified |
| Catalog and CDC-deprovision errors | Raw text in 503/409/500 bodies | Classified |

## Benchmark

From the BI and Data audits: routes/support.rs, dashboard.rs, embed.rs, agent.rs, catalog.rs, connector_deprovision.rs.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A grep test or unit test shows no route builds a response from err.to_string() of an upstream error
- [ ] A forced ClickHouse error on a public dashboard shows the fixed message
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- The fourteen older Phase-1 handlers: SEC-6

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
