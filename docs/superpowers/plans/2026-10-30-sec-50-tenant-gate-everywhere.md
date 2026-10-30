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
- **S8** (review round 1). Gate the two fingerprint reads, or document why not: `publication_body` (`gold.rs:~775`) and `pipeline_lag::clickhouse_fingerprint_token` (`pipeline_lag.rs:279`).
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

### Handoff (slice 1)

Developer: a non-Opus coding agent. Branch `fix/sec-50-tenant-gate`. The
plan's base (`main` at `9540552c`) is an ancestor of HEAD; the branch's
merge-base with the current `origin/main` is `4ce58470`, and the 7 commits
above it are the ones listed below. Everything below was run in the
foreground on bluefin through `/home/shiro/rantai/bin/cargo-locked`; no
`cargo clean`, no background job.

#### Commits

| Commit | What |
| --- | --- |
| `8180ce10` | `docs(plan): SEC-50, every statement passes the tenant gate` — the plan itself. |
| `8343ccf6` | `feat(queries): one tenant gate entry point for every caller (SEC-50 T1)` — `GateActor` (`Caller` / `AsTenant` / `CatalogTenant`) and `gate_statement`, lifted out of the query path so every site asks the same gate. |
| `42c29fdd` | `feat(pipelines): the tenant gate on the authored-pipeline rewrite (SEC-50 T2)` — `enforce_pipeline_sql` gates the statement as the pipeline's own tenant, before the policy rewrite. |
| `e0436ab1` | `feat(gold): the tenant gate on the Gold export (SEC-50 T3)` — the export gates as the catalog's tenant, before the first read of the mart; the fingerprint read is gated too. |
| `04cced98` | `feat(startup): the boot warning names every path that needs the catalog tenant (SEC-50 T4)` — `warn_if_catalog_tenant_unconfigured` names the new paths. |
| `96b5b4af` | `docs(sec-50): what the tenant gate covers, and what an operator must set (SEC-50 T5)` — the feature page and `docs/OPERATIONS.md`. |
| `abda61c4` | `test(pipelines): the lineage fixture gives its pipeline a tenant (SEC-50)` — the one fix this slice needed; see "The one failure SEC-50 caused" below. |

#### Commands run, with their counts

All from `rust/`, through `/home/shiro/rantai/bin/cargo-locked`:

```text
cargo fmt --check
  exit 0

cargo clippy --workspace --all-targets --all-features -- -D warnings
  exit 0

cargo test --workspace          # > /tmp/sec50-test-full.log
  exit 0 — 159 binaries, 8576 passed, 0 failed, 51 ignored
```

From the repo root:

```text
POSTGRES_PASSWORD=d RUSTFS_ACCESS_KEY=d RUSTFS_SECRET_KEY=d \
LAKEKEEPER_ENCRYPTION_KEY=d CONNECTOR_SECRETS_KEY=d \
  docker compose --profile '*' config --quiet
  -> compose-ok

git status
  -> nothing to commit, working tree clean
```

Scoped, while fixing the one failure (per `AGENTS.md`, "Keep build time down"):

```text
cargo test -p lakehouse-api --test column_lineage_pipeline
  -> 7 passed; 0 failed
cargo fmt --check && cargo clippy -p lakehouse-api --all-targets --all-features -- -D warnings
  -> exit 0
```

#### The one failure SEC-50 caused, and what was done about it

The first full workspace run stopped at the first failing binary (cargo is
fail-fast by default) and reported **19 binaries, 5914 passed, 4 failed, 37
ignored**, exit 101. The four failures were all in
`rust/crates/lakehouse-api/tests/column_lineage_pipeline.rs`, and all four
were SEC-50's gate:

```text
---- the_enforce_call_keeps_the_submitted_sql_in_postgres stdout ----
thread '...' panicked at crates/lakehouse-api/tests/column_lineage_pipeline.rs:206:5:
assertion `left == right` failed: {"error":"Table not found or not available to your organisation: bronze.orders"}
  left: 404
 right: 200
```

