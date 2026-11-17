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

### Handoff (review round 1)

Developer: a non-Opus coding agent. Branch `fix/sec-50-tenant-gate`, on top of
the review commit `6a0efbcd`. Every finding of review round 1 is answered below,
in the order the review lists them. Everything was run in the foreground on
bluefin through `/home/shiro/rantai/bin/cargo-locked`; no `cargo clean`, no
background job, no `~/.cargo/config.toml` change.

#### Commits

| Commit | What |
| --- | --- |
| `177ebe17` | `fix(gate): an unrestricted principal passes the tenant gate (PR #281 review round 1 B1)` -- R1: `GateActor::AsTenant` checks `is_unrestricted` FIRST, so a `*:*` principal passes with a tenant and without one, whether or not it belongs to that tenant. With the R1 unit tests (S6) and an HTTP-level test. |
| `b4978e27` | `ci(gold-export): name the catalog tenant in the Gold export job's .env (PR #281 review round 1 B2)` -- the `Rust · Gold export to Iceberg (ADR 0010)` job's `.env` names `CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001`, with a comment saying why, exactly as the two G8 jobs do. |
| `e0d1c391` | `fix(pipelines): a gate refusal on the enforce route answers 422 (PR #281 review round 1 S1)` -- R2: the gate's own sentence, with this route's status; an upstream failure keeps its own. |
| `1803ba76` | `test(pipelines): the enforce harness gives each test its own engine (PR #281 review round 1)` -- the one commit the review did not ask for; see "The one extra commit" below. |
| `2144cc34` | `fix(pipelines): a pipeline with no tenant is refused with a 409 (PR #281 review round 1 S2)` -- R3: a NULL-tenant pipeline with an author who is not `*:*` is refused before the gate, with its own sentence. |
| `8ef9a0ff` | `fix(gold): a signed-in person's Gold export is gated as that person (PR #281 review round 1 S3)` -- R4: a person is gated as `Caller`, a service identity or the run token as `CatalogTenant`, through one helper both the route and the copilot's tool use. |
| `3f3d7964` | `fix(gate): a job with no user is refused when the deployment cannot count its tenants (PR #281 review round 1 S4)` -- R5: `CatalogTenant` with no pool refuses with D6's sentence, even with the setting named. |
| `b6383c38` | `test(gold): the Gold export's own gate is tested (PR #281 review round 1 S5)` -- a test through `export_mart` itself: D6's sentence, and no request reaches the engine. |
| `c3dfbe40` | `docs(sec-50): the boot warning and the operator docs say what is gated today (PR #281 review round 1 S7)` -- the boot warning, `.env.example`, `docs/OPERATIONS.md` and the feature page. Also NIT 2 (the "scheduled Gold export" wording). |
| `25684c8f` | `refactor(gold): the gate's error is wrapped explicitly, not by a From (PR #281 review round 1 NIT)` -- NIT 1: `TenantGateRefused` loses `#[from]`, and the one gate call wraps the error itself. |

#### Commands run, with their counts

The full block, on the final commit (`25684c8f`), from `rust/`:

```text
cargo fmt --check
  exit 0

cargo clippy --workspace --all-targets --all-features -- -D warnings
  exit 0

cargo test --workspace          # > /tmp/sec50-r1-test.log
  exit 0 -- 159 binaries, 8588 passed, 0 failed, 51 ignored
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

Scoped, per commit (per `AGENTS.md`, "Keep build time down"):

```text
B1  cargo test -p lakehouse-api --lib sec50_gate_tests      -> 9 passed; 0 failed
    cargo test -p lakehouse-api --test pipeline_enforce      -> 18 passed; 0 failed
B2  (no Rust file changed -- no cargo command)
S1  cargo test -p lakehouse-api --test pipeline_enforce      -> 18 passed; 0 failed
    cargo test -p lakehouse-api --test column_lineage_pipeline-> 7 passed; 0 failed
    cargo test -p lakehouse-api --test sec11_guard            -> 4 passed; 0 failed
