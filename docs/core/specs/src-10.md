# `SRC-10` More load modes

| | |
| --- | --- |
| Backlog | `SRC-10` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Sources) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Loads either append or replace; there is no way to keep one current row per key, or a history of changes.

## What users get

Loads that keep one current row per key, or keep a full history.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Sync modes | Append and replace | All five Airbyte modes, chosen per table |
| Deduplicate on primary key (SCD type 1) | None | Merge on a declared or detected primary key with a cursor column |
| History (SCD type 2) | None | Valid-from, valid-to and current-flag columns, as Databricks |
| Refresh one table, or clear it | None | Per table: refresh keeping or removing old rows, and clear (Airbyte refresh/clear) |

## Benchmark

Airbyte: full refresh overwrite, full refresh append, full refresh overwrite + dedup, incremental append, incremental append + dedup. Databricks Lakeflow: SCD type 1 and 2.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] An update at the source produces one current row in dedup mode and a closed plus a new row in SCD 2 mode
- [ ] Refreshing one table does not touch the connector's other tables
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
