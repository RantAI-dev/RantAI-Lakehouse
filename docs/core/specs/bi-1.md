# `BI-1` Export PDF and Excel

| | |
| --- | --- |
| Backlog | `BI-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

PDF is the browser's print, export is CSV only, and the server CSV route has no button.

## What users get

Download a dashboard as a PDF and any chart's data as Excel or CSV.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Dashboard PDF | Browser print | Server-made PDF of the whole dashboard, A4 or letter, portrait or landscape, with filters applied and listed |
| Chart data | CSV only | CSV and Excel (.xlsx), formatted or raw values |
| Row limit | n/a | Up to 1,000,000 rows (Metabase) |
| Button | Endpoint exists, no button | A download menu on every chart and dashboard |
| Permissions | n/a | Downloads apply masking; an admin can switch downloads off per role |

## Benchmark

Metabase: dashboard PDF, chart downloads CSV/XLSX/JSON/PNG up to 1,000,000 rows. Tableau: PDF, PNG, crosstab to Excel.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BI-1-AC1` A dashboard downloads as a PDF matching the screen
- `BI-1-AC2` A 200,000-row chart downloads as Excel
- `BI-1-AC3` A masked column is masked in the download
- `BI-1-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- PowerPoint export (owner decision)

## Asking the assistant

Ships with [`AI-10`](ai-10.md) (Reports and exports), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