**This is a mismatch between the plan's D5 and an existing fixture, not a
bug in the code.** T2's accept list says "a pipeline with NULL `tenant_id` →
refused" (which is D5, and which the new
`tests/pipeline_enforce.rs::a_pipeline_with_no_tenant_is_refused` asserts),
while the same section also says "the existing tests still pass".
`tests/column_lineage_pipeline.rs` seeded a tenant-NULL pipeline and expected
the enforce call to answer `200`, so under D5 it is now a refusal. The two
lines of the plan cannot both hold for that fixture.

What was done: the **fixture** was changed, not the assertion. Every pipeline
in that file now carries a tenant — Meridian Group
(`11111111-1111-4111-8111-000000000001`), the tenant the seeded identity
migration gives `yoga`, the author there — and the deployment names that
tenant in `CATALOG_TENANT_ID`. The statement those tests read
(`bronze.orders`) is a table no upload claimed, so only the catalog's own
tenant may read it, which is what a pipeline that can run at all looks like
now. The seven tests assert the same statuses and the same lineage mappings
they asserted before; no assertion was weakened, removed or relaxed
(`AGENTS.md` rule 2). `abda61c4` carries the reasoning.

#### Mutation checks (section 5)

Done by the previous session; the mutations were reverted and `git diff` was
empty when this handoff started. Quoted from the logs:

```text
/tmp/sec50-mut-t2.log
  thread 'a_pipeline_cannot_read_an_upload_table_another_tenant_claimed' panicked at
    crates/lakehouse-api/tests/pipeline_enforce.rs:672:5:
  assertion `left == right` failed: {"sql":"SELECT * FROM bronze.group_orders"}
    left: 200
   right: 404
  test a_pipeline_cannot_read_an_upload_table_another_tenant_claimed ... FAILED
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 16 filtered out; finished in 0.97s

/tmp/sec50-mut-t3.log
  thread 'a_job_with_no_catalog_tenant_is_refused_before_anything_reaches_the_engine' panicked at
    crates/lakehouse-api/tests/gold_export_auth.rs:204:5:
  assertion `left == right` failed: {"error":"Lakekeeper gold-export token is unavailable (check LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE; see ADR 0011)"}
    left: 503
   right: 422
  test a_job_with_no_catalog_tenant_is_refused_before_anything_reaches_the_engine ... FAILED
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.98s
```

Both gates hold: with the gate removed, the cross-tenant pipeline reads the
other tenant's upload table, and the job with no catalog tenant reaches the
token file instead of being refused with D6's sentence.

#### Did the AI gold tool need a change? (T3)

**No, not one of its own.** The AI tool `export_gold_mart`
(`rust/crates/lakehouse-api/src/routes/ai/tools/gold.rs:105`) calls
`crate::gold_export::export_mart(...)` directly, and the gate lives inside
`export_mart` (`rust/crates/lakehouse-api/src/gold_export.rs:670`), before
the first read of the mart. So the copilot's tool goes through the gated path
by construction, and there is no second copy of the gate to keep in step.

The only line that changed in that file is mechanical: `export_mart` now
takes `state` (it carries the deployment's tenant count and
`CATALOG_TENANT_ID`), so the call site passes it. The other gated read, the
export's content fingerprint, is gated in
`rust/crates/lakehouse-api/src/routes/gold.rs:285` (`gated_source_table`),
which is the route's own path.

#### Not verified, with the reason

- `bun run typecheck && bun run lint && bun run test` — **not run.** The slice
  changes no TypeScript file: the branch touches 15 files, all Rust or
  Markdown. Per `AGENTS.md` ("Only build what changed") and the plan's
  section 6, which lists only the Rust and compose lines.
- `python3 ops/lint/check_intra_package_imports.py &&
  python3 ops/lint/check_bare_iceberg_count.py` and
  `(cd dagster && python -m pytest dispar_orchestrate -q)` — **not run**, for
  the same reason: no Python and no Dagster file changed.
- `docker compose up` on a profile — **not run.** The slice changes no
  compose file; section 6 asks for `config --quiet`, which passed. (`AGENTS.md`
  rule 8 asks for a real `up` only for a compose edit.)
