# `DATA-13` Column-level lineage

| | |
| --- | --- |
| Backlog | `DATA-13` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Also involves | Build |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code on 2026-10-09 |

## Why

Column mappings are never recorded, so the column-lineage view is hidden as always empty.

## What users get

See which source columns feed a Gold column, and what breaks if a source column changes.

## Target specs

"Today" is `main` at `c338862`, re-read from the code on 2026-10-09, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Column mappings | Always empty. No lineage is stored: the table-level map is rebuilt on every request. Pipelines already record their rename, cast and select steps, but lineage does not read them | Recorded automatically for every pipeline, Gold build and saved SQL source |
| View | The Lineage tab shows the table-level map. A column list is coded but shows only when there are mappings, so never | Column lineage on the asset's Lineage tab, upstream and downstream |
| Impact | Table level only: downstream tables, and the saved queries and dashboards that read the table directly (not those reached through a Silver or Gold table). Dashboards are listed with a chart count, not chart by chart. Nothing starts from a column | From a source column, list every downstream column, chart and dashboard |
| Retention | n/a: nothing is stored. Build links are read from the last 180 days of the ClickHouse query log | 1 year (both competitors) |

Found while re-reading, not run: the lineage gate (`ops/g3a/g3a_test.py`, `RECORDED_LINEAGE_EDGE_KINDS`) does not list the `build` link kind that `routes/lineage/builds.rs` emits, and fails on any kind it does not list. The plan for this task checks it.

## Benchmark

Databricks and Snowflake: automatic column-level lineage, kept 1 year. External lineage stays GOV-3.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-13-AC1` A Gold column shows the Bronze columns it comes from
- `DATA-13-AC2` A Bronze column lists the dashboards that depend on it
- `DATA-13-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Lineage from systems outside the lakehouse: GOV-3

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
