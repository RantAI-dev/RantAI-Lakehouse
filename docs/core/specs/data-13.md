# `DATA-13` Column-level lineage

| | |
| --- | --- |
| Backlog | `DATA-13` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data, with the pipelines stream |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Column mappings are never recorded, so the column-lineage view is hidden as always empty.

## What users get

See which source columns feed a Gold column, and what breaks if a source column changes.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Column mappings | Always empty | Recorded automatically for every pipeline, Gold build and saved SQL source |
| View | Hidden | Column lineage on the asset's Lineage tab, upstream and downstream |
| Impact | None | From a source column, list every downstream column, chart and dashboard |
| Retention | n/a | 1 year (both competitors) |

## Benchmark

Databricks and Snowflake: automatic column-level lineage, kept 1 year. External lineage stays GOV-3.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A Gold column shows the Bronze columns it comes from
- [ ] A Bronze column lists the dashboards that depend on it
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Lineage from systems outside the lakehouse: GOV-3

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
