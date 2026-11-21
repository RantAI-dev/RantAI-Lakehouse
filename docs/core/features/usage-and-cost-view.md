# Usage and cost view

| | |
| --- | --- |
| Module | Operations |
| Backlog | `OPS-8` |
| Kind | Feature |
| Spec | `docs/core/specs/ops-8.md` |
| Status | Released (accepted 2026-11-19; PR #591) |
| Priority | P2 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | Engineering plan kept by the planner |
| Started | 2026-11-16 |
| Shipped | 2026-11-17 |
| Evidence | PR #553 (the tenant on every query run); PR #564 (the usage page, the rates and the CSV export); PR #574 (the measured size of every layer); PR #591 (the person's name, the two storage numbers and the rest of the bucket) |

## Problem

Nobody can see who uses the lakehouse or how much it costs to run. The data for queries is recorded but never added up, and it carries no tenant.

## What the user can do when this is done

1. Open Monitoring → Usage and pick a period.
2. See queries, scanned bytes and engine time per day, per person and per engine, for their own tenant.
3. See storage per layer: the current data and what is stored on disk for each Iceberg layer, one number for the ClickHouse layers, and the bytes in the bucket that no current table holds, measured once a day.
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
- Engine time is the wall-clock time of the query, not CPU time.
- Rates are one set for the whole deployment, in one currency.
- A rate takes effect from the date entered and never changes earlier days.
- A stopped query counts as cancelled.
- A tenant administrator sees only their tenant. The all-tenant administrator can also group by tenant and sees runs recorded with no tenant.
- Storage is shown to the platform operator only.
- The ClickHouse layers (`silver`, `serving`) show one number: their bytes are on the ClickHouse server's own disk, not in the warehouse bucket.
- "Stored on disk" counts a table's history and its orphan files: it is what the object store holds under the table's location, not only the live data.
- The page is empty until the first daily run (the storage job runs once a day at 01:00 UTC). It says "Not measured yet"; that is not an error.
- A deployment whose daily job already ran shows the layers with their current data and "Stored on disk: not measured yet. The daily job runs at 01:00 UTC." instead of an error, until the next run appends the disk column; no line for the rest of the bucket appears in that state.
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
| `OPS-8-AC1` to `AC3` | As in the spec | As in the spec | Pass 2026-11-19 |
| `OPS-8-AC4` | Trigger `storage_snapshot_job` once and take the bucket's size from a `capacity_snapshot` run within the same hour; compare the two on the Usage page | The sum of "stored on disk" over the Iceberg layers plus "In the bucket but in no current table" equals the bucket within 1 %, when both are measured within the same hour | Pass 2026-11-19 |
| `OPS-8-AC5` to `AC6` | As in the spec | As in the spec | Pass 2026-11-19 |

Storage per layer shipped in PR #574, so `OPS-8-AC4` can now run.
