# ADR 0018: several organisations in one install

- Status: **Accepted** by the product owner, 2026-11-16, as proposed. Slice (a), the
  prototype tests, is deferred while `ADM-1` stays P3 (owner, same day).
- Date: 2026-11-16
- Backlog: `ADM-1` (P3). The owner decided on 2026-11-15 that research and this ADR come first,
  with no code, and that the build is blocked by `SEC-20`.
- Builds on: ADR 0003 (one warehouse per tenant), ADR 0014 decision 6 (uploads land in the
  shared catalog), and the `SEC-20` decisions in `docs/core/features/queries-scoped-to-the-tenant.md`.

## Context

Tenants exist: the `tenant` table, membership in `app_user_tenant`, the active tenant in the
`X-Tenant` header (`tenant_scope.rs:30`), and `tenant_id` on connectors, pipelines, uploads,
saved queries, folders and public links. But the Bronze, Silver and Gold data sit in one
Lakekeeper warehouse (`LAKEKEEPER_WAREHOUSE`, default `default`), which belongs to one tenant,
named by `CATALOG_TENANT_ID`. Every other tenant is refused by the catalog gate
(`routes/catalog.rs:211`, rule at `:235-244`). So one install serves one organisation's data.

Part of the separation is already built and not used:

- ADR 0003 (2026-08-31) decided one Lakekeeper warehouse per tenant inside one project.
- `provision_tenant` (`routes/identity.rs:1718`) creates a warehouse `tenant-<slug>` for each
  new tenant, on its own prefix of `TENANT_WAREHOUSE_S3_BUCKET`, and migration `0042` stores
  `warehouse_id` and `provisioning_status`.
- Provisioning stops at `grants_ready`. No namespace is created, because
  `ensure_bronze_namespace` and `ensure_gold_namespace` are tied to the shared catalog
  (`identity.rs:1757-1776`). Nothing reads, writes or queries these warehouses.

What still assumes one catalog: `CATALOG_TENANT_ID` in 41 files (gate sites, alerts, assistant
tools, Gold export, compose, ops gates, CI); `LAKEKEEPER_WAREHOUSE` in Dagster (4 files) and in
the Trino catalog file that compose writes; the Trino catalog name `iceberg`, written into SQL
(`lineage.rs:310`, `routes/ai/prompt.rs:912,983`); the ClickHouse databases `serving`, `silver`
and `lake` (`tenant.rs:110`, `sql_guard.rs:454`, `lakehouse-alerts/src/lib.rs:1041`); and one
Dagster code location whose authored pipelines run as a service identity with no tenant.

Research notes (untracked, planner): `plans/adm-1-research-repo.md` and
`plans/adm-1-research-external.md`.

How others draw the line (from their documentation, see the external notes): Snowflake
isolates by account inside an organisation. Databricks shares one Unity Catalog metastore per
region and isolates by catalog and workspace. Fabric isolates by workspace on shared capacity.
Metabase separates tenants by row rules on one database, or routes each tenant to its own
database.

## Decision

1. **An organisation is a tenant.** No new entity. The words "organisation" in the console and
   "tenant" in the code mean the same row.
2. **One Lakekeeper warehouse per organisation, in the one project**, as ADR 0003 says. The
   warehouse is the isolation boundary. Each warehouse keeps its own storage prefix; per the
   Lakekeeper docs, locations are never shared between warehouses, because vended credentials
   would leak data across them. The API, Dagster and Trino select the warehouse by name through
   the Iceberg REST `warehouse` parameter, which works on the pinned v0.13.3 without a project
   header. One Lakekeeper project per organisation is **not** chosen. It needs the
   `x-project-id` header, which is documented only for later releases (v0.14 and v0.15), and
   Trino's support for sending it is not confirmed.
3. **The current catalog tenant keeps the `default` warehouse.** `CATALOG_TENANT_ID` stops
   meaning "the only tenant with data". It names the tenant that owns the warehouse the install
   started with. A one-tenant install changes nothing.
4. **Trino: one Iceberg catalog per organisation**, named from the tenant id, not the slug,
   because a slug can be renamed. A query runs in its organisation's catalog. The API rewrites
   nothing across catalogs: a reference to another organisation's catalog is refused with the
   `SEC-20` 404 sentence. Adding a catalog at run time needs `catalog.management=dynamic`; if
   the Iceberg REST connector does not support it on Trino 483, a new organisation needs a
   Trino restart, and that limit is written down.
5. **ClickHouse: one serving database per organisation** on the one server, with a ClickHouse
   role per organisation that can read only its database, and a quota per organisation.
   ClickHouse quotas count queries and rows, not CPU or memory, so one organisation can still
   slow another down. That limit is told to customers; a separate server per organisation is a
   later option.
6. **Dagster: one code location, the tenant on every run.** Each authored pipeline's runs carry
   its tenant as a tag and use that tenant's warehouse and serving database. A concurrency limit
   per tenant tag keeps one organisation from taking all run slots. One code location per
   organisation is not chosen: adding one at run time is not confirmed in open-source Dagster.
7. **No sharing between organisations.** One organisation never reads another's tables. Sharing
   data between organisations is a separate feature (PRODUCT.md, "Share data with another
   organisation", Missing).
8. **Only a full administrator (`*:*`) creates or removes an organisation**, as tenants are
   created today. A full administrator sees every organisation; everyone else sees only their
   own.

## Consequences

- The `SEC-20` gate changes meaning. Today a tenant that is not the catalog tenant is refused
  the shared catalog. After this, each tenant is let into its own warehouse and refused the
  others. The 404 and 422 sentences in the `SEC-20` decisions stay, but their trigger moves.
  That is why the build waits for `SEC-20` to merge.
  The `SEC-20` work (plans 94 to 96, #538, #547, #548) keeps the rule in four helpers:
  `resolve_reach`, `claim_on`, `table_is_reachable` and `catalog_tenant_refusal`. The rule
  changes there and nowhere else; the ADM-1 plan anchors them (AI Chat planner, 2026-11-16).
  Audit rows owned by the deployment (Dagster, services) follow the shared-catalog rule only
  through `catalog_tenant_refusal`, so they change with it.
- Uploads (ADR 0014 decision 6) move from the shared `bronze` namespace to the uploading
  tenant's warehouse. Existing uploaded tables need a move or stay where they are; the plan
  decides with the owner.
- Gold export, alerts, the assistant's tools and lineage all name the organisation's catalog
  and serving database instead of a constant. The 41 `CATALOG_TENANT_ID` sites are the work
  list.
- One organisation can slow another down on ClickHouse and Trino. Quotas limit counts, not CPU
  or memory.

## Not confirmed, to test on the pinned versions before the plan

- Lakekeeper v0.13.3: several warehouses in one project, each on its own prefix, read and
  written by name with no project header.
- Trino 483: an Iceberg REST catalog added with `CREATE CATALOG` under
  `catalog.management=dynamic`, and whether `SHOW CATALOGS` hides a catalog the user may not use.
- ClickHouse 26.8: a role limited to one database, and memory or thread limits in a settings
  profile per organisation.
- Dagster 1.13.20: `tag_concurrency_limits` with a limit per unique tenant value.

## Size and order

Size L (several weeks). Suggested slices, each its own plan: (a) the prototype tests above;
(b) provisioning finishes (namespaces, Trino catalog, ClickHouse database and role);
(c) query, catalog and upload paths use the tenant's warehouse; (d) Dagster pipelines, Gold
export and alerts; (e) the assistant's tools and lineage. Slice (a) can run before `SEC-20`
merges, because it changes no product code.
