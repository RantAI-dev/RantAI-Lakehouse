# `SEC-17` Upload hardening

| | |
| --- | --- |
| Backlog | `SEC-17` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| When | Next |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

An upload with one very long header line can create millions of columns; there is no limit on concurrent uploads; cells starting with =, +, - or @ export as formulas.

## What users get

An upload cannot exhaust the ingestion process, and exported files cannot run formulas.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Columns per file | No cap | 1,000 columns *(proposed)*; more is refused with a fixed message |
| Concurrent uploads | No limit | 4 per user and 16 per install, configurable *(proposed)* |
| CSV export of cells starting with = + - @ | Exported as they are | Prefixed with a quote so spreadsheets treat them as text |

## Benchmark

From the Data audit; not re-checked. Snowflake's schema evolution caps at 100 new columns per load by default.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SEC-17-AC1` A file whose header is 50 MB of commas is refused quickly
- `SEC-17-AC2` A fifth concurrent upload by one user waits or gets 429
- `SEC-17-AC3` An exported =HYPERLINK cell opens as text
- `SEC-17-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
