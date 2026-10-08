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

Developer: Claude Sonnet 5.5. Branch `fix/sec-16-cross-tenant`, four commits
on top of `eae369a` (T1 `ddfc5e5`, then T2, T3, T4; see `git log`).

**Commits.**

- T1 `fix(api): the ingestible list shows a caller only its own tenant's
  connectors`. `lakehouse-store/src/connectors.rs`:
  `list_ingestible_connectors_for_tenant` (SQL `($1::uuid IS NULL OR
  tenant_id = $1)`, bound) over a private `list_ingestible`; the unscoped
  `list_ingestible_connectors` keeps its signature (lineage and others use
  it). The same commit also adds `any_connector_of_tenant_targets`, which T3
  uses (the store changes sit together). Handler in `routes/connectors.rs`,
  comment in `policy.rs`.
- T2 `fix(api): annotations pass the tenant gate on read and need an
  existing asset`. `routes/catalog.rs`: `get_annotation` takes
  `Extension<Principal>` and `HeaderMap`; new `ensure_asset_exists`.
- T3 `fix(api): a taken table name gets one sentence unless the caller's own
  connector took it`. `routes/uploads.rs`, tests, and the console test stub.
- T4 docs: `CHANGELOG.md` (Security), `docs/core/features/upload-file.md`
  (the rule, and QA step 15a).

**Commands run (all foreground, `CARGO_TARGET_DIR=/home/hv/.cache/
lakehouse-catalog-target CARGO_BUILD_JOBS=2`).**

- `cargo fmt --check`: clean on the final tree (after `cargo fmt`, where
  every hunk was mine).
- `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets
  --all-features -- -D warnings`: finished with no warning, after T1, T2
  and T3. The full-workspace clippy line was not run (only these two crates
  changed; `lakehouse-auth` was checked as a dependency).
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (none in
  files I touched). `bun run test`: 865 pass, 1 skip, 0 fail (866 tests,
  99 files).
- Python lints: not run, no Python touched.

**Not verified (the reason is the instruction not to build test binaries on
this machine).** Every Rust test, old and new: `cargo test` was never run.
New or changed Rust tests, all awaiting CI:

- `lakehouse-store/tests/connectors.rs`:
  `list_ingestible_connectors_for_tenant_returns_only_that_tenants_rows`,
  `any_connector_of_tenant_targets_counts_only_that_tenants_connectors`.
- `lakehouse-api/tests/connector_ingestible_tenant.rs` (new, five tests).
- `lakehouse-api/src/routes/catalog.rs`, `tenant_scoping` module:
  `get_annotation_refuses_with_403_when_the_caller_is_not_a_member`,
  `get_annotation_answers_the_empty_shape_for_an_existing_asset_without_one`,
  `get_annotation_answers_404_for_an_id_that_is_not_in_the_catalog`,
  `put_annotation_answers_404_and_writes_nothing_for_an_id_that_is_not_in_the_catalog`,
  `annotation_routes_answer_503_when_the_catalog_cannot_be_asked`; changed:
  `put_annotation_still_writes_when_the_caller_is_a_member_of_the_catalog_tenant`
  (it used a dead ClickHouse address and now mocks a catalog that holds
  `serving.mart_x`).
- `lakehouse-api/tests/route_auth.rs`:
  `a_seeded_analyst_is_not_denied_catalog_annotation_read` now accepts a 403
  only with the tenant gate's fixed reason (as the Data Engineer write test
  does), because the seeded app has several tenants and no
  `CATALOG_TENANT_ID`, so the read now answers the gate's 403 (changed
  behaviour by design, E4).
- `lakehouse-api/tests/upload_routes.rs`:
  `another_tenants_connector_closes_the_table_with_the_one_sentence` (new);
  `NOT_FREE` constant and the pin in `routes/uploads.rs`
  (`the_messages_the_plan_words_are_the_plans_words`) take the new sentence.

**Tests most likely to fail in CI, and why.**

1. `a_seeded_analyst_is_not_denied_catalog_annotation_read` and any other
   test I did not find that reads an annotation through the router with a
   seeded multi-tenant app: the read is now gated. I grepped `tests/` and
   `src/` for `annotation` and found only `route_auth.rs`.
