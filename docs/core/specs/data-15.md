# `DATA-15` Asset page depth

| | |
| --- | --- |
| Backlog | `DATA-15` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

The asset page previews 25 to 100 rows, shows 7 days of counts, and cannot grant access or restore a table.

## What users get

The asset page matches Databricks' table page.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Data preview | 25–100 rows | Up to 1,000 rows, with masking applied |
| Usage | 7 days of counts | 30 days: queries, frequent users, frequent queries, tables joined with it, column popularity |
| Access | Not available | Grant and revoke from the asset, for users with the right to manage access |
| Restore | Not available | Restore a dropped table within the retention window *(proposed: 7 days)* |
| History | Not checked | Table versions with time, operation and author |

## Benchmark

Databricks Catalog Explorer: sample data about 1,000 rows, Insights over 30 days (frequent users, queries, joined tables), Permissions tab, history, UNDROP.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] Preview shows 1,000 rows with masked columns masked
- [ ] Usage covers 30 days
- [ ] A dropped table is restored from its page within 7 days
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Entity relationship diagram *(proposed drop)*

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
