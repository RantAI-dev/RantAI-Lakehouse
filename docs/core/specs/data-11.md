# `DATA-11` Search that finds columns and tags

| | |
| --- | --- |
| Backlog | `DATA-11` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Catalog search matches table names by substring only; a column name or tag finds nothing.

## What users get

Find a table by a column name, a description or a tag, from anywhere.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| What is searched | Table names | Names, descriptions, column names, column descriptions, tag keys and values (Databricks' list) |
| Filters | None | Type, owner, layer (Bronze/Silver/Gold), tag, certification |
| Matching | Substring | Tolerant: any word order, one-typo tolerance, ranked by relevance and use |
| Where | Data Explorer only | A search box reachable from every page (keyboard shortcut) |
| Speed | Not measured | Results within 500 ms for a catalog of 10,000 tables *(proposed)* |
| Permissions | n/a | Only assets the user may see appear |

## Benchmark

Databricks search: names, comments, column names and comments, tag keys; filters by type, owner, catalog, schema, tag, certification; syntax filters. Snowflake Universal Search: fuzzy, role-filtered.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] Searching a column name finds its table
- [ ] Searching 'revnue' finds 'revenue'
- [ ] A user without access to a table does not see it in results
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Plain-language search (AI team, AI-16)

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
