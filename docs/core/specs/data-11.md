# `DATA-11` Search that finds columns and tags

| | |
| --- | --- |
| Backlog | `DATA-11` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec corrected against the code 2026-10-09. Feature page [`catalog-search.md`](../features/catalog-search.md) drafted, decisions not signed. Plan written |

## Why

Catalog search looks for the typed text as one piece, in names, descriptions and tags. A column name finds nothing, one wrong letter finds nothing, and results come back in no useful order.

## What users get

Find a table by a column name, a description or a tag, from anywhere.

## Target specs

"Today" is `main` at `c338862`, re-read from the code on 2026-10-09, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| What is searched | Data Explorer: id, name, namespace, description, owner. The ⌘K box: name, id, description, tags. Column names and column descriptions: nowhere | Names, descriptions, column names, column descriptions, tag keys and values (Databricks' list). A tag is one word until `DATA-12` gives tags keys and values |
| Filters | Data Explorer filters on type, layer, tier, name, namespace, freshness and size. The API also accepts owner; the page does not offer it. No tag filter. Certification does not exist | Type, owner, layer (Bronze/Silver/Gold), tag, certification. The certification filter **Waits for `DATA-12`**, which creates the mark |
| Matching | The whole typed text as one substring, any letter case. No ranking | Tolerant: any word order, one-typo tolerance, ranked by relevance and use |
| Where | Data Explorer, and a ⌘K / Ctrl+K box on every signed-in page. By reading the code, the ⌘K box hides a table matched only by its description or tag (not run) | A search box reachable from every page (keyboard shortcut), showing every match and why it matched |
| Speed | Not measured. Every search rebuilds the whole catalog from six ClickHouse queries | Results within 500 ms for a catalog of 10,000 tables *(proposed)* |
| Permissions | Search needs `catalog:read` and passes the catalog's tenant check, which admits or refuses the whole catalog. The product has no per-table visibility: whoever may open the catalog sees every table in it | Only assets the user may see appear. This task keeps both checks on every search path; per-table visibility is not built here (feature page, decision 5) |

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
