# SEC-50: every statement passes the tenant gate, not only Query Studio's

Backlog: `SEC-50` (Task, Administration & Security, P2). Waits for `SEC-20`: merged (#244 on
2026-10-24, #263 on 2026-10-27). Base: `main` at `9540552c`. Every `file:line` below was
read at that commit.

Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does not change
a decision below. If the code disagrees with this plan, stop and report it.

## 0. What is wrong, in one paragraph

`SEC-20` put a tenant gate inside `routes::query::rewrite_sql_for_principal_with_headers`
(`routes/query.rs:110`). It runs `sql_rewrite::gate_targets` and then
`refuse_unreachable_tables` (`:546`) **before** `policy_engine::rewrite_sql_for_roles`. Thirteen
production call sites skip that function and call `rewrite_sql_for_roles` directly, so none of
them is gated:
- authored pipelines (`routes/authored_pipelines.rs:1462`);
- gold export (`gold_export.rs:483`);
- every dashboard path (`routes/support.rs:312`, `routes/dashboard.rs:2973`, `:3672`, `:4043`,
  `routes/dashboard_sources.rs:145`, plus their callers in embed, render, metrics and the report
  runner);
- alerts (`routes/alerts.rs:644`).

The real data leak is authored pipelines. They are the only path that can name a `bronze.*`
table, and `bronze.*` is where another tenant's upload lives (`upload_table_claim`, migration
`0058`). The other paths read only `serving.*`: `sql_guard::check_sql_source`
(`sql_guard.rs:296`), `lakehouse-bi/src/builder.rs:344`, `lakehouse-alerts/src/lib.rs:1337`, and
the gold source schema. There, the gate decides who may read the shared catalog.

## 1. Decisions (made; do not change)

- **D1. One gate, called from every site.** `SEC-20`'s gate (`resolve_reach`, `reach_from`,
  `refuse_unreachable_tables`, `query.rs:286-600`) gets one `pub(crate)` entry point that every
  site calls before its `rewrite_sql_for_roles`. Nobody writes a second copy of the reach logic
  (`AGENTS.md` rule 4).
- **D2. Signed-in users follow `SEC-20`'s rule** (product owner, 2026-10-30). The caller's
  tenant is the request's `X-Tenant` header, else the caller's first tenant
  (`tenant_scope::resolve`, `tenant_scope.rs:51`).
  - With more than one tenant, only the tenant named in `CATALOG_TENANT_ID` may read the shared
    catalog (`serving.*`, Silver, Gold).
  - A principal holding `*:*` passes. A deployment with one tenant behaves as before.
  - The owner accepted the consequence: with `CATALOG_TENANT_ID` unset (the default,
    `docker-compose.yml:661`, `.env.example:565`) and the four tenants every install seeds,
    regular users see no dashboard data until an operator sets it.
- **D3. Jobs with no user act as the catalog's tenant** (product owner, 2026-10-30). This covers
  alerts, the scheduled gold export, and embed tokens (signed with the deployment's own
  secret).
  - The reach is `All` when the deployment has one tenant.
  - It is `Tenant { CATALOG_TENANT_ID, owns_catalog: true }` when that setting is set.
  - Otherwise the job is refused with the fixed sentence in D6.
- **D4. Public dashboard links act as the tenant that made them.**
  - A new nullable column `dashboard_public_link.tenant_id` records the creator's acting tenant
    (header, else first tenant) when the link is made.
  - A link made before this change (NULL) acts as its creator's first tenant at view time,
    loaded from `created_by`.
  - A creator who no longer exists, or belongs to no tenant, fails closed: `TenantReach::NoTenant`.
- **D5. Authored pipelines act as the pipeline's own tenant.**
  - The tenant is `pipeline_definition.tenant_id` (`migrations/0042_tenant_provisioning.sql:60`,
    `Pipeline.tenant_id`, `lakehouse-store/src/pipelines.rs:120`), applied as if the author had
    sent that tenant in `X-Tenant`.
  - So an author who left that tenant is refused (`tenant_scope::resolve` answers 404).
  - A pipeline with a NULL tenant is refused as `NoTenant`, the migration's own fail-closed rule.
- **D6. Fixed sentences** (`AGENTS.md` principle 4). These are the only new ones:
  - a table outside the reach: `SEC-20`'s existing `table_not_available` 404 sentence
    (`query.rs:248`), unchanged;
  - a job with no user, several tenants and no `CATALOG_TENANT_ID`: 422, `This job reads the
    shared catalog, and no tenant owns it here. Set CATALOG_TENANT_ID to the tenant that owns
    this deployment's shared catalog.`
- **D7. The startup warning says what changed.** `warn_if_catalog_tenant_unconfigured`
  (`main.rs:1205`) names dashboards, embeds, public links, alerts, gold export and authored
  pipelines, besides the catalog and the Dagster-job list it names today.
