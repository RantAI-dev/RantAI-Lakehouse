# Usage and cost view

| | |
| --- | --- |
| Module | Operations |
| Backlog | `OPS-8` |
| Kind | Feature |
| Spec | `docs/core/specs/ops-8.md` |
| Status | Building |
| Priority | P2 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | Engineering plan kept by the planner |
| Started | 2026-11-16 |

## Problem

Nobody can see who uses the lakehouse or how much it costs to run. The data for queries is recorded but never added up, and it carries no tenant.

## What the user can do when this is done

1. Open Monitoring → Usage and pick a period.
2. See queries, scanned bytes and engine time per day, per person and per engine, for their own tenant.
3. See storage per layer, one value per day.
4. Enter rates and see an estimated amount, marked as an estimate.
5. Export the period as CSV.

## Asking the assistant

No assistant work in this feature. `AGENTS.md` assistant parity binds BI features only.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | "Cost" means usage in physical units. Money appears only from admin-set rates and is labelled an estimate | as written | Owner, 2026-11-15 |
| 2 | Scope of the first version: queries by day, person, engine; storage by layer. Assistant tokens are a separate item | as written | Owner, 2026-11-15 |
| 3 | Who sees it: a new `usage:read` for the tenant admin role and `*:*`; a person without it does not see the page (a personal "my usage" view is out of this feature) | as written | Owner, 2026-11-15 |
| 4 | Targets: 365 days kept, 5-minute freshness, p95 under 2 s for 90 days, CSV export | as written | Owner, 2026-11-16 |
| 5 | No assistant tool in this feature | as written | Owner, 2026-11-15 |
| 6 | A tenant administrator sees only their tenant; `*:*` sees all | as written | Owner, 2026-11-16 |
| 7 | Rows written before the tenant column exist show only to `*:*`, as "no tenant", never guessed (like `0099_saved_query_tenant.sql`) | as written | Owner, 2026-11-16 |
| 8 | Delivery in two parts: A records the tenant on every `query_history` row; B adds the route, the page, rates and export | as written | Owner, 2026-11-16 |
| 9 | Storage source: Iceberg snapshot summaries (`total-files-size`) or the object store, chosen by the plan's scout after measuring both; never a constant | as written | Owner, 2026-11-16 |

## Limits to tell a customer

- Amounts are estimates from rates the administrator enters, not a bill.
- Queries run before the update have no tenant and are visible only to the platform operator.
- Queries run through dashboard and chart data are not counted.
- Queries run through the lists of filter values, and through schema and join lookups, are not counted.
- Queries run through the preview of a SQL dashboard source are not counted.
- Queries run through the assistant's dashboard chart tool are not counted.
- Queries run through a public embed are not counted.
- Queries run to deliver a scheduled report are not counted.
- Queries run to evaluate an alert rule are not counted.
- Queries run through the preview of a catalog table are not counted.
- Queries run through the preview of a pipeline are not counted.
- Queries run through the download of a past query's result, and through cost estimates (EXPLAIN), are not counted.
- Queries run through data quality checks are not counted.
- Queries run through gold export jobs are not counted.
- Queries run through the preview of a policy are not counted.
- Queries run through lookups of parameter, map, hierarchy and calculated-field values are not counted.
- Assistant token usage is not shown.
- No budget or alert.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `OPS-8-AC1` to `AC6` | As in the spec | As in the spec | Not run |