S2  cargo test -p lakehouse-api --test pipeline_enforce      -> 18 passed; 0 failed
    (run four times in a row after the harness fix: 18 passed every time)
S3  cargo test -p lakehouse-api --test gold_export_auth      -> 7 passed; 0 failed
    cargo test -p lakehouse-api --test gold_publication      -> 14 passed; 0 failed
    cargo test -p lakehouse-api --lib ai::tools::gold        -> 3 passed; 0 failed
S4  cargo test -p lakehouse-api --lib sec50_gate_tests       -> 10 passed; 0 failed
S5  cargo test -p lakehouse-api --lib gold_export            -> 33 passed; 0 failed
S7  cargo test -p lakehouse-api --bin lakehouse-api boot_warning -> 3 passed; 0 failed
NIT cargo test -p lakehouse-api --lib gold_export            -> 33 passed; 0 failed
     cargo test -p lakehouse-api --test gold_export_auth     -> 7 passed; 0 failed
route_auth (not touched, re-run at the end)                 -> 65 passed; 0 failed
sec11_guard (re-run at the end)                             -> 4 passed; 0 failed
```

#### Mutation checks (section 5)

Every new refusal was proved by removing the line that makes it and watching
the named test FAIL. The mutations were reverted; `git status` was empty when
the handoff started, and the final verification above ran on the restored tree.

```text
/tmp/sec50-r1-mut-b1.log   (B1 / R1: the `is_unrestricted` check in the AsTenant arm)
  test routes::query::sec50_gate_tests::an_unrestricted_principal_reaches_everything_even_as_a_site_with_no_tenant ... FAILED
  test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 2822 filtered out; finished in 1.18s
  test a_pipeline_authored_by_an_unrestricted_principal_is_not_refused_for_its_tenant ... FAILED
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out; finished in 1.09s

/tmp/sec50-r1-mut-s1.log   (S1 / R2: the 422 mapping on the enforce route)
  test a_pipeline_cannot_read_an_upload_table_another_tenant_claimed ... FAILED
    assertion `left == right` failed: {"error":"Table not found or not available to your organisation: bronze.group_orders"}
      left: 404
     right: 422
  test a_pipeline_with_no_tenant_is_refused ... FAILED
  test an_author_who_left_the_pipelines_tenant_is_refused ... FAILED
  test result: FAILED. 15 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.64s

/tmp/sec50-r1-mut-s2.log   (S2 / R3: the 409 for a NULL-tenant pipeline)
  test a_pipeline_with_no_tenant_is_refused ... FAILED
    assertion `left == right` failed: {"error":"Table not found or not available to your organisation: bronze.group_orders"}
      left: 422
     right: 409
  test result: FAILED. 17 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.52s

/tmp/sec50-r1-mut-s3.log   (S3 / R4: the actor a person is gated as)
  test a_person_of_another_tenant_holding_gold_export_is_refused ... FAILED
    assertion `left == right` failed: {"error":"Lakekeeper gold-export token is unavailable (check LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE; see ADR 0011)"}
      left: 503
     right: 404
  test result: FAILED. 6 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.49s

/tmp/sec50-r1-mut-s4.log   (S4 / R5: the refusal when the tenant count cannot be read)
  test routes::query::sec50_gate_tests::a_job_with_no_user_is_refused_in_a_deployment_that_cannot_count_its_tenants ... FAILED
  test routes::query::sec50_gate_tests::a_job_with_no_user_in_a_deployment_that_cannot_say_who_owns_the_catalog_is_refused ... FAILED
  test result: FAILED. 8 passed; 2 failed; 0 ignored; 0 measured; 2822 filtered out; finished in 0.97s

