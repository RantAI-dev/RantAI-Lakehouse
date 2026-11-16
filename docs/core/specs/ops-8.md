# `OPS-8` Usage and cost view

| | |
| --- | --- |
| Backlog | `OPS-8` in [BACKLOG.md](../BACKLOG.md) |
| Module | Operations |
| Size | M, in two parts (planner's estimate) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md) |
| Spec checked | Against the code at `6da7012a1` on 2026-11-16. Feature page [`usage-and-cost-view.md`](../features/usage-and-cost-view.md) |

## Why

An administrator cannot see who uses the lakehouse or how much. `query_history` (migration `0005`) already stores one row per query run: owner, time, status, `duration_ms`, `scanned_bytes`, `engine`, `workload_class` and `cost_units`. `cost_units` is scanned bytes divided by 1,000,000 (`routes/query.rs` `bytes_to_cost_units`). Nothing adds these rows up. `query_history` has no tenant column, so a sum over it would show one tenant's usage to another. An earlier `usage` kind of `/api/ops` was removed (WS1, `routes/ops.rs` module doc) because its storage figure came from an invented bytes-per-row constant. PRODUCT §2 lists the view as Missing; Snowflake and Databricks both have one.

## What users get

A tenant administrator opens Monitoring → Usage and sees, for a chosen period, how many queries ran, how many bytes they scanned and how much engine time they took, by day, by person and by engine, plus how much storage each layer holds. When the administrator has entered rates, the page also shows an estimated amount in money, labelled as an estimate. Everything can be exported as CSV.

## Target specs

Best competitor per row from `plans/ops-8-research.md`.

| Capability | Today | Best competitor | Target |
| --- | --- | --- | --- |
| Query usage by day, person, engine | None | Snowflake `QUERY_HISTORY`: user, role, warehouse, database | Day, person, engine, workload class |
| Freshness | None | Databricks query history about 1 hour; Snowflake up to 45 minutes | A finished query counts within 5 minutes |
| History kept | `query_history` is never trimmed | 365 days (Snowflake, Databricks query history) | At least 365 days; no trimming in this feature |
| Storage by layer | None (the old figure was invented) | Snowflake daily average per database | Bytes per layer (Bronze, Silver, Gold, Serving) from a measured source, one value per day |
| Money | None | Databricks: usage times admin-set rate, marked as estimate | Admin-set rate per TB scanned, per engine hour and per GB-month stored, with an effective date; shown only when set, labelled "estimate" |
| Tenant separation | Not possible, no tenant column | One account per customer | A tenant administrator sees only their tenant; `*:*` sees all tenants and the rows with no tenant |
| Who can see | None | Admin roles (`USAGE_VIEWER`, account admin) | A new permission `usage:read`, given to the tenant admin role |
| Export | None | Databricks CSV | CSV of the aggregated rows for the chosen period |
| Page speed | None | Not published | p95 under 2 seconds for a 90-day period on staging |
| Budgets and alerts | None | Databricks and Snowflake: threshold alerts, notify only | Not in this feature |

## Acceptance checklist

- `OPS-8-AC1` Run three queries as one person; within 5 minutes Monitoring → Usage shows them under that person, engine and day
- `OPS-8-AC2` Signed in as an administrator of tenant B, the page shows none of tenant A's people or queries
- `OPS-8-AC3` With no rates set, the page shows no money column; after setting a rate per TB, an amount appears marked "estimate"
- `OPS-8-AC4` Storage by layer shows a value per layer that matches the object store within 1 %
- `OPS-8-AC5` Export a 30-day period as CSV; the totals match the page
- `OPS-8-AC6` A person without `usage:read` gets 403 on the route and does not see the menu item

## Not included

Assistant token usage (not recorded today; a separate item), budgets and alerts, enforcing a limit, query text on the page, trimming `query_history`, an assistant tool.
