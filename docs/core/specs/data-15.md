# `DATA-15` Asset page depth

| | |
| --- | --- |
| Backlog | `DATA-15` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code on 2026-10-09 |

## Why

The asset page previews 25 to 100 rows, shows 7 days of counts, and cannot grant or revoke access directly or restore a table.

## What users get

The asset page matches Databricks' table page.

## Target specs

"Today" is `main` at `c338862`, re-read from the code on 2026-10-09, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Data preview | 25, 50 or 100 rows. Masking and row filters are already applied; a preview that cannot be masked shows no rows | Up to 1,000 rows, with masking applied |
| Usage | 7 days: number of queries, number of people, average time, and the viewer's own last 5 queries. Dashboard reads are not counted | 30 days: queries, frequent users, frequent queries, tables joined with it, column popularity |
| Access | An Access tab shows the viewer's own access, sets classification, and adds masking and row-filter policies. A user can request access; approval is in the approvals inbox and gives a 30-day permission that is not limited to this table. No direct grant, and nothing revokes a grant | Grant and revoke from the asset, for users with the right to manage access |
| Restore | Not available | Restore a dropped table within the retention window *(proposed: 7 days)* |
| History | Iceberg tables list their versions with time, operation and row counts, each with a link to query it. No author. ClickHouse tables keep no versions | Table versions with time, operation and author |

## Benchmark

Databricks Catalog Explorer: sample data about 1,000 rows, Insights over 30 days (frequent users, queries, joined tables), Permissions tab, history, UNDROP.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-15-AC1` Preview shows 1,000 rows with masked columns masked
- `DATA-15-AC2` Usage covers 30 days
- `DATA-15-AC3` A dropped table is restored from its page within 7 days
- `DATA-15-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Entity relationship diagram *(proposed drop)*

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
