# `DATA-14` Data quality in depth

| | |
| --- | --- |
| Backlog | `DATA-14` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Quality checks are hand-written rules and freshness against a fixed target; there is no library, no anomaly detection, no overview.

## What users get

Quality checked automatically, with a clear list of what is wrong.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Ready checks | Hand-written rules | Library: null count and percent, duplicate count, row count, freshness, accepted values, min/max/average, unique count (Snowflake's set) |
| Expectations | n/a | Each check takes a threshold; breaking it opens an incident |
| Schedules | Not checked | Per table: on every load, or every 1 hour to every day |
| Anomaly detection | Freshness against a set target only | Automatic volume and freshness anomalies learned from at least 14 days of history *(proposed)* |
| Overview | None | A quality page listing open incidents by table, with history |
| Alerts | None | Through SRC-7's channels |

## Benchmark

Snowflake (Ent+): system metric functions (nulls, duplicates, freshness, row count, statistics), expectations, schedules, volume and freshness anomaly detection, monitoring dashboard with incidents. Databricks: anomaly detection on freshness and completeness, profiling.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] Adding a 'no nulls' check on a column with nulls opens an incident
- [ ] A load with 10% of the usual rows is flagged as a volume anomaly
- [ ] The quality page lists the incident until it is resolved
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- AI-suggested checks (AI team, AI-16)

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
