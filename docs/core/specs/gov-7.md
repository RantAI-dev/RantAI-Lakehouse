# `GOV-7` Read access per schema and table, enforced everywhere data is read

| | |
| --- | --- |
| Backlog | `GOV-7` in [BACKLOG.md](../BACKLOG.md) |
| Module | Governance |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code at `dcaae8a5` on 2026-11-09. Feature page [`table-access-per-role.md`](../features/table-access-per-role.md) |

## Why

`query:read` reads every table (`routes/catalog_grants.rs:12-14`). Masks and row filters per role exist (`policy_engine.rs`, `sql_rewrite.rs`), but nothing can hide a table from a role. A company cannot give Finance only the `finance` schema.

## What users get

An administrator opens a role and chooses "All tables" (the default) or "Only these", then picks schemas and tables. People holding only narrowed roles can read only those tables, everywhere: Query Studio, dashboards, embeds and public links, alerts, reports, authored pipelines, Gold export, the assistant, and the catalog and its search. A table they may not read does not appear anywhere. A query naming it gets the same "not found" as a table that does not exist.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Granularity | None (all tables) | Whole schema, or single tables |
| Default for existing and new roles | All tables | All tables, so an upgrade changes nobody's access (owner, 2026-11-09) |
| Several roles | n/a | Union: any role on "All tables" means all tables; otherwise the union of the listed ones |
| A role holding `*:*` | n/a | Cannot be narrowed, so an admin cannot lock everyone out |
| A refused query | n/a | 404 with the existing "Table not found or not available to your organisation" text; it does not reveal whether the table exists |
| Catalog list, search, detail, lakehouse table browser, dashboard field pickers, the assistant's data map | Every table | Only readable tables; hidden detail pages answer 404 |
| Embeds and public links | Masks of the fixed `Dashboard Viewer` role | Also that role's table access |
| Alerts, reports, chart alerts | Owner's masks | Also the owner's table access |
| Authored pipelines | Author's masks | Also the author's table access |
| Service identities, Dagster's own builds, quality checks | Not role-based | Unchanged (governed by scopes or run as the system); stated as a limit |
| Time for the access check per query | n/a | One Postgres read per statement; budget under 5 ms at p95 on the dev stack *(proposed)* |

## Benchmark

| Product | Table-level read access |
| --- | --- |
| Metabase | Data permissions per database, schema and table ("Can view" / "Blocked"); blocked tables are hidden in the query builder and data browser |
| Superset | `datasource_access` per dataset and `schema_access` per schema; SQL Lab refuses tables outside them |
| Looker | Access grants on explores and fields, model sets per role |

Target: match Metabase and Superset (schema and table granularity, hidden in browsing, refused in SQL) and go further by covering the assistant, alerts and embeds with the same rule.

## Acceptance checklist

The full list is on the feature page, [`table-access-per-role.md`](../features/table-access-per-role.md).

## Not included

- Column-level hiding (masks already cover columns).
- A temporary grant (`DATA-15`) adding a table: a grant gives a permission, not a table. To give a narrowed person a table, add it to one of their roles.
- Access requests for hidden tables (they are hidden, so nobody can request them).
- Changing what service identities or the Iceberg door can read.

## Asking the assistant

The assistant reads only the tables the person may read: its data map, its SQL and its metric queries all go through the same rule. Setting a role's table access through the assistant is not included (Governance, not BI; `AI-15` does not apply).