- The `ops/g8` harnesses — **not run.** They need a live deployment. The
  plan states they put the analyst in the catalog's tenant already, so they
  should stay green; that is an expectation, not a measurement, and the
  reviewer should run them.
- Slice 2's mutation checks — out of this slice.
- The reviewer's own re-run of the block above — the reviewer's job, per
  `AGENTS.md` step 4.

#### Not done, by rule

- `docs/core/BACKLOG.md` was not touched (the reviewer sets the status, the
  PR and the shipped date at merge).
- No pull request was opened or merged (the developer never does).

## 9. Review (planner appends below)

### Review round 1 (planner, 2026-10-30, at `1f15d91f`)

The review covers `8343ccf6..1f15d91f` and the handoff. An independent read-only pass read the
whole diff and the CI workflow. I checked both blockers myself in `ci.yml` and the harnesses.

**What holds:**
- **The gate.** `Caller` is unchanged for Query Studio. `AsTenant(None)` gives `NoTenant`.
  `AsTenant(Some)` goes through `tenant_scope::headers_for_tenant` and `resolve_reach`, one rule,
  not two. `CatalogTenant` gives `All`, then `Tenant`, then the D6 sentence, byte for byte.
- **Pipelines.** The gate runs before the role rewrite and before the engine, and the test proves
  the engine is never asked. The wrong doc comment is corrected truthfully.
- **Gold.** The route gates before the fingerprint, and `export_mart` gates before the count, the
  `LIMIT 0` probe and the batch.
- **The three files outside the plan** are justified: `mod query` becomes `pub(crate)`, which
  `gold_export.rs` needs; `tenant_scope::headers_for_tenant` reuses `resolve`; `gold.rs` was named
  in T3.
- **No assertion was weakened.** The `column_lineage_pipeline.rs` fixture change (`abda61c4`) and
  the `pipeline_enforce.rs` fixture change in T2 are both right under D5. The handoff should have
  named the second one.

#### Planner decisions for this round (amend D3 and D5)

- **R1, amends D5.** A `*:*` author passes `AsTenant` in both cases, `None` or `Some(t)` whether
  or not the author is in `t`. This is `SEC-20`'s signed rule ("a principal holding `*:*`
  passes"), and an unrestricted principal reads everything in Query Studio anyway, so a pipeline
  running as that principal reaches no more than the principal does. Check `is_unrestricted`
  first in the `AsTenant` arm, and say so in the doc at `authored_pipelines.rs:1368-1393`.
- **R2.** On `POST /api/pipelines/runnable/{id}/enforce`, which only the orchestrator calls, a
  gate refusal answers **422** with the same fixed sentence, not 404.
  - The orchestrator treats only 409 and 422 as final (`authored_factory.py:413`). A 404 is
    retried as a transient failure and shows the operator a generic `HTTPError`.
  - `SEC-20`'s reasons for a 404 (no oracle for a person, Query Studio's *Run all*) do not apply
    to a service caller.
- **R3.** A pipeline whose `tenant_id` is NULL, with an author who is not `*:*`, is refused with
  its own fixed **409**, before the gate: `This pipeline belongs to no tenant. An administrator
  assigns one with PUT /api/pipelines/{id}/tenant.` The table sentence would mislead, and 409 is
  final for the orchestrator.
- **R4, amends D3.** Gold export:
  - a **signed-in person** is gated as `GateActor::Caller`, so their tenant must own the
    catalog;
  - a **service principal or the run token** is gated as `CatalogTenant`.
  D3 covers jobs with no user. A person from another tenant who holds `gold:export` must not
  export the shared catalog.
- **R5.** With no pool, `CatalogTenant` refuses with the D6 sentence (count `None` and the setting
  set → refuse). That is `SEC-20`'s own rule: `catalog_tenant_refusal` refuses with no pool,
  `routes/catalog.rs:173-175`.

#### BLOCKER

