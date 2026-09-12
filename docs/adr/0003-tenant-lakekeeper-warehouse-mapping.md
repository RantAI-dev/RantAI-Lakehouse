# ADR 0003 — Tenant → Lakekeeper project/warehouse mapping

- **Status:** Accepted
- **Phase:** P1
- **Date:** 2026-08-31

## Context

`lakehouse-api/src/tenant.rs` resolves `TENANT_ID` (default `"dispar-dki"`)
and friends from the environment — one deployment, one tenant, one image,
per its module doc comment ("Reading them from the environment lets one
image serve different deployments... without a rebuild"). Lakekeeper has
its own two-level structure above a table: a **project** contains one or
more **warehouses**, and every catalog operation (create namespace, create
table, load table) is scoped to exactly one warehouse.

`lakehouse-iceberg::catalog::IcebergClientConfig` needs a concrete
Lakekeeper `warehouse` string to connect. This ADR defines exactly how
`TENANT_ID` becomes that string, so the mapping is a documented convention
rather than an ad hoc choice buried in whichever call site constructs the
config first.

## Decision

**One Lakekeeper project per deployment, one warehouse per tenant, named
`TENANT_ID` verbatim.**

- **Project:** this deployment's single Lakekeeper project is named
  `lakehouse` (Lakekeeper's default/only project in a single-tenant-per-
  process deployment; P1b does not exercise Lakekeeper's multi-project
  support at all). A future genuinely multi-tenant Lakekeeper deployment
  (one Lakekeeper serving several independent `lakehouse-api` processes)
  would introduce a `LAKEKEEPER_PROJECT` env var at that point — out of
  scope today because nothing in this codebase runs more than one tenant
  per process (`tenant.rs`'s whole premise is one image, one tenant, per
  deployment).
- **Warehouse:** named exactly `TENANT_ID`'s value — `"dispar-dki"` in the
  Dispar production console, whatever a partner demo deployment overrides
  it to. No prefix, no suffix, no hashing. `TENANT_ID` is already
  validated at the `env_or` call site to be non-empty-after-trim (see
  `tenant.rs`), and Lakekeeper warehouse names accept the same character
  set `TENANT_ID` values already use in practice (lowercase, digits,
  hyphens) — no additional sanitization pass is introduced. If a future
  `TENANT_ID` needs characters Lakekeeper's warehouse-name grammar
  rejects, that surfaces as a clear warehouse-creation error rather than a
  silent rename, which is the correct failure mode: a warehouse name that
  doesn't match its `TENANT_ID` would be a worse debugging experience than
  a loud failure at setup time.
- **Rationale for verbatim, not derived:** every other tenant-labelled
  value in this codebase (`TENANT_OWNER`, `TENANT_DOMAIN`, audit records)
  already uses `TENANT_ID` as the join key. Deriving a different warehouse
  name (e.g. a hash, or a `wh-` prefix) would mean every operator-facing
  surface that needs to correlate "this tenant" with "this Lakekeeper
  warehouse" needs the mapping function, not just the string. Verbatim
  means `grep`-ability: an operator debugging a Lakekeeper warehouse can
  find the owning tenant by name with no lookup table.
- **`lakehouse_api::config::Config::lakekeeper_warehouse` is a separate
  field from `tenant::TENANT_ID`, not derived from it automatically.** The
  config field's doc comment says so explicitly: "NOT `TENANT_ID` itself."
  This is deliberate, not an oversight — see "What P1b does NOT do" below.

## Namespace scope within a warehouse

Bronze tables live in a single flat `bronze` namespace inside the tenant's
warehouse (ADR 0004) — namespace is not a second tenant-scoping dimension.
Tenant isolation is enforced once, at the warehouse boundary; a namespace
inside a warehouse is a data-organization concern (Bronze vs. a future
Silver/Gold-in-Iceberg namespace), not an isolation boundary. Two
namespaces inside the same warehouse are inherently the same tenant's data.

## What P1b does NOT do

- **Does not auto-derive `lakekeeper_warehouse` from `TENANT_ID` in code.**
  `Config` carries both as independent env-driven fields
  (`TENANT_ID` in `tenant.rs`, `LAKEKEEPER_WAREHOUSE` in `config.rs`) with
  the SAME default-naming convention documented here, but nothing computes
  one from the other at runtime. Reasons: (1) P1b wires no route to either
  crate yet (P6 does), so there is no call site today where the derivation
  would actually run; (2) writing the derivation now, untested against a
  real multi-tenant scenario, risks encoding the wrong assumption (e.g.
  ignoring the project-name axis this ADR reserves for later) into code
  that would then need a migration to fix. The convention is documented
  and enforced by operator discipline (set `LAKEKEEPER_WAREHOUSE` to match
  `TENANT_ID` in every deployment's environment) until P6 wires the two
  together in code, at which point this ADR's convention is exactly what
  that code should implement.
- **Does not create the warehouse itself.** Lakekeeper warehouse creation
  (`POST /management/v1/warehouse`, with its own storage-profile/
  storage-credential body) is an operational/bootstrap step, not something
  `lakehouse-iceberg` does at connect time — `IcebergClient::connect` only
  ever loads an *existing* warehouse's catalog config. Provisioning a new
  tenant's warehouse is deployment tooling (compose init job today,
  eventually a console action in P6), tracked separately.

## Consequences

- Every G1 test run targets the warehouse named by `LAKEKEEPER_WAREHOUSE`
  (default `"default"` in `lakehouse-api::config`, matching this
  deployment's docker-compose bootstrap, which provisions a single
  `default` warehouse rather than one named after the Dispar tenant — the
  compose stack is a generic dev environment, not the Dispar deployment
  itself).
- A production Dispar deployment sets `LAKEKEEPER_WAREHOUSE=dispar-dki`
  (matching `TENANT_ID`'s default) and provisions a Lakekeeper warehouse of
  that exact name before `lakehouse-iceberg` is ever pointed at it.
- Adding a second tenant to one Lakekeeper instance later needs only a new
  warehouse (`POST /management/v1/warehouse`) and that tenant's deployment
  setting `LAKEKEEPER_WAREHOUSE` to its own `TENANT_ID` — no schema change,
  no code change to this ADR's mapping.

## Verification

No runtime code enforces this convention yet (see "What P1b does NOT do"),
so there is nothing to unit-test beyond the existing `Config` field tests
in `lakehouse-api/src/config.rs` (`lakekeeper_and_rustfs_fields_are_overridable`).
The G1 test itself is the practical verification that a warehouse
identifier, once set to match convention, round-trips through
`IcebergClientConfig` correctly.
