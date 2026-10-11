# `DATA-19` Uploaded tables feed pipelines and dashboards

| | |
| --- | --- |
| Backlog | `DATA-19` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Also involves | Build |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

An uploaded table cannot be the source of a pipeline, so it never reaches Silver, Gold or a dashboard.

## What users get

An uploaded table becomes a Silver or Gold table and appears on a dashboard, like any other table.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Pipeline source | Not possible | An uploaded table is selectable as a pipeline source |
| Re-upload | Replace or append | A re-upload triggers the downstream pipeline, as a connector run does |
| Lineage | None | Lineage shows the upload as the origin |

## Benchmark

Both competitors treat an uploaded table like any other table.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-19-AC1` Upload, build a Silver table from it, chart the Gold table: all from the console
- `DATA-19-AC2` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