- **D8. Two pull requests.** #250 (`BI-19`, open) changes `alerts.rs`, `dashboard.rs`,
  `dashboard_sources.rs`, `embed.rs`, `metrics.rs`, `public_links.rs`, `render.rs` and
  `report_runner.rs`.
  - **Slice 1 (this plan, now):** the shared entry point, authored pipelines, gold export, the
    warning and the docs. None of these files overlaps #250.
  - **Slice 2:** everything else. It starts only after #250 merges, and its anchors are re-read
    then. Its tasks are listed in section 4 so the scope is fixed now.

## 2. Anchors (at `9540552c`)

| What | Where |
| --- | --- |
| The gated rewrite | `routes/query.rs:110` (`rewrite_sql_for_principal_with_headers`), `:133` (`_inner`), the gate call at `:162-164` |
| The reach | `resolve_reach` (`:284`), `reach_from` (`:330`), `TenantReach` (`:228`), `deployment_tenant_count` (`:442`) |
| The table check | `refuse_unreachable_tables` (`:546`), `claim_on` (`:511`), `raw_table_name` (`:483`) |
| The tenant from a header | `tenant_scope::resolve` (`tenant_scope.rs:51`) |
| The role rewrite (ungated) | `policy_engine::rewrite_sql_for_roles` (`policy_engine.rs:237`) |
| Pipelines | `enforce_pipeline_sql` (`routes/authored_pipelines.rs:1389`); the author is loaded at `:1419-1424`, the rewrite at `:1462`; `POST /api/pipelines/runnable/{id}/enforce` (`routes/mod.rs:314`), called by `dagster/dispar_orchestrate/authored_factory.py:486` |
| Gold export | `export_mart` (`gold_export.rs:612`): unrewritten count at `:639` and schema probe at `:668`, batch `export_batch_sql` at `:483` (loop `:692`); route `routes/gold.rs:300`, unrewritten `mart_content_fingerprint` at `gold.rs:320`; AI tool `routes/ai/tools/gold.rs:73` (`:105`) |
| Startup warning | `warn_if_catalog_tenant_unconfigured` (`main.rs:1205`) |
| Tests to reuse | `tests/query_tenant_scope.rs`: `spin_up_with_clickhouse` (`:72`), `claim_table_for`, `belong_to`, `post_sql_for_tenant`, the `OWNER_TENANT`/`OTHER_TENANT` constants. `tests/pipeline_enforce.rs`: `app()` (`:74`), `seed_pipeline(pool, run_as, tenant_id)` (`:112`), `first_tenant_of` (`:149`), `enforce` (`:161`). `tests/gold_export_auth.rs` |
| Feature page | `docs/core/features/queries-scoped-to-the-tenant.md`, "Limits to tell a customer" (`:67`) |
| Migrations | highest on `main` `0116`; open PRs hold up to `0119` (#277). Slice 2's migration takes the next free number above every number on `main` and on open PRs when it is written |

## 3. Slice 1 tasks (one commit each, in order)

Per commit, Rust checks are scoped to the crate (`AGENTS.md`, "Keep build time down"). Push
after every commit.

**T1. The shared entry point (`routes/query.rs`).**
- Add `pub(crate) enum GateActor<'a>` with three cases:
  - `Caller { principal: &'a Principal, headers: &'a HeaderMap }`, today's behaviour;
  - `AsTenant { principal: &'a Principal, tenant_id: Option<Uuid> }` (D5: `None` resolves to
    `NoTenant`; `Some` is checked with `tenant_scope::resolve` as if sent in `X-Tenant`);
  - `CatalogTenant` (D3).
- Add `pub(crate) async fn gate_statement(state, sql, dialect, actor) -> Result<(), ApiError>`.
  It runs `gate_targets` and the same table and listing checks `refuse_unreachable_tables` runs.
- Make `rewrite_sql_for_principal_inner` call `gate_statement` with `GateActor::Caller`, so
  Query Studio's behaviour does not move.
- For `CatalogTenant`, the reach comes from the tenant count and `CATALOG_TENANT_ID` only: `All`
  if one tenant, `Tenant { id, owns_catalog: true }` if set, else the D6 refusal.
- Doc comments: the `SEC-50` tag, `# Errors`, and why the catalog tenant is the right actor for
  a job with no user.
- *Accept:*
  - the existing `routes::query::tests` and `tests/query_tenant_scope.rs` pass unchanged;
  - new unit tests in `query.rs` cover the reach for every `GateActor` case, including
    `AsTenant { tenant_id: None }` → `NoTenant` and `CatalogTenant` with several tenants and no
    setting → the D6 sentence.

**T2. Authored pipelines (`routes/authored_pipelines.rs`).**
- In `enforce_pipeline_sql`, call `gate_statement(.., GateActor::AsTenant { principal: &author,
  tenant_id: pipeline.tenant_id })` before `rewrite_sql_for_roles` (`:1462`).
- Fix the doc comment that promises a 404 "for another tenant's" pipeline. The code only 404s
  an unknown id (`AGENTS.md` rule 1). Say what it does: the route is for the orchestrator only,
  and the statement is gated as the pipeline's tenant.
- *Accept:* new tests in `tests/pipeline_enforce.rs`, reusing its helpers:
  - a pipeline of tenant B whose SQL reads a `bronze.*` table claimed by tenant A → 404 with
    the `SEC-20` sentence, and nothing reaches the engine;
  - the same table claimed by tenant B → 200;
  - a pipeline with NULL `tenant_id` → refused;
  - an author no longer in the pipeline's tenant → refused;
  - the existing tests still pass.

**T3. Gold export (`gold_export.rs`, `routes/gold.rs`, `routes/ai/tools/gold.rs`).**
- Gate as `GateActor::CatalogTenant` once per export, before the first read of the mart.
- The unrewritten reads (the count at `:639`, the schema probe at `:668`, the fingerprint at
  `gold.rs:320`) must run only after the gate passes. Move the gate to the top of `export_mart`
  and say why at the site (`SEC-50`).
- The AI tool goes through the same function, so it needs no change of its own. Check this and
  say so in the handoff.
- *Accept:* new tests (extend `tests/gold_export_auth.rs`):
  - several tenants, no `CATALOG_TENANT_ID`, a service caller → 422 with the D6 sentence, and no
    `SELECT count()` reaches the wiremock ClickHouse;
  - the same with `CATALOG_TENANT_ID` set → today's behaviour;
  - one tenant → today's behaviour.

**T4. The startup warning (`main.rs:1205`).**
- List the new paths (D7) in the warning text.
- *Accept:* the existing test of the warning, if any, is updated; otherwise add one that checks
  the text names `dashboards` and `alerts`.

**T5. Docs.**
- `docs/core/features/queries-scoped-to-the-tenant.md`: in "Limits", say authored pipelines and
  gold export are now gated (slice 1), and that dashboards, embeds, public links and alerts
  follow in slice 2.
- `docs/OPERATIONS.md`: state where `CATALOG_TENANT_ID` is described, and that with several
  tenants it is required for regular users to see shared data.
- The developer does not touch `BACKLOG.md`; the reviewer sets it at merge.

## 4. Slice 2 (after #250 merges; anchors re-read then)

Listed so the scope is fixed now. A separate plan section and PR.

- **S1.** Dashboards for a signed-in user, gated as `GateActor::Caller`. The handlers gain
  `headers: HeaderMap`. Sites: `run_spec_sql` / `try_run_spec_sql` (`support.rs:227/312`) and
  its callers (`dashboard::get`, `specs_preview`, layers, `add_bins`, `annotate_table`,
  `preview_rows`), `run_records` (`dashboard.rs:2973`), `build_chart_file` (`:3672`, download
  routes), `values_with_limit` (`:4043`), `dashboard_sources` save and preview (`:145`,
  `:391`), and `metrics::value_query` (`metrics.rs:724`).
- **S2.** `/api/dashboard/values` with no board lists `serving` tables from `system.columns`
  (`dashboard.rs:4020`). Gate it as a listing.
- **S3.** Embeds (`POST /api/embed/data`, `/api/embed/values`): `GateActor::CatalogTenant` (D3).
- **S4.** Public links (D4):
  - a migration adds `dashboard_public_link.tenant_id UUID NULL REFERENCES tenant(id) ON DELETE
    SET NULL`, with a why-header and no backfill;
  - creation records the acting tenant;
  - viewing gates as `AsTenant` (the link's tenant, else the creator's first tenant).
- **S5.** Alerts (`alerts.rs:644`, the AI `run_alert_rule`): `GateActor::CatalogTenant`. A rule
  refused by the gate is recorded as failed with the D6 sentence; the run goes on to the next
  rule.
- **S6.** The report runner (tick and send-now): `GateActor::Caller` with the subscription
  owner's principal and an empty `HeaderMap`, which means the owner's first tenant.
- **S7.** Tests: for each route, a non-catalog tenant is refused, the catalog tenant passes, and
  `*:*` passes. Plus the mutation checks of section 5.

## 5. Mutation checks (in the handoff)

For each gated site, remove the `gate_statement` call on purpose, run the named test, and quote
that it fails. Then restore it. Slice 1: T2's cross-tenant test and T3's no-`CATALOG_TENANT_ID`
test.

## 6. Verification before handoff

`AGENTS.md`, the Rust and compose lines. Run on bluefin (the build machine), in the
foreground, on the final commit:

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
docker compose --profile '*' config --quiet
git status
```

The G8 harnesses already put the analyst in the catalog's tenant (#244), so they should stay
green. If a G-gate goes red, read why before changing a harness, and quote the failure in the
handoff.

## 7. Out of scope

- Per-tenant `serving` marts (a tenant's own Gold). That is a product feature, not this fix.
- Moving the `*:*` rule or the one-tenant rule. They stay as `SEC-20` signed them.
- `SEC-55` (the Parquet preview in the S3 connector). That is its own item.

## 8. Handoff (developer appends below)

## 9. Review (planner appends below)
