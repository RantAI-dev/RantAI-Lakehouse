# `SEC-16` No cross-tenant leaks in the Data module: plan (part A)

Planner: Claude Opus. Developer: a different agent. Base: `origin/main` at
`cd8e3df`. Branch: `fix/sec-16-cross-tenant`. Spec:
`docs/core/specs/sec-16.md`. Feature page:
`docs/core/features/tenant-isolation-data-module.md` (decisions D1–D6; not
yet signed, the defaults are built).

Part B of the spec (a Bronze namespace per tenant) is not in this plan; the
feature page says why.

## 1. What is wrong, with anchors (verified at `cd8e3df`)

- **Ingestible list.** `rust/crates/lakehouse-api/src/routes/connectors.rs:201`
  (`list_ingestible`) takes no `Principal` and returns
  `connectors::list_ingestible_connectors(pool)` whole. The route needs
  `ingest:read` (`POLICY_TABLE`). Its intended caller is the service
  identity seeded by `bootstrap_ingest_run_service`
  (`lakehouse-api/src/main.rs:514`), used by
  `dagster/dispar_orchestrate/ingest_factory.py`,
  `replication_metrics.py:229` and `ops/debezium/render_compose.py`; all
  three need every tenant's rows. `IngestibleConnector`
  (`lakehouse-store/src/connectors.rs:1403`) carries no tenant.
- **Refusal that says why.** `routes/uploads.rs:1075` (`ensure_table_free`)
  answers `CONNECTOR_TABLE` when `connectors::any_connector_targets(pool,
  table)` is true for a connector of any tenant, and `TABLE_NOT_FREE`
  ("…no upload of this tenant created it…") for another tenant's claim or an
  unclaimed existing table. `ingest` (`:1259`) answers `TABLE_NOT_FREE`
  again when the claim is lost (`:1288`).
- **Annotation read.** `routes/catalog.rs:2374` (`get_annotation`) takes no
  `Principal` and calls no gate. `put_annotation` (`:2421`) calls
  `catalog_tenant_refusal` (`:164`) since the review of pull request 79.
  Neither looks the asset up.

Reuse: `crate::tenant_scope::resolve` (`tenant_scope.rs:51`),
`is_unrestricted`, the principal's kind (user or service; see
`kind_for_audit` in `lakehouse-auth/src/principal.rs`), `catalog_tenant_refusal`,
and whatever `routes::catalog::detail` uses to decide that an asset id
exists (grep before writing a second lookup; rule 4).

## 2. Decisions already made

- E1. `list_ingestible` takes the caller. A service principal, or an
  unrestricted one, gets every row as today. Any other caller gets the rows
  of the tenant `tenant_scope::resolve` gives, through a store function that
  filters in SQL (a bound parameter); `None` gives an empty list. A missing
  principal is 401. The `dueAfter`/`dueUntil` window applies after, as now.
- E2. The service check uses the auth layer's own notion of a service
  principal. If that turns out not to be available on `Principal` at the
  handler, stop and report; do not compare display names.
- E3. `ensure_table_free`: `CONNECTOR_TABLE` only when a connector **of the
  caller's tenant** targets the name. Every other refusal (another tenant's
  connector, another tenant's claim, an unclaimed existing table, a lost
  claim in `ingest`) is one constant: `That table name cannot be used.
  Choose another name.` `TABLE_NOT_FREE` keeps its name and takes that
  text. This needs a tenant-aware form of `any_connector_targets`; keep the
  existing function if other callers use it.
- E4. `get_annotation` takes the caller and the headers and calls
  `catalog_tenant_refusal`; a refusal is `ApiError::PermissionDenied` with
  the gate's reason, as in `put_annotation`.
- E5. Both annotation handlers answer 404 `Asset not found.` when the id is
  not an asset of the catalog, after the gate and the id validation and
  before any read or write of the annotation table. If the catalog cannot
  be asked, 503 with a fixed sentence (fail closed). The check must not
  add a second, different definition of "this asset exists".
- E6. No migration. No console change is expected; if the console shows
  `TABLE_NOT_FREE`'s old text anywhere (a test, a mapped message), update
  it in T3's commit.

## 3. Tasks

One task per commit, in this order.

**T1. Ingestible list.** Store function plus handler per E1–E2. Update the
handler's doc comment and the `POLICY_TABLE` comment for the route to say
who sees what.
*Check:* `sqlx::test` for the store function (two tenants, an unassigned
connector: only the asked tenant's rows). Route tests: a user of tenant A
with `ingest:read` sees A's connector and not B's (this is the regression
test; say in its doc comment that it fails before the fix); a tenant-less
user with `ingest:read` gets `[]`; the service identity gets both; an
`X-Tenant` naming a tenant the caller is not in is 404. The existing
`ingest_read_scope_can_call_ingestible_but_not_the_base_connectors_route`
in `tests/route_auth.rs` must still pass unchanged in meaning.

**T2. Annotation read and existence.** E4 and E5.
*Check:* route tests: read refused with 403 for a caller outside the
catalog's tenant in a two-tenant installation; read and write 404 for an
id that is not in the catalog (and nothing is written: the annotation
table has no row afterwards); read and write still succeed for an asset
that exists; the existing annotation tests updated only where they used an
id that does not exist, each such change named in the commit body.

**T3. One sentence for a taken name.** E3.
*Check:* route tests in `tests/upload_routes.rs`: another tenant's
connector targeting the name gives the one sentence and not
`CONNECTOR_TABLE` (regression test); the caller's own tenant's connector
still gives `CONNECTOR_TABLE`; another tenant's claim gives the one
sentence. The existing test that lists the fixed sentences
(`routes/uploads.rs`, near `:1987`) is kept in step.

**T4. Docs.** `CHANGELOG.md`; `docs/core/features/upload-file.md` where it
quotes the old sentence; the spec's "Today" column is not edited here (the
planner corrects the spec).

## 4. Pull request

One slice.

## 5. Out of scope

Part B. `routes/lineage.rs` and `routes/pipelines.rs` (another team's
files). The Dagster callers (they keep working because of E1; do not
change them). Any migration or new dependency.

## 6. Verification

Per commit: the scoped checks of `AGENTS.md`. Before handoff: the block in
`AGENTS.md` for the languages touched.

**On this machine, do not build Rust test binaries** (`cargo test`, `cargo
build`, `cargo check --tests` have crashed it). Run `cargo fmt --check` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` with
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
CARGO_BUILD_JOBS=2`, and write every Rust test as *not verified* in the
handoff. CI on the pull request runs them.

## 7. Handoff

(The developer writes here.)

## 8. Review

(The planner writes here.)