2. The `tenant_scoping` annotation tests: they rely on the wiremock answering
   every `POST` with `{meta, data, rows}` and on `ChClient::rows` parsing it
   as it does for the existing `mock_clickhouse`. The "exists" mock answers
   every query with one `(name, type)` row; the existence check for a
   `serving.` id only needs the `system.columns` query to return a row.
   The 503 test uses `http://127.0.0.1:0`, the address the pre-existing
   tests use for "ClickHouse not reachable".
3. `connector_ingestible_tenant.rs`: assumes Bayu is in Meridian Group only
   as his first tenant (`X-Tenant` absent resolves to his first), that Data
   Engineer holds `ingest:read`, and that Fajar's role is `*:*`; it inserts
   `connector` rows with the same column list `connector_upload_table.rs`
   uses. The service-identity test uses the same `create_service_identity` /
   `ensure_service_credential` calls as `gold_export_auth.rs`.
4. `another_tenants_connector_closes_the_table_with_the_one_sentence`: the
   insert adds `source_objects` to the column list of the other tests' insert;
   it assumes the upload belongs to Bayu's first tenant (Group), as the
   existing connector tests do.
5. `sqlx` offline: none of the new queries use the `query!` macros.

**Plan/code mismatches and choices.**

- E2: `Principal.id` is `PrincipalId::{User(Uuid), Service(Uuid)}` (public
  field, public enum, re-exported from `lakehouse_auth`);
  `kind_for_audit()` is a `match` on it. The handler uses
  `matches!(principal.id, PrincipalId::Service(_))` and
  `catalog::is_unrestricted`. A handler can tell a service from a user; no
  name is compared. Service principals come from `ServiceTokenAuthenticator`
  with `scopes` as permissions; the Dagster service identity is therefore
  scoped by `ingest:read` and sees every row through the `Service` check.
- E5: `detail` decides existence in two places (a `silver.`/`serving.` id by
  `catalog_source::clickhouse_source`, any other id by a slug row in
  `bronze_meta[_sec].dataset_sync`, inside `bronze_asset_detail_body`), and
  `catalog_profile::resolve_source` already packages both for `sample` and
  `profile`. `ensure_asset_exists` calls `resolve_source` and adds nothing
  of its own to the definition; the only added logic is reading its `Ok(None)`
  as "no such asset" for a `silver.`/`serving.` id and as "exists but
  nothing readable" for a Bronze slug. Cost: one `system.columns` read for a
  silver or serving id; one registry read plus the `bronze_source` lookups
  (an Iceberg `DESCRIBE` if configured, then `system.columns`) for a Bronze
  slug. No catalog listing, no cheaper single-asset lookup existed. Any
  non-404 failure becomes the fixed 503 "The catalog could not be checked.
  Try again." (`resolve_source` alone would have surfaced a 422 with
  upstream text, which is why its errors are not passed through). One
  consequence: a Bronze slug that is registered but whose table lookup
  fails is a 503, not a pass (fail closed).
- E3: a connector of another tenant, or with no tenant, still closes the name
  (ADR 0014, decision 5, a connector's table must never be loaded by an
  upload) but with `TABLE_NOT_FREE`. So `ensure_table_free` asks
  `any_connector_of_tenant_targets` first (-> `CONNECTOR_TABLE`), then the
  existing `any_connector_targets` (-> `TABLE_NOT_FREE`), which stays in use.
  The existing test
  `a_connector_that_loads_the_table_is_refused_even_when_the_tenant_holds_the_claim`
  and `a_table_a_connector_loads_is_409_and_nothing_is_claimed` use
  `conn-pg-lakehouse`, a Group connector, so they still cover "own tenant's
  connector gives `CONNECTOR_TABLE`" without a new test.
- Order in `put_annotation`: gate, id length, existence, then the body. A
  bad body for a non-existent id is 404, not 400.
- Table claims: untouched, as the plan says.
- The console shows the old `TABLE_NOT_FREE` sentence only in
  `src/services/clients/uploads.test.ts` (a stubbed server reply); updated
  to the new sentence. No page maps it.
- `docs/core/features/upload-file.md` did not quote the old sentence; I added
  the new rule and a QA step (15a) rather than editing a quote. The ADR was
  not touched.
- Not changed: `routes/lineage.rs`, `routes/pipelines.rs`, Dagster, the
  spec's "Today" column, any migration or dependency.

## 8. Review

(The planner writes here.)
