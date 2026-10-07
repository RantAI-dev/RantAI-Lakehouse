# `SRC-11` Connector operations

| | |
| --- | --- |
| Backlog | `SRC-11` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Sources) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Ingest jobs have no checked retry policy, no readable log per run, and column choice is unverified.

## What users get

Runs that recover by themselves and leave a readable trail.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Retries | Not checked for ingest jobs | Up to 3 retries per run with backoff 1, 5 and 15 minutes *(proposed; Airbyte retries more)*; transient errors only |
| Auto-disable | None | A connector failing 30 runs in a row is paused and an alert sent (Airbyte's threshold) |
| Run log | Per-table results only | A log per run and per attempt, kept 30 days *(proposed)*, viewable and downloadable |
| Run history | Not checked | A timeline of the last 50 runs with status, rows, duration *(proposed)* |
| Column selection | Not checked | Choose columns per table, not only tables |

## Benchmark

Airbyte: retries with backoff, auto-disable after 30 consecutive failures, per-attempt logs, a timeline, field selection. Databricks: retries and event logs.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A transient failure (database briefly down) succeeds on retry without anyone acting
- [ ] The run log shows each attempt
- [ ] Deselecting a column removes it from the next load
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