/tmp/sec50-r1-mut-s5.log   (S5: the gate inside `export_mart`)
  test gold_export::tests::a_job_with_no_catalog_tenant_is_refused_before_the_engine_is_asked ... FAILED
    a deployment that names no catalog tenant must refuse the export
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2832 filtered out; finished in 0.75s
```

#### What each finding became, and where

- **B1 (R1).** `routes/query.rs`'s `GateActor::AsTenant` arm checks
  `is_unrestricted` before it looks at the tenant at all, so `None` and
  `Some(t)` both give `All`. The rule is `SEC-20`'s own signed rule, and it is
  safe for the same reason it is safe in Query Studio. The doc at
  `routes/authored_pipelines.rs`'s "Which tenant the statement acts as" says so.
  Tests: two unit tests in `routes::query::sec50_gate_tests` (S6) and one
  HTTP-level test in `tests/pipeline_enforce.rs`.
- **B2.** `.github/workflows/ci.yml`, the `gold-export` job's "Write CI-only .env"
  step. The harness (`ops/g7`, `ops/gold_export`) is unchanged.
- **S1 (R2).** `routes/authored_pipelines.rs`'s `gate_refusal_as_unprocessable`
  maps the gate's own 404 to a 422 and leaves every other status alone, so an
  unreadable database stays a retryable 503. The sentence is the gate's.
- **S2 (R3).** `routes/authored_pipelines.rs`'s `NO_TENANT_REASON`, checked
  after the author is loaded and before the gate. A `*:*` author is exempt.
- **S3 (R4).** `routes/gold.rs`'s `gold_export_gate_actor` is the one place that
  decides, and `export_mart` now takes the actor. The copilot's tool
  (`routes/ai/tools/gold.rs`) uses the same helper, with no `X-Tenant` header --
  a tool call carries the model's arguments, not the chat's request headers, so
  the person's first tenant acts, as `tenant_scope::resolve` does when no header
  is sent.
- **S4 (R5).** `routes/query.rs`'s `catalog_tenant_reach` refuses when the count
  is `None`, with or without the setting.
- **S5.** A unit test in `src/gold_export.rs` calls `export_mart` itself.
- **S6.** The two `AsTenant` unit tests above.
- **S7.** `main.rs`'s boot warning (and its test), `.env.example`,
  `docs/OPERATIONS.md` and the feature page.
- **NIT 1.** `GoldExportError::TenantGateRefused` has no `#[from]`; the gate call
  wraps the error explicitly.
