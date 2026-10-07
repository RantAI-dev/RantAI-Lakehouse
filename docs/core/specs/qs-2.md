# `QS-2` Results you can use

| | |
| --- | --- |
| Backlog | `QS-2` in [BACKLOG.md](../BACKLOG.md) |
| Area | Query Studio |
| Who builds it | Query Studio, with Security (SEC-21) |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Results stop at 2,000 rows after computing everything, the grid only sorts, downloads are a 2,000-row browser CSV, and stop does not stop the server.

## What users get

See, filter and download full results; stop a query that runs too long.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Rows shown | 2,000, computed in full | Up to 64,000 rows or 10 MB, whichever comes first (Databricks); the limit is applied in the engine |
| Grid | Sort only | Sort, filter per column, column statistics (nulls, distinct, min/max, histogram), cell inspector for long and JSON values, copy cells |
| Download | Browser CSV of 2,000 rows; server route has no button | Server-side download of the full result as CSV, TSV, Excel (1,048,576-row sheet limit) and Parquet, up to 5 GB (Databricks), with masking re-applied (SEC-21) |
| Stop | Browser only | Cancels the query on the engine within 5 seconds *(proposed)* |
| Timeouts | Fixed 60 s | Admin-set default and maximum, from 10 seconds to 2 hours *(proposed)* |
| Progress | Not checked | Elapsed time and rows read while running |

## Benchmark

Databricks SQL editor: results up to 64,000 rows or 10 MB, filters and column profiling in the grid, downloads CSV/TSV/Excel up to about 5 GB; timeouts configurable; cancel. Snowflake: column statistics, cell inspector.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A 100,000-row result shows 64,000 rows and says it was cut
- [ ] Downloading it as Parquet gives all 100,000 rows
- [ ] Pressing stop on a long query frees the engine (visible in system.processes)
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Help inside the SQL editor is [`AI-12`](ai-12.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
