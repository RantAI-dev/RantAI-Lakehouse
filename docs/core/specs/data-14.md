# `DATA-14` Data quality in depth

| | |
| --- | --- |
| Backlog | `DATA-14` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec corrected against the code 2026-10-09. Not planned, not built |

## Why

Four ready checks exist and run hourly, and a page lists their latest results. A failed check tells nobody, nothing is learned from history, and there is no list of what is wrong now.

## What users get

Quality checked automatically, with a clear list of what is wrong.

## Target specs

"Today" is `main` at `c338862`, re-read from the code on 2026-10-09, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Ready checks | Four: row count, column filled in (percent), no repeated values, value within a range. Picked from a form on the asset page; typed as text on the Data Quality page. Missing: accepted values, average, unique count, freshness as a check | Library: null count and percent, duplicate count, row count, freshness, accepted values, min/max/average, unique count (Snowflake's set) |
| Expectations | Every rule has a threshold and a passed, warning or failed result, which feeds the asset's health. A failure opens no incident | Each check takes a threshold; breaking it opens an incident |
| Schedules | One setting for all rules: a rule runs again when its result is over 60 minutes old, 25 rules per 15-minute tick, and on demand. No per-table choice, nothing runs on load | Per table: on every load, or every 1 hour to every day |
| Anomaly detection | Freshness against a set target only. Volume: an optional alert per pipeline when a run loads under half the median of earlier runs; fixed, per pipeline, not per table | Automatic volume and freshness anomalies learned from at least 14 days of history *(proposed)* |
| Overview | A Data Quality page lists every rule with its latest result. No incidents, no grouping by table. Every run is stored but only the latest is shown, and editing a rule deletes its runs | A quality page listing open incidents by table, with history |
| Alerts | None | Through SRC-7's channels |
| Who may add a rule | Any signed-in user; editing and deleting need `governance:write` | Adding a rule needs the permission editing one needs |

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