- **NIT 2.** `docs/OPERATIONS.md` and the feature page now say that the 422
  hits the scheduled export and a manual trigger done as a service -- not only
  the schedule. (Corrected by `PR #281 review round 2 S1`: the assistant's own
  tool is a SIGNED-IN PERSON's call -- `routes/ai/tools/gold.rs` gates it as
  `Caller` -- so it never reaches this 422; a person of another tenant gets the
  gate's own 404 instead.)

#### Two changes the review did not ask for, and why

1. **`export_mart` became `pub(crate)`** (`8ef9a0ff`). R4 makes the export's
   actor part of the function's signature, and the actor type is `routes::query`'s
   `pub(crate)` `GateActor` (the plan's T1). A `pub` function cannot take a
   `pub(crate)` type, and `export_mart` is called from inside this crate only --
   the route and the copilot's tool. Nothing outside the crate used it.
2. **The enforce harness gives each test its own engine** (`1803ba76`).
   `wiremock`'s `MockServer::start()` hands out a server from a global pool, and
   the server goes back into that pool as soon as the last handle to it is
   dropped. `tests/pipeline_enforce.rs`'s `app()` dropped the engine it built
   while the app's `CH_URL` still pointed at it, so the next test to ask for a
   server was given that same address -- and that test's requests landed in the
   first test's request log. The visible symptom was a flake in
   `a_pipeline_cannot_read_an_upload_table_another_tenant_claimed`, whose
   "nothing reached the engine" assertion saw another test's `system.columns`
   queries. It was latent at the review commit (five consecutive runs green) and
   became visible as soon as S2 moved the timing. The stand-in is now a BARE
   server (`MockServer::builder().start()`), which is never pooled, and `app()`
   hands the engine back so it stays alive for the whole test -- a bare server
   stops when its last handle goes, and the app's `CH_URL` must keep pointing at
   a running engine. No assertion was weakened: the same requests are still
   counted, on a server that now belongs to the test that counts them.

Two fixtures were updated because R4 and R3 changed what a caller must look
like, not because a test was hard to satisfy. No assertion was weakened,
relaxed or removed in either:

- `tests/gold_publication.rs`'s `Sessions::with` now mints its principals in the
  catalog's own tenant: under R4 a person who triggers a Gold export has to be a
  member of it. Every assertion is the one it was.
- `tests/gold_export_auth.rs` grants `gold:export` to two seeded people through
  an `access_grant` row -- the production mechanism for granting a permission to
  one person -- so that the "a person of another tenant" case and the "a person
  of the catalog's tenant" case are the same shape.

#### Not verified, with the reason

- **The `Rust · G3a Dagster + dlt Bronze ingest` check (BLOCKER 1) and the
  `Rust · Gold export to Iceberg (ADR 0010)` check (BLOCKER 2) were NOT run
  locally.** Both need a live compose stack with a real `ClickHouse` and
  Lakekeeper, which this machine does not run for the Rust test suite; CI runs
  both on the pull request. What IS verified here: BLOCKER 1's fix is the R1
  rule, and it is covered by three tests (`routes::query::sec50_gate_tests`'s two
  and `tests/pipeline_enforce.rs`'s one, which reproduces the G7 shape -- an
  unrestricted author, a pipeline with no tenant, the statement allowed).
  BLOCKER 2's fix is one line in the job's `.env`, and the same value is what
  the two G8 jobs already use.
- `bun run typecheck && bun run lint && bun run test` -- **not run.** The branch
  touches no TypeScript file: the ten commits above change Rust, Markdown, one
  `.env.example` and one CI workflow.
- `python3 ops/lint/check_intra_package_imports.py &&
  python3 ops/lint/check_bare_iceberg_count.py` and
  `(cd dagster && python -m pytest dispar_orchestrate -q)` -- **not run**, for
  the same reason: no Python and no Dagster file changed.
- `docker compose up` on a profile -- **not run.** No compose file changed;
  `config --quiet` passed, which is what the plan's section 6 asks for.
- `docs/core/BACKLOG.md` -- **not touched** (the reviewer sets the status, the
  PR and the shipped date at merge).
- `docs/core/PRODUCT.md` -- **not touched.** Its "Query tenancy" row still says
  that "dashboards, scheduled exports, alerts and authored pipelines are not
  scoped this way yet", which is now false for the authored pipelines and the
  Gold export. `AGENTS.md` makes that row the planner's, at Done.
- The reviewer's own re-run of the block above -- the reviewer's job, per
  `AGENTS.md` step 4.
- Slice 2's mutation checks and the two fingerprint reads of S8 -- out of this
  slice.

### Handoff (review round 2)

Developer: a non-Opus coding agent. Branch `fix/sec-50-tenant-gate`, on top of the
review commit `ba554bbb`. Every finding of review round 2 is answered below, in the order
the review lists them. Everything was run in the foreground on bluefin through
`/home/shiro/rantai/bin/cargo-locked`; no `cargo clean`, no background job, no
`~/.cargo/config.toml` change, and no container that this work did not start was removed.

#### Commits

| Commit | What |
| --- | --- |
| `bcf2d3b5` | `Merge remote-tracking branch 'origin/main' into fix/sec-50-tenant-gate` -- BLOCKER 2: the branch was behind `main` and protection requires it to be up to date. A merge, not a rebase, as the review says. |
| `1bfad2df` | `fix(g7): the BLD-1 gate authors as the tenant that owns the shared catalog (PR #281 review round 2 B1)` -- BLOCKER 1: the G3a job's CI-only `.env` names `CATALOG_TENANT_ID`, and `ops/g7/bld1_pipeline_author_test.py` reads the SAME setting, picks an author of that tenant and scopes the pipeline to it. Not one masking assertion changed. |
| `307c82e9` | `docs(sec-50): the assistant's Gold tool is a person's call, not a job with no user (PR #281 review round 2 S1)` -- SHOULD-FIX 1: `docs/OPERATIONS.md` and the round 1 handoff sentence now say that the assistant's own tool is gated as the SIGNED-IN PERSON, and that a signed-in person of another tenant gets the gate's own 404 rather than the 422. |
| `15edcb13` | `test(sec-50): the assistant's Gold tool is gated as the signed-in person (PR #281 review round 2 S2)` -- SHOULD-FIX 2: a new test calls the copilot's tool as a person of another tenant (holding `gold:export`, with a catalog tenant set) and is refused with the gate's own 404, with nothing reaching the engine. The mutation check is below. |
| `0550608d` | `ci(gold-export): say truthfully why this job names the catalog's tenant (PR #281 review round 2 NIT)` -- the `gold-export` job's comment no longer claims the export there "is a job with no user": that job's harness signs in as the `*:*` bootstrap admin, so its export passes as a `Caller` (`All`). The setting stays, with the reason that is true. |
| `4ab0b4bf` | `docs(ops): an unassigned pipeline is invisible to a tenant, not to a tenantless admin (PR #281 review round 2 NIT)` -- `docs/OPERATIONS.md` states the exception (a Platform Admin who belongs to no tenant of their own sees every authored pipeline) and says that `PUT /api/pipelines/{id}/tenant` needs `identity:write`. |
| `7de0801c` | `docs(gold): the export route's `# Errors` lists the 404 a person of another tenant gets (PR #281 review round 2 NIT)` -- `routes/gold.rs`'s `export` lists the gate's 404 beside the 422 it already listed. |
| `02b7053c` | `test(pipelines): the unrestricted-author test's doc says what the test checks (PR #281 review round 2 NIT)` -- `tests/pipeline_enforce.rs`: the doc no longer claims a second case the test does not check, nor a G7 shape that the harness does not produce. No assertion changed. |
| `95299d76` | `fix(sec-50): no pool means "could not verify tenant count", not "set CATALOG_TENANT_ID" (PR #281 review round 2 NIT)` -- R5, amended: the job-with-no-user actor in a deployment with no pool refuses with `SEC-20`'s own sentence (a 503), not D6's 422, and the sentence is ONE constant now. |

#### Commands run, with their counts

Everything below was run in the foreground, from `rust/`, through
`/home/shiro/rantai/bin/cargo-locked`, on the final commit `95299d76`:

```text
cargo fmt --check
  exit 0

cargo clippy -p lakehouse-api --all-targets -- -D warnings
  exit 0

cargo test -p lakehouse-api routes::ai::tools::gold   -> 4 passed; 0 failed
cargo test -p lakehouse-api sec50_gate_tests           -> 10 passed; 0 failed
cargo test -p lakehouse-api routes::gold               -> 16 passed; 0 failed
cargo test -p lakehouse-api routes::catalog            -> 211 passed; 0 failed
cargo test -p lakehouse-api --test pipeline_enforce    -> 18 passed; 0 failed
cargo test -p lakehouse-api --test gold_export_auth    -> 7 passed; 0 failed
cargo test -p lakehouse-api --test sec11_guard         -> 4 passed; 0 failed
```

The Python and compose lines, from the repo root (the B1 commit changed the G7 harness and
`docker-compose.yml`, so these are its own checks):

```text
python3 -m py_compile ops/g7/bld1_pipeline_author_test.py
  -> OK (no output)

python3 ops/lint/check_intra_package_imports.py   -> OK: all intra-package dispar_orchestrate imports resolve; exit 0
python3 ops/lint/check_bare_iceberg_count.py      -> R11 guard: no bare count() against a Bronze Iceberg table found; exit 0
python3 ops/lint/check_compose_init_readiness.py  -> exit 0
python3 ops/lint/check_connector_gate_coverage.py -> OK: every supported type is covered or explicitly waived (0 waived); exit 0
python3 -m unittest discover -s ops/lint -p 'test_*.py'   -> Ran 23 tests; OK
python3 -m unittest discover -s ops/gates -p 'test_*.py' -> Ran 29 tests; OK

POSTGRES_PASSWORD=d RUSTFS_ACCESS_KEY=d RUSTFS_SECRET_KEY=d \
  LAKEKEEPER_ENCRYPTION_KEY=d CONNECTOR_SECRETS_KEY=d \
    docker compose --profile '*' config --quiet
  -> exit 0

git status
  -> nothing to commit, working tree clean
```

#### The mutation check

S2 asked for one: swap the actor the copilot's tool is gated as, and watch the named test
FAIL. The mutation was `crate::routes::gold::gold_export_gate_actor(Some(principal), &headers)`
-> `crate::routes::query::GateActor::CatalogTenant`, in
`rust/crates/lakehouse-api/src/routes/ai/tools/gold.rs`. It was reverted afterwards; the
verification above ran on the restored tree, and the committed code gates as the signed-in
person.

```text
test routes::ai::tools::gold::tests::the_tool_gates_the_export_as_the_signed_in_person ... FAILED
  panicked at crates/lakehouse-api/src/routes/ai/tools/gold.rs:357:9:
  assertion `left == right` failed: the tool must gate the export as the signed-in person:
    {"namespace":"gold","table":"gold_export_smoke","formatVersion":2,"rowsExported":0,
     "note":"Append-only: menjalankan ulang menambah baris baru, bukan menggantikan."}
    left: None
   right: Some("Table not found or not available to your organisation: serving.gold_export_smoke")
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2834 filtered out
```

That is the leak the review named: with the catalog's tenant as the actor, a person of another
tenant exports the shared catalog through the assistant, and every other test in the file still
passes.

#### What each finding became, and where

- **BLOCKER 1 (the red G3a job).** `.github/workflows/ci.yml`'s `g3a-dagster` job's CI-only
  `.env` names `CATALOG_TENANT_ID`, with the reason; `ops/g7/bld1_pipeline_author_test.py`
  reads the SAME setting (the way it reads its other settings) and picks an author who
  belongs to that tenant, scoping the pipeline to it (`_user_by_role`, `_tenant_shared_with`).
  A holder of the role outside that tenant is refused with a clear message instead of being
  picked and refused by the API later; with the setting unset the harness keeps the behaviour
  it had. `docker-compose.yml`'s `bld1-test-runner` service passes the setting into the
  container. Not one masking assertion changed.
- **BLOCKER 2 (the branch state).** `main` is merged in (`bcf2d3b5`), not rebased.
- **SHOULD-FIX 1.** `docs/OPERATIONS.md`'s "What the setting is for now (`SEC-50`)" says the
  assistant's own tool is a SIGNED-IN PERSON's call, gated as that person, and that a
  signed-in person of another tenant is refused with the gate's own 404 -- not with the 422,
  which is the job-with-no-user refusal. The round 1 handoff's "NIT 2" paragraph says the
  same, and is corrected.
- **SHOULD-FIX 2.** `rust/crates/lakehouse-api/src/routes/ai/tools/gold.rs`'s new test
  `the_tool_gates_the_export_as_the_signed_in_person`: a real per-test Postgres (the four
  seeded tenants), a `CATALOG_TENANT_ID` set, a bare `wiremock` engine, and a person of
  "Meridian Retail" (`sari@meridian.example`, who is not of the catalog's tenant) holding
  `gold:export`. The tool refuses with the gate's own 404 sentence, and the engine is asked
  nothing.
- **NIT 1 (the B2 comment).** `.github/workflows/ci.yml`'s `gold-export` job. The comment now
  says what is true: the shared catalog belongs to a tenant and every multi-tenant deployment
  must name it; THIS job's harness signs in as the `*:*` bootstrap admin
  (`ops/gold_export/gold_export_test.py`'s `step_login`), so its own export passes as a
  `Caller` with every table in reach and does not lean on the setting -- but an exporter that
  is not an unrestricted admin does, and the setting belongs there either way.
- **NIT 2 (`OPERATIONS.md`).** The exception to "invisible" is stated, and
  `PUT /api/pipelines/{id}/tenant`'s `identity:write` permission is named. Both checked
  against the code: `routes::pipelines::list`'s `all_tenants` and
  `lakehouse_store::pipelines::list_pipelines`'s `WHERE ($2 OR ...)`, and `POLICY_TABLE`'s
  `("PUT", "/api/pipelines/{id}/tenant", Policy::RequiresPermission("identity:write"))`.
- **NIT 3 (`routes/gold.rs`).** `export`'s `# Errors` lists the gate's 404 beside the 422.
- **NIT 4 (`tests/pipeline_enforce.rs`).** The doc of
  `a_pipeline_authored_by_an_unrestricted_principal_is_not_refused_for_its_tenant` now states
  the case the test checks (a pipeline with no tenant of its own), names the case it does not
  check (a pipeline in a tenant the author is not a member of), and drops the "this is the
  shape the G7 harness creates" claim: the G7 gate joins its account to `meridian-group`
  BEFORE `POST /api/pipelines`, and the create route stores the creator's active tenant, so
  its pipeline is stored WITH a tenant.
- **R5, amended.** `routes/query.rs`'s `catalog_tenant_reach` refuses with
  `ApiError::Unavailable(COULD_NOT_VERIFY_TENANT_COUNT)` -- a 503, "could not verify tenant
  count" -- when the count is `None`, not with D6's 422 that tells the operator to set a
  setting which may already be set. The sentence is one constant
  (`routes::catalog::COULD_NOT_VERIFY_TENANT_COUNT`) shared by the three sites that refuse in
  this state. The 422 with D6's sentence is unchanged for the case it is for: a deployment
  that CAN count its tenants, holds several, and names no catalog tenant.

#### Not verified, with the reason

- **The full workspace test and the G-gates were NOT run locally.** Under the owner's rule of
  2026-10-30, CI is the final check: the run on the pushed head `95299d76` (workflow run
  `41072004930`) is what confirms the G3a job (`Rust · G3a Dagster + dlt Bronze ingest`) and
  the full workspace (`Rust · test (stable)`). What IS verified here is every crate this
  round touched: `lakehouse-api`'s fmt, clippy and the tests listed above.
- **`docker compose up` on the affected profile was NOT run.** `docker-compose.yml` changed in
  B1 (one environment variable on the `bld1-test-runner` service), and `config --quiet`
  passes. The profile proof is the G3a job on the PR.
- `bun run typecheck && bun run lint && bun run test` -- **not run.** The branch touches no
  TypeScript file.
- `(cd dagster && python -m pytest dispar_orchestrate -q)` -- **not run.** No Dagster file
  changed; the CI job `Dagster · unit tests` is green on this head.
- `docs/core/BACKLOG.md` -- **not touched** (the reviewer sets the status, the PR and the
  shipped date at merge).
- Slice 2's mutation checks and the two fingerprint reads of S8 -- out of this slice.

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

### Review round 2 (planner, 2026-10-30, at `ba554bbb`)

An independent read-only pass checked every round 1 finding against the code. All of them are
fixed:
- R1: `*:*` passes `AsTenant` (`query.rs:339`).
- B2: the setting is in the right CI job.
- R2: 422 only on the enforce route.
- R3: the 409 comes before the gate, with `*:*` exempt.
- R4: person versus service is decided by `PrincipalId::Service`.
- R5, S5, S6.
- `1803ba76` is a test-isolation change only.
CI ran on the PR: G7 itself now **passes**, and the Gold export job passes.

#### BLOCKER

1. **The CI check `Rust · G3a Dagster + dlt Bronze ingest` is red at `ba554bbb`**, though not
   because of G7. The same job's `ops/g7/bld1_pipeline_author_test.py` fails: `[bld-1] FAILED:
   the masked-author run FAILED instead of writing masked rows`.
   - **Cause.** The harness picks a regular Analyst as the author and scopes the pipeline to
     that author's tenant. The job's `.env` names no `CATALOG_TENANT_ID`, and `0002` seeds four
     tenants, so under D2/D5 the gate refuses the shared `bronze.g3a_orders`. That is the signed
     behaviour, exactly as G8 was in #244.
   - **Fix (decided):**
     - add `CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001` to that job's CI-only `.env`,
       with a comment;
     - in the harness, choose an author who belongs to THAT tenant and scope the pipeline to it.
       `_user_by_role` and `_tenant_shared_with` prefer the catalog's tenant (read the setting
       from the environment, the way the harness reads its other settings) and fail with a clear
       message if no holder of the role belongs to it.
   - Every masking assertion stays exactly as it is.
2. **The branch is behind `main`**, and protection requires it to be up to date. Merge `main`
   in; do not rebase.

#### SHOULD-FIX

1. `docs/OPERATIONS.md:~1592-1600` lists "the assistant's own tool" among callers with no user,
   which act as the catalog's tenant. The code gates the tool as the signed-in person
   (`routes/ai/tools/gold.rs:~115`, `Caller`). The text also implies every Gold export in a
   multi-tenant deployment without the setting gets the 422; a signed-in non-`*:*` person gets
   the 404 table sentence instead. Make both true. Fix the round 1 handoff sentence that says
   the same.
2. Nothing proves the assistant's gold tool uses `Caller`: if it were switched to
   `CatalogTenant`, every test would still pass. Add a test that calls the tool as a person of
   another tenant (with `gold:export`, a catalog tenant set) and is refused, and run a mutation
   check that swaps the actor.

#### NIT

- The B2 comment (`ci.yml:~2696-2701`) says the Gold export "is a job with no user". This job's
  harness signs in as the `*:*` bootstrap admin, so it passes as `Caller` (`All`). Keep the
  setting, but say why truthfully: it names the catalog's tenant as every multi-tenant
  deployment must, and a non-admin exporter would need it (`AGENTS.md` rule 1).
- `OPERATIONS.md:~1634`: "an unassigned pipeline is invisible" is half true. A `*:*` admin sees
  every pipeline. Say so, and say `PUT /api/pipelines/{id}/tenant` needs `identity:write`.
- `routes/gold.rs:~361-370`: the route's `# Errors` must list the 404 a person of another tenant
  now gets.
- `tests/pipeline_enforce.rs:~749`: the doc claims a case the test does not check. Make it match
  the test.
- **R5, amended (planner):** with no pool, `CatalogTenant` refuses with `SEC-20`'s own sentence
  for that state, `ApiError::Unavailable("could not verify tenant count")`
  (`routes/catalog.rs:179`), not D6's. D6 tells the operator to set a setting that may already
  be set. Update its unit test.

**Open: 2 BLOCKERs** (both the CI setup and the branch state, not the gate's logic). One fix
round: the BLOCKERs, then SHOULD-FIX 1-2, then the NITs. Cite `PR #281 review round 2 B<n>/S<n>`.
Under the owner's rule of 2026-10-30, CI is the final check: run fmt, the clippy and tests of
`lakehouse-api` for what you touched, and the harness file's own Python checks if any. Push, and
CI runs the whole workspace and the G-gates. Then write a short round 2 handoff.