1. **The G7 check (`Rust · G3a Dagster + dlt Bronze ingest`) will go red.**
   - `ops/g7/g7_pipeline_test.py:420-431` and `:933-946` create Bronze pipelines as the bootstrap
     admin. That admin is `*:*` and has no tenant (`main.rs` `bootstrap_admin`), and no `X-Tenant`
     is sent. So the pipelines are stored with a NULL tenant, and D5 refuses them with a 404.
   - The job is green on `main`, runs on every Rust change, and was not run in the handoff.
   - Fix: R1. The harness does not change.
2. **The CI check `Rust · Gold export to Iceberg (ADR 0010)` will go red.**
   - Its `.env` (`ci.yml:~2681-2702`) names no `CATALOG_TENANT_ID`, and migration `0002` seeds
     four tenants. So `CatalogTenant` answers 422 to `ops/gold_export/gold_export_test.py:181`.
   - Fix: add `CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001` to that job's `.env`, with
     a comment saying why. This is exactly what #244 did for the two G8 jobs (`ci.yml:2777`,
     `:2922`). It is D2/D3's documented contract, not a weaker gate.

#### SHOULD-FIX

1. R2: the 422 on the enforce route, with a test that a cross-tenant statement answers 422 there.
2. R3: the 409 for a NULL-tenant pipeline, with a test.
3. R4: Gold by a person is `Caller`. Test: a user of another tenant holding `gold:export` is
   refused, and a user of the catalog's tenant passes.
4. R5: `CatalogTenant` with no pool refuses. Add a unit test.
5. **`export_mart`'s own gate is untested.** The AI tool's only gate is the one at
   `gold_export.rs:~670`, and the route's gate fires first in every test. Add a test through
   `export_mart` (or the AI tool) with several tenants and no setting: the D6 sentence, and
   nothing reaches the engine. Run the mutation check on that gate and quote it.
6. **R1's unit tests:** a `*:*` principal with `AsTenant(None)` and with `AsTenant(Some(t))` for a
   `t` it is not in → `All`.
7. **Docs that are not true for slice 1:**
   - The boot warning (`main.rs:~1199-1206`) says dashboards, embeds, public links and alerts are
     refused. They are not gated until slice 2. Name only what is gated now, and say the rest
     follows in slice 2.
   - `docs/OPERATIONS.md:~1577` says `.env.example` uses the same wording. Update `.env.example`
     (`:559-565`) to match.
   - `docs/OPERATIONS.md:~1599-1603`: without the setting, a regular member "sees no shared rows".
     Not true for dashboards until slice 2. Also, with the setting unset there is no catalog
     tenant at all, so "a member of any tenant other than the catalog's own" is wrong.
   - Add to `OPERATIONS.md`: how to find pipelines with no tenant, and how to assign one (`PUT
     /api/pipelines/{id}/tenant`), checked against the code. Since `0042` no backfill was ever
     done, so older deployments have such pipelines, and R3 now refuses them.
   - The feature page (`queries-scoped-to-the-tenant.md:~41`) says two paths can name
     `bronze.*`. Only pipelines can (plan section 0).

#### NIT

- `TenantGateRefused(#[from] ApiError)` (`gold_export.rs:~145`) would turn any future `?` on an
  `ApiError` in that file into "tenant gate refused". Use an explicit `.map_err`.
- `OPERATIONS.md:~1591` says "the scheduled Gold export". The 422 also hits a manual or assistant
  export done as a service.

#### Added to slice 2 (section 4)

- **S8.** The two fingerprint reads that remain ungated:
  - `publication_body`'s fingerprint (`gold.rs:~775`, answers a boolean only);
  - `pipeline_lag::clickhouse_fingerprint_token` (`pipeline_lag.rs:279`, orchestrator only).
  Neither leaks rows. Gate both or document why not.

**Open: 2 BLOCKERs.** One fix round, in order: BLOCKER 1 (R1), BLOCKER 2, then SHOULD-FIX 1-7,
then the NITs.
- Cite `PR #NN review round 1 B<n>/S<n>` at each site. The PR number is filled in when it is
  opened; until then, cite `SEC-50 review round 1 B<n>/S<n>`.
- Then the full verification block, plus the two CI checks above run locally if possible. If not,
  say so: CI runs them on the PR.
- Then a short round 1 handoff under section 8.
