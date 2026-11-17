# SEC-50 slice 2: dashboards, embeds, public links, alerts and reports pass the tenant gate

Backlog `SEC-50` (Task, Administration & Security, P2). Slice 1 merged as #281. Slice 2 was
**waiting for #250** (`BI-19` parts B–E), which merged on 2026-11-02 as `ff48aa5b`.
Base: `main` at `9c066426`. Every `file:line` below was read at that commit. Paths are under
`rust/crates/lakehouse-api/src/` unless they say otherwise.

Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does not change
a decision below. If the code disagrees with this plan, stop and report it.

This is the **slice 2 plan check** the original plan asked for
(`docs/superpowers/plans/2026-10-30-sec-50-tenant-gate-everywhere.md`, section 4: "anchors
re-read then"). Section 1 keeps that plan's decisions D1–D8 and slice 1's review amendments
R1–R5. Where the code has moved, the change is written down as a mismatch (M1–M13) with what
this plan does about it. Nothing is changed without saying so.

## 0. What is wrong, with evidence

Slice 1 lifted `SEC-20`'s gate into `routes::query::gate_statement` (`routes/query.rs:327`)
with three actors (`GateActor`, `:268`). Today only the query path (`:181-197`), authored
pipelines (`routes/authored_pipelines.rs:1558`) and the Gold export (`gold_export.rs:696`,
`routes/gold.rs:322`) call it. Every other statement still goes to
`policy_engine::rewrite_sql_for_roles` with no gate:

| Path | Rewrite site at `9c066426` | Who reaches it |
| --- | --- | --- |
| Every tile, layer, table count, bin read, preview row, metric | `routes/support.rs:312` (`try_run_spec_sql`), wrapped by `run_spec_sql` `:227` | `dashboard::get` → `payload_for` (`dashboard.rs:232`) → `get_body` (`:339`, tiles at `:537`, `:584`); `specs_preview` (`:1047`, `:1098`); `layers::RoleRunner` (`layers.rs:92-110`); `support.rs:823`, `:913`, `:947`, `:1060`; `dashboard_sources.rs:478`; `metrics::value_query` (`metrics.rs:718`, `:781`); the assistant's metric tool (`ai/tools/metrics.rs:222`); embeds and public links (`embed.rs:1490` in `render_board_payload` `:1378`) |
| Records (drill and raw-table pages) | `dashboard.rs:3333` (`run_records` `:3320`) | `GET /api/dashboard/records` (`records` `:2744` → `records_for_roles` `:3208`) |
| Filter values | `dashboard.rs:4434` (`values_with_limit` `:4372`) | `GET /api/dashboard/values` (`values` `:4176`); `POST /api/embed/values` (`embed.rs:505`, `:556`) |
| Chart and table downloads | `dashboard.rs:4039` (`build_chart_file` `:4010`) | `chart_export` `:3872`, `table_export` `:3891`; the report runner's attachments (`report_runner.rs:982`) |
| SQL sources, save and preview | `dashboard_sources.rs:145` (`check_and_rewrite` `:137`) | `save` `:253`, `preview` `:407` / `preview_for_roles` `:434` |
| Alerts (mart, metric, digest rules) | `alerts.rs:869` (`impl SqlGate for ApiSqlGate` `:866`) | `/api/alerts/run` (`:1125`, gate built `:1157`); the assistant's `run_alert_rule` (`ai/tools/alerts.rs:140`, gate `:183`) |
| **New in #250:** chart alerts | through `payload_for` | `chart_alerts::read` (`chart_alerts.rs:96`, `:150`), as the person who set the alert |
| **New in #250:** chart image (`format=png`), PDF | through `payload_for` via the render token | `pdf_export::export_chart_png` (`pdf_export.rs:453`), `make_png` `:236`, `make_pdf` `:195` → `render::data` (`render.rs:69`, `:122`) |
| Report emptiness check | through `payload_for` | `report_runner.rs:764` |

So, on a deployment with several tenants, a member of a tenant that does not own the shared
catalog reads `serving.*` rows through every dashboard path, although Query Studio refuses
the same statement with `SEC-20`'s 404 (`table_not_available`, `query.rs:495`). The backlog
row and `PRODUCT.md:153` ("Dashboards, embeds, public links and alerts are not yet (slice 2)")
say the same.

Checked and **not** a gated path (nothing they run goes through the role rewrite):
`dashboard_access.rs` (permission rows), `dashboard_comments.rs` (comments and mention mail),
`dashboard_file.rs` (a dashboard as a file; it lists `serving` table and column names, see
M9), `public_links.rs` itself (link rows), `embed.rs` map files.

## 1. Decisions

### Kept from the original plan and slice 1 (do not change)

- **D1.** One gate, called from every site; nobody writes a second copy of the reach logic.
- **D2.** A signed-in user acts as `X-Tenant`, else their first tenant. With several tenants only
  `CATALOG_TENANT_ID`'s tenant reads the shared catalog. `*:*` passes. One tenant: as before.
  The owner accepted that, with the setting unset and the four seeded tenants, regular users
  see no dashboard data until an operator sets it.
- **D3.** A job with no user acts as the catalog's tenant (`GateActor::CatalogTenant`): `All`
  with one tenant, the catalog tenant when set, else D6's 422. Covers alerts, embeds.
- **D4.** A public link acts as the tenant that made it: new nullable
  `dashboard_public_link.tenant_id`; NULL acts as the creator's first tenant at view time;
  a creator who no longer exists fails closed. (See M7 for the one-tenant case.)
- **D6.** The only new sentences are `SEC-20`'s `table_not_available` 404 (unchanged) and D6's
  `JOB_NEEDS_CATALOG_TENANT` (`query.rs:464`). R5 (amended): no pool → `SEC-20`'s
  `COULD_NOT_VERIFY_TENANT_COUNT` 503.
- **D7.** The boot warning names what is gated.
- **R1.** `*:*` passes `AsTenant` in every case (`query.rs:368-383`).
- **R4.** A signed-in person is gated as that person (`Caller`), even when the call comes
  through the assistant; only a service principal or a run token is `CatalogTenant`.
- **S6** (original section 4). The report runner acts as the subscription owner:
  `Caller { owner, &HeaderMap::new() }`, which means the owner's first tenant.

### Mismatches found at `9c066426`, and what this plan does

- **M1. The sites take `ch`, not `state`.** `gate_statement` needs `&AppState` (the pool,
  `config.catalog_tenant_id`). `try_run_spec_sql`, `run_records`, `records_for_roles`,
  `values_with_limit`, `check_and_rewrite`, `get_body`, `render_board_payload`, `RoleRunner`
  and `ApiSqlGate` take `&ChClient` (and `ApiSqlGate` a pool), never the state.
  *(planner default)* Add one carrier in `routes/query.rs`:
  `pub(crate) struct StatementGate<'a>` holding `&'a AppState` and a `GateActor<'a>`, with
  `pub(crate) async fn check(&self, sql: &str) -> Result<(), ApiError>` (ClickHouse dialect;
  every slice 2 site is ClickHouse). Every site above takes `&StatementGate<'_>` and calls
  `check` before its `rewrite_sql_for_roles`. `gate_statement` stays the one-shot entry point
  and is implemented with the same internals, so there is still one copy (D1).
- **M2. Reach is resolved once per request, not once per tile.** `reach_for_actor` reads
  Postgres two or three times for a `Caller` (`catalog_tenant_refusal`, the tenant count;
  `query.rs:530-560`, `:688`). A board's tiles run side by side (`tile_pool`), so a gate per
  tile is that many reads per tile. *(planner default)* `StatementGate` resolves the reach
  lazily on its first `check` and keeps it for the request (`tokio::sync::OnceCell`; a
  failure is kept too, so every tile gets the same refusal). Split `refuse_unreachable_tables`
  (`:798`) into "reach" and "check tables against a reach"; both `gate_statement` and
  `StatementGate::check` use the split halves. Claims (`claim_on`, `:757`) are still read per
  statement; `serving.*` names have no claim lookup (`raw_table_name`, `:733`).
  Not measured; T10 measures it (principle 5).
- **M3. A gate refusal must fit a tile's error slot.** `SpecRunFailure::Refused` holds a
  `&'static str` (`support.rs:248-262`); the `SEC-20` sentence names the table, so it is a
  `String`. *(planner default)* Add `SpecRunFailure::Gated(ApiError)`. `run_spec_sql` turns it
  into the tile's `{"error": <the gate's own sentence>}`, like `Refused`; callers that classify
  (`support.rs:823` and the others) return the `ApiError` as it is. The gate runs **before**
  the rewrite, so a refused tile never reaches the result cache lookup (`support.rs:388-400`)
  or the engine.
- **M4. Three new callers read as a person who is not the requester's session (#250).**
  The original plan put all alerts under `CatalogTenant` (S5). Chart alerts did not exist then;
  `BI-19` C4 reads them **as the person who set the alert** (`chart_alerts.rs:8-15`).
  *(planner default)* `payload_for` gains `headers: &HeaderMap` and always gates as
  `Caller { principal, headers }`:
  - `GET /api/dashboard`: the request's headers;
  - `render::data` (PDF and chart PNG): the token's person, `HeaderMap::new()`;
  - `report_runner.rs:764` and `:982`: the owner, `HeaderMap::new()` (S6);
  - `chart_alerts::read`: the rule's owner, `HeaderMap::new()`.
  A refusal in `chart_alerts::read` is a tile `error`, which it already turns into
  `failed(UNREADABLE)` (`chart_alerts.rs:163-165`). Keep that; do not add a sentence.
- **M5. The render token carries no tenant.** A person who works under `X-Tenant: B` and
  exports a PDF or a chart image gets it as their **first** tenant. Not a leak (they belong to
  both), but the file can differ from the screen. Written down in the feature page's limits;
  carrying the acting tenant in `RenderClaims` is out of scope (section 4).
- **M6. The assistant's `run_alert_rule` is a person's call.** It builds `ApiSqlGate` with no
  principal (`ai/tools/alerts.rs:140-186`). Under R4 a person is gated as the person.
  *(planner default)* The tool takes the caller's principal (the dispatcher already passes one
  to `metrics`, `ai/tools/metrics.rs:222`) and gates as `Caller { principal, &HeaderMap::new() }`;
  `/api/alerts/run` (a run token or a service principal, `check_run_token`) gates as
  `CatalogTenant`.
- **M7. Rule authoring opens what D3 closes.** A person of a non-catalog tenant who holds
  `alert:write` can save a mart or metric rule over `serving.*`, and the scheduled run, acting
  as the catalog's tenant, then delivers the catalog's number to that person's target.
  *(planner default; owner to confirm)* Gate the rule's statement as the **saving person**
  (`Caller`) on create and update (`authorise_rule_create` `alerts.rs:315`,
  `authorise_rule_update` `:289`), with `SEC-20`'s 404 sentence. Chart rules are already read
  as their owner (M4) and need nothing more.
- **M8. A public link whose creator is gone, in a one-tenant deployment.** `copy_legacy`
  (`lakehouse-store/src/public_links.rs:329`) copies pre-`BI-19` links with `created_by` NULL,
  and a link made by a service identity stores NULL too (`public_links.rs:654-658`). D4 says
  "fails closed", which in a **one-tenant** deployment would break links that work today,
  against D2 ("a deployment with one tenant behaves as before"). *(planner default)* Add a
  fourth actor, `GateActor::Ownerless`: `All` when the deployment holds one tenant (the same
  count `CatalogTenant` reads), `NoTenant` otherwise; the no-pool rule is R5's. The view acts as:
  - the link's `tenant_id` set: `AsTenant { creator, Some(tenant_id) }` (a creator who left
    that tenant gets the 404, D5's rule);
  - `tenant_id` NULL, creator loaded: `Caller { creator, &HeaderMap::new() }` (their first
    tenant, D4's wording);
  - creator NULL, deleted or disabled (`render::principal_of` returns `None`): `Ownerless`.
- **M9. `/api/dashboard/values` with no board (S2).** It reads `system.columns` unrewritten
  (`dashboard.rs:4407-4428`) and then runs one statement over `serving.<table>` for each table.
  The original plan said "gate it as a listing". *(planner default)* Gate the **final**
  statement with `StatementGate::check`: it names every `serving.*` table it reads, and the
  table names from `system.columns` never reach the response when it is refused. No separate
  listing gate. Table and column **names** of the shared catalog are still listed to every
  tenant by `/api/dashboard/fields` (`:2220`) and the import preview
  (`dashboard_file.rs:179`, `readable_tables`). That is metadata, not rows, and out of scope;
  it gets its own backlog item (proposed `SEC-58`).
- **M10. The migration number.** Highest on `main` is `0117_outside_table_read_day.sql`; no
  open PR adds a migration (checked #264, #335, #341 on 2026-11-02). Use **`0118`**, and check
  again when the migration is written.
- **M11. Fixtures that will flip.** The integration tests run on migration `0002`'s four
  tenants. Those that read tiles as a user without `*:*` (expected: `tests/render_data.rs`,
  `report_runner.rs`, `chart_alerts.rs`, `chart_png.rs`, `embed_tokens.rs`, `public_links.rs`,
  `dashboard_access_routes.rs`, `report_routes.rs`) will start to see refusals. Fix the
  **fixture**, as slice 1 did for `column_lineage_pipeline.rs` (`abda61c4`): name
  `CATALOG_TENANT_ID` and put the reader in that tenant. Never weaken an assertion (rule 2);
  name every fixture change in the handoff.
- **M12. S8, the two fingerprint reads** (slice 1 review round 1). At `9c066426`:
  `publication_body`'s fingerprint is `gold.rs:854` (the function is `:839`), and
  `pipeline_lag::clickhouse_fingerprint_token` is `pipeline_lag.rs:275`. Both answer a boolean
  or a token, never rows. *(planner default)* Gate `publication_body` as the route's caller
  (it already has the principal for `can_edit`); leave `pipeline_lag` ungated with a comment at
  the site saying why (orchestrator-only route, an opaque token, the tables are the
  pipeline's own inputs).
- **M13. The assistant.** The dashboard tools that read data go through `value_query` (M1) and
  `payload_for` is not called from `routes/ai/`. No tool schema changes, so
  `tests/fixtures/tool_schemas.json` is not regenerated. The AI-15 eval
  (`ops/ai_eval/`) runs against a live stack; it must run with `CATALOG_TENANT_ID` set or as a
  `*:*` user, and the handoff says which.

### Consequence the owner already accepted (restated for QA)

With several tenants and `CATALOG_TENANT_ID` unset: every tile of every signed-in non-`*:*`
user shows `SEC-20`'s sentence; embeds and the scheduled alerts are refused with D6's 422 per
tile or rule; public links show the sentence. That is D2/D3, not a bug.

## 2. Anchors (verified at `9c066426`)

| What | Where |
| --- | --- |
| The gate | `routes/query.rs`: `GateActor` `:268`, `gate_statement` `:327`, `reach_for_actor` `:359`, `catalog_tenant_reach` `:437`, `JOB_NEEDS_CATALOG_TENANT` `:464`, `table_not_available` `:495`, `LISTING_NOT_AVAILABLE` `:512`, `resolve_reach` `:530`, `reach_from` `:577`, `deployment_tenant_count` `:688`, `claim_on` `:757`, `refuse_unreachable_tables` `:798`; unit tests `mod sec50_gate_tests` `:5282` |
| Tile runner | `routes/support.rs`: `run_spec_sql` `:227`, `SpecRunFailure` `:248`, `try_run_spec_sql` `:302` (rewrite `:312`, cache `:388-400`), `annotate_bins` `:793`, `add_bins` `:808`, `preview_bin_reads` `:850`, `annotate_table` `:893`, `preview_rows` `:1037`; tests `mod run_spec_sql_enforcement` `:1226` |
| Layers | `routes/layers.rs`: `RoleRunner` `:92`, its `run` `:99`; built at `dashboard.rs:498`, `:1125`, `embed.rs:1427` |
| Dashboard | `routes/dashboard.rs`: `get` `:197`, `payload_for` `:232`, `get_body` `:339`, `specs_preview` `:1047`, `fields` `:2220`, `records` `:2744`, `records_for_roles` `:3208`, `run_records` `:3320`, `chart_export` `:3872`, `table_export` `:3891`, `build_chart_file` `:4010`, `values` `:4176`, `values_with_limit` `:4372`; test modules `records_enforcement` `:5522`, `values_enforcement` `:5794`, `records_pages` `:6832`, and `get_body` callers in `layout_tabs.rs:120`, `:654`, `organise_paths.rs:200`, `:374` |
| SQL sources | `routes/dashboard_sources.rs`: `checked_and_rewritten` `:120`, `check_and_rewrite` `:137` (rewrite `:145`), `save` `:253`, `preview` `:407`, `preview_for_roles` `:434`, `:478` |
| Metrics | `routes/metrics.rs`: `value_query` `:718` (`run_spec_sql` `:781`); `routes/ai/tools/metrics.rs:222` |
| Embeds, public links | `routes/embed.rs`: `EMBED_VIEWER_ROLE` `:44`, `data` `:372`, `values` `:505` (`:556`), `public_dashboard` `:948`, `render_board_payload` `:1378` (`run_spec_sql` `:1490`); `routes/public_links.rs`: `resolve` `:195`, `put_link` `:605` (`created_by` `:654`); store `lakehouse-store/src/public_links.rs`: `PublicLink` `:41` (`created_by` `:53`), `COLUMNS` `:101`, `replace` `:166`, `update` `:216`, `copy_legacy` `:329`; migration `rust/migrations/0098_public_links.sql` |
| Render, PDF, PNG | `routes/render.rs`: `data` `:69`, `payload_for` `:122`, `principal_of` `:194`; `routes/pdf_export.rs`: `make_pdf` `:195`, `make_png` `:236`, `export_chart_png` `:453` |
| Reports | `routes/report_runner.rs`: `payload_for` `:764`, `build_chart_file` `:982` |
| Chart alerts | `routes/chart_alerts.rs`: `read` `:96` (`principal_of` `:102`, `payload_for` `:150`), `evaluate` `:194` |
| Alerts | `routes/alerts.rs`: `authorise_rule_update` `:289`, `authorise_rule_create` `:315`, `ApiSqlGate` `:808`, `impl SqlGate` `:866` (rewrite `:869`), `run` `:1125` (gate `:1157`), tests `:2254`, `:2277`; `routes/ai/tools/alerts.rs`: `run_alert_rule` `:140` (gate `:183`); `lakehouse-alerts/src/lib.rs` `gate.gate` at `:1799`, `:1881`, `:1993` |
| S8 | `routes/gold.rs`: `publication_body` `:839` (fingerprint `:854`); `pipeline_lag.rs`: `clickhouse_fingerprint_token` `:275` (called `:218`) |
| Boot warning | `main.rs`: `CATALOG_TENANT_UNCONFIGURED_WARNING` `:1242`, `warn_if_catalog_tenant_unconfigured` `:1273`, tests `:1297`, `:1311` |
| Operator text | `.env.example:596-606`; `docker-compose.yml:702`, `:2882`; `docs/OPERATIONS.md:1697` ("What the setting is for now"), `:1718`; feature page `docs/core/features/queries-scoped-to-the-tenant.md:39-41`, `:69-79` |
| Test helpers to reuse | `tests/query_tenant_scope.rs`: `OWNER_TENANT` `:52`, `OTHER_TENANT` `:54`, `spin_up_with_clickhouse` `:72`, `mount_engine` `:81`, `sent_statements` `:112`, `engine_requests` `:123`, `belong_to` `:155`; `tests/pipeline_enforce.rs` (bare `wiremock` server, `1803ba76`) |
| Bench | `ops/bench/dashboard_open_bench.py` (`BI-4`) |

## 3. Tasks (one commit each, in order)

Rust work is batched (`AGENTS.md`, "Keep build time down"). Per commit: `cargo fmt --check`,
`cargo clippy -p <crate> --all-targets -- -D warnings`, and the scoped tests named in *Accept*.
Every cargo call through `/home/shiro/rantai/bin/cargo-locked`. **Push after every commit.**
Cite `SEC-50 slice 2 M<n>` (or `D<n>`) at each site and in the commit body.

**T1. The gate carrier (`routes/query.rs`).** M1, M2, M8.
- `StatementGate<'a> { state, actor, reach: OnceCell<…> }` with `new(state, actor)` and
  `check(&self, sql)`. `ApiError` is not `Clone` (`lakehouse-core/src/error.rs:22`, `derive(Debug,
  Error)`), so the cell keeps a refusal as its variant and its fixed sentence and rebuilds the
  `ApiError` for each `check`; it never stores upstream text.
- Split `refuse_unreachable_tables` into resolving the reach and checking `GateTargets`
  against a reach; `gate_statement` keeps its signature and behaviour.
- `GateActor::Ownerless` (M8), with its doc comment and the `SEC-50 slice 2 M8` tag.
- A `#[cfg(test)]` constructor for an unrestricted gate (a `*:*` principal, so `All` without a
  pool) for the unit-test modules that call the runner with a mock engine. It must not exist
  outside `cfg(test)`.
- *Accept:* `cargo test -p lakehouse-api sec50_gate_tests` (existing tests unchanged, plus
  new: `Ownerless` with one tenant → `All`, several → `NoTenant`, no pool → the R5 503; the
  reach is read once for two `check` calls — count the Postgres reads or the `OnceCell`
  state; a refused first check refuses the second the same way) and
  `cargo test -p lakehouse-api --test query_tenant_scope` (unchanged).

**T2. The tile runner and every caller (`support.rs`, `layers.rs`, `dashboard.rs`,
`metrics.rs`, `ai/tools/metrics.rs`, `embed.rs`, `dashboard_sources.rs:478`, and the test
modules).** M1, M3, M4 (route half). S1 for tiles.
- `try_run_spec_sql` and `run_spec_sql` take `&StatementGate<'_>`; `check` runs after the
  empty-SQL return and before the rewrite. `SpecRunFailure::Gated(ApiError)` (M3).
- `RoleRunner` gains the gate. `get_body` takes it; `payload_for` takes `headers` and builds
  `Caller` (M4); `dashboard::get` takes `headers: HeaderMap`. `specs_preview` likewise.
- `value_query` builds `Caller { principal, &HeaderMap::new() }` (the assistant sends no
  `X-Tenant`, slice 1 S3); the metrics route passes its request's headers if it has them,
  else the same.
- Embeds compile with `CatalogTenant` here (their tests come in T5).
- Callers of `payload_for` outside `dashboard.rs` (`render.rs:122`, `report_runner.rs:764`,
  `chart_alerts.rs:150`) pass `&HeaderMap::new()` now; their tests come in T4.
- *Accept:* `cargo test -p lakehouse-api routes::support`, `routes::dashboard`,
  `routes::layers`, `routes::metrics`, `routes::dashboard_sources` pass. New HTTP tests in a
  new `tests/dashboard_tenant_scope.rs` (reuse `query_tenant_scope.rs`'s helpers): with
  several tenants and `CATALOG_TENANT_ID` set, `GET /api/dashboard` as a member of
  `OTHER_TENANT` answers 200 with every data tile's `error` equal to `SEC-20`'s sentence and
  **no statement reaches the engine** (`engine_requests`); as a member of `OWNER_TENANT` → rows;
  `*:*` → rows; one tenant → rows. **Cache:** the owner reads the board first (the tile is
  remembered), then the other tenant's member reads it and is refused, not served from memory.

**T3. The other signed-in sites (`dashboard.rs`, `dashboard_sources.rs`).** S1, S2 (M9).
- `run_records`/`records_for_roles`, `values_with_limit`/`values_for_roles`,
  `build_chart_file` (+ `chart_export`, `table_export`), `check_and_rewrite` (save and
  preview) take the gate; their handlers take `headers` and gate as `Caller`. The values
  no-board path gates the final statement (M9). The report runner's `build_chart_file` call
  passes `HeaderMap::new()` (S6).
- *Accept:* `cargo test -p lakehouse-api routes::dashboard` and `routes::dashboard_sources`;
  in `tests/dashboard_tenant_scope.rs`, the three-way matrix (other tenant refused with
  nothing at the engine, owner tenant served, `*:*` served) for `GET /api/dashboard/records`,
  `GET /api/dashboard/values` (with and without `board`), `GET /api/dashboard/chart-export`
  (CSV), `GET /api/dashboard/table-export`, `POST /api/dashboard/sources/preview` and
  `POST /api/dashboard/sources`.

**T4. Delegated readers: render (PDF, chart PNG), reports, chart alerts.** M4, M5, S6.
- No new code beyond T2/T3 unless a site still builds its own rewrite; this commit is the
  tests and the M11 fixture fixes for these files.
- *Accept:* `cargo test -p lakehouse-api --test render_data --test report_runner
  --test report_routes --test chart_alerts --test chart_png`. New cases: a render token for a
  person of `OTHER_TENANT` → the tile errors with the sentence; a report owned by such a person
  is refused per tile (and its attachment is left out with the existing "left out" line, not a
  new sentence); a chart alert set by such a person ends `failed(UNREADABLE)` and sends nothing.

**T5. Embeds (`embed.rs`).** S3, D3.
- `data` and `values` gate as `CatalogTenant`; `render_board_payload` takes the gate from its
  caller (embed: `CatalogTenant`; public link: T6).
- *Accept:* `cargo test -p lakehouse-api --test embed_tokens`; new cases: several tenants and no
  `CATALOG_TENANT_ID` → each data tile's `error` is D6's sentence and nothing reaches the
  engine; the setting set → rows; one tenant → rows; `POST /api/embed/values` the same three.

**T6. Public links: migration, store, creation, view.** S4, D4, M8, M10.
- `rust/migrations/0118_public_link_tenant.sql` (number re-checked): `ALTER TABLE
  dashboard_public_link ADD COLUMN IF NOT EXISTS tenant_id UUID NULL REFERENCES tenant (id)
  ON DELETE SET NULL;` with a why-header (D4, M8: NULL means "the creator's first tenant at
  view time", no backfill, and why `SET NULL` then falls back to the creator).
- Store: `PublicLink.tenant_id`, in `COLUMNS`; `replace` takes it; `update` never touches it;
  `copy_legacy` leaves it NULL. `lakehouse-store/tests/public_links.rs` covers the round trip.
- `put_link`: the acting tenant = `tenant_scope::resolve(principal, &headers)` (add
  `headers` to the handler); a service identity records NULL.
- `public_dashboard` (and `public_map_geojson` only if it runs a statement; it does not at
  `9c066426`): build the actor per M8, loading the creator with `render::principal_of`.
- *Accept:* `cargo test -p lakehouse-store --test public_links` and
  `cargo test -p lakehouse-api --test public_links`; new cases: a link made by a member of
  `OTHER_TENANT` (setting set) → tiles refused; by a member of `OWNER_TENANT` → rows; the
  creator later removed from that tenant → refused; a legacy link (`created_by` NULL) in a
  one-tenant deployment → rows, in a several-tenant one → refused; a NULL-tenant link whose
  creator is in `OWNER_TENANT` → rows.

**T7. Alerts (`alerts.rs`, `ai/tools/alerts.rs`).** S5, M6, M7.
- `ApiSqlGate` holds `&AppState` and a `GateActor` (drop the separate `pg`/`ch` fields, or keep
  them derived from the state). `gate()` calls `StatementGate::check` first; a refusal returns
  the gate's fixed sentence as the `Err(String)`, so `lakehouse-alerts` records the rule as
  failed and goes on (`lib.rs:1799`, `:1881`), and a digest tile is skipped (`:1993`).
- `/api/alerts/run`: `CatalogTenant`. `run_alert_rule`: `Caller { principal, &HeaderMap::new() }`
  (M6). Rule create/update for mart and metric rules: gate the rule's statement as the saving
  person (M7); build the statement with the same `lakehouse-alerts` builder the run uses, do not
  write a second one (rule 4); if the crate does not expose it, stop and report.
- *Accept:* `cargo test -p lakehouse-api routes::alerts` and `routes::ai::tools::alerts`;
  `cargo test -p lakehouse-alerts` (unchanged). New cases: several tenants, no setting → a mart
  rule's run result is failed with D6's sentence and the next rule still runs; setting set →
  value read; `run_alert_rule` by a member of `OTHER_TENANT` → refused; saving a mart rule as
  a member of `OTHER_TENANT` → 404 with `SEC-20`'s sentence and no `INSERT INTO
  console.alert_rule` (the existing harness at `alerts.rs:2254` counts inserts).

**T8. S8, the fingerprint reads (`gold.rs`, `pipeline_lag.rs`).** M12.
- `publication_body`: gate the fingerprint as the route's caller (`gold_export_gate_actor`,
  slice 1 R4, so a service principal is `CatalogTenant`). `clickhouse_fingerprint_token`: a
  comment at the site stating why it is not gated.
- *Accept:* `cargo test -p lakehouse-api routes::gold` and
  `cargo test -p lakehouse-api --test gold_publication`; one new case: a member of
  `OTHER_TENANT` reading a publication gets the 404 and the engine sees no fingerprint read.

**T9. The boot warning, operator text and docs.** D7.
- `main.rs:1242`: name dashboards, embeds, public links, alerts, reports and chart images as
  gated; drop "gated in SEC-50's next slice". Update both tests at `:1297` and `:1311`; the
  second one's purpose (nothing named as refused that is still served) becomes "every path is
  named", and the commit says why the test changed (rule 2).
- `.env.example:596-606`, `docs/OPERATIONS.md:1697-1730`, the feature page
  (`queries-scoped-to-the-tenant.md`, "Limits" `:69-79` and `:39-41`): what is gated, the
  per-tile sentence a user sees, M5 (PDF and image act as the first tenant), M8, and M9's
  remaining metadata exposure with its backlog ID.
- The developer does not touch `BACKLOG.md` or `PRODUCT.md` (the planner does, at merge).
- *Accept:* `cargo test -p lakehouse-api --bin lakehouse-api boot_warning`; `git diff --stat`
  shows only the files named.

**T10. Proof (handoff only, no commit).**
- **Mutation checks** (original section 5): for each of T2 (tile runner), T3 (records, values,
  chart export, sources), T5 (embed), T6 (public link view), T7 (alert gate, rule save) and T8,
  remove the `check` call, run the named test, quote that it fails, restore.
- **Measure M2:** `ops/bench/dashboard_open_bench.py` on the same stack before and after, as a
  member of the catalog tenant; quote both numbers and the command. No target is invented here;
  if the difference is visible, the planner decides.

## 4. Out of scope

- Per-tenant `serving` marts; moving the `*:*` or one-tenant rules (original section 7).
- Carrying the acting tenant in the render token (M5). Proposed for `SEC-54`'s list.
- Shared catalog table and column **names** shown to every tenant (`/api/dashboard/fields`,
  import preview, `system.columns` reads): proposed `SEC-58` (M9).
- `dashboard_comments`, `dashboard_access`, `dashboard_file` beyond M9: they run no gated
  statement.
- `SEC-55`, `SEC-56`.
- The console (`src/`): no TypeScript change; a refused tile already shows its `error`.

## 5. Verification before handoff

`AGENTS.md`, the Rust line, on the final commit. Under the owner's rule of 2026-10-30
("blaze locally, CI is final"), the full workspace run is CI's; run locally every crate this
slice touched (`lakehouse-api`, `lakehouse-store`), fmt and clippy for the workspace, and say
*not verified* for anything else.

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p lakehouse-store && cargo test -p lakehouse-api
docker compose --profile '*' config --quiet
git status
```

The G-gates (`g3a-dagster`, `gold-export`, `g8-*`, `g6-alerts`) must stay green on the PR.
`g6-alerts` (`ops/g6/src17_alert_delivery_test.py`) only makes `connector_failure` and
`upload_failure` rules (`:336-385`), which run no gated statement, so it should not move; if any
gate goes red, read why before changing a harness, and quote the failure.

## 6. Handoff (developer appends below)

## 7. Review (planner appends below)


**Migration number and order (planner, 2026-11-02):** the public-link migration is `0120` (`0118` is SEC-57 and `0119` is AI-23). Owner to confirm: M7. Planner default: yes, saving a mart or metric rule is gated as the person saving it.

### Developer report

**T7, the M7 half (rule save): stopped and reported, as the plan says.** The rest of T7
(the gate on the run, D3 and M6) is implemented and pushed as `ea0e8703`.

The plan's T7 says, for rule create and update: "build the statement with the same
`lakehouse-alerts` builder the run uses, do not write a second one (rule 4); if the crate does
not expose it, stop and report." It does not expose it:

- `current_value` (`rust/crates/lakehouse-alerts/src/lib.rs:1779`) builds the mart rule's
  statement inline, `let sql = format!("SELECT {expr} AS v FROM serving.{mart_ident}");`
  (`:1798`), and is private.
- `metric_value` (`:1813`) builds a metric rule's statement from `lakehouse_bi::builder`
  inside itself (`:1861-1877`) and is private.
- `lakehouse-alerts` exports no builder: its `pub` items are the rule types, the store
  functions, the evaluators and the `SqlGate` trait (`:2117`); nothing returns a rule's
  statement.

So saving a mart or metric rule is **not** gated as the saving person, and the acceptance case
"saving a mart rule as a member of `OTHER_TENANT` → 404 with `SEC-20`'s sentence and no
`INSERT INTO console.alert_rule`" has no code to test and is **not implemented**. M7's hole
(a person of a non-catalog tenant who holds `alert:write` can save a rule over `serving.*` that
the scheduled run, acting as the catalog's tenant, then delivers) is therefore still open.

What would close it, for the planner to decide (I did not choose for the plan): either a
`pub fn mart_statement(mart, measure, agg) -> Result<String, AlertError>` (and the metric
equivalent) extracted from `current_value`/`metric_value` and called by both the run and the
save path — one builder, shared, no second copy — or a plan change that gates the save another
way. A second inline copy in `routes::alerts` is what the plan forbids, so I did not write one.

### Planner decision on the T7 developer report (2026-11-02, at `0ab03794`)

**Accepted, and the planner chooses the first option. M7 stays in this slice.**

- **D7a.** `lakehouse-alerts` exports the statement builders that the run already uses, extracted and not copied:
  - `pub fn mart_statement(..) -> Result<String, AlertError>`, from `current_value` (`lib.rs` ~1798);
  - `pub fn metric_statement(..) -> Result<String, AlertError>`, from `metric_value` (~1861–1877).

  `current_value` and `metric_value` then call them. The run's behaviour is unchanged, and the crate's existing tests must stay green.
- `routes::alerts` (create and update) and the assistant's `create_alert_rule` / `update_alert_rule` build the rule's statement with these same functions, and send it through the gate as the person saving it, exactly as the run does (D3). A refusal gives the 404 with `SEC-20`'s sentence, and no row is written.
- **Accept:**
  - the plan's T7 case: saving a mart rule as a member of `OTHER_TENANT` gets 404, and there is no `INSERT INTO console.alert_rule`;
  - the same case for a metric rule;
  - `cargo test -p lakehouse-alerts`, unchanged and green.
- Rule 4 holds: there is one builder, used by both the run and the save.

### Handoff (developer, 2026-11-02)

Branch `feat/sec-50-slice-2`, pushed after every commit. T1–T6 were already there
(`db7a337d` … `c114567c`); this machine did T7–T9. No PR was opened. Nothing was
force-pushed; the two pushes that raced the planner's `24b798c5` used
`git pull --rebase`.

Commits (this session, in order):

| Commit | What |
| --- | --- |
| `ea0e8703` | T7: the alert run asks the gate — `ApiSqlGate` holds the state and an actor, `gate()` checks before the rewrite; `/api/alerts/run` is `CatalogTenant` (D3), the assistant's `run_alert_rule` is the caller (M6/R4). |
| `0ab03794` | T7's `### Developer report`: M7 could not be implemented — `lakehouse-alerts` exposed no builder. |
| `f4c7d370` | T8: `publication_body` gates the fingerprint as the route's caller; `clickhouse_fingerprint_token` stays ungated with the reason at the site (M12). |
| `def8a64f` | T7 (planner's D7a): `mart_statement`, `metric_statement` and `statement_for_input` extracted and exported; `authorise_rule_create`/`authorise_rule_update` gate the rule's statement as the saving person (M7). |
| `3d8e5e2e` | T9: the boot warning names every gated path (D7); `.env.example`, `docs/OPERATIONS.md`, the feature page. |

**T7's M7 half, second attempt (D7a), for the record.** The report at `0ab03794` said the
crate did not expose the builders; the planner chose to export them, and the extraction is
larger than the two functions named, on purpose: `metric_statement(ch, name)` carries the
metric/source/catalog lookups too (`lib.rs` `metric_value`), because the save path would
otherwise have copied them, and `statement_for_input` carries the save's own
`normalize_input` for the same reason (a mart given as `serving.x`, or an omitted `agg`,
must resolve here exactly as at save and at run). One builder per kind, used by both the run
and the save; nothing is copied.

**Fixtures changed (`M11`), no assertion weakened:**

1. `routes::alerts::tests::silence_end_to_end::state_for` now names `CATALOG_TENANT_ID`: its
   rule reads `serving.route_test_mart`, and without the setting D6 refuses it, so the test
   would have been about the gate instead of the silence.
2. `routes::alerts::tests::webhook_targets`: the state now has a real pool (the test became
   `#[sqlx::test]`) and names `CATALOG_TENANT_ID`, because its rule reads
   `serving.mart_wisman` and the gate now runs before the webhook check it is about. The
   assertions are unchanged (400, the fixed message, zero inserts).

**API signatures that moved** (all in-crate, both paths updated): `alerts::create`,
`alerts::update` and `authorise_rule_create`/`authorise_rule_update` take `headers`;
`run_alert_rule` takes the principal; `publication` and `set_publication` take `HeaderMap`.
No tool schema changed, so `tests/fixtures/tool_schemas.json` is not regenerated (M13).

**Commands run, with counts (all on the final commit, `-j 2`, through
`~/rantai/bin/cargo-locked`):**

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean |
| `cargo clippy -p lakehouse-api -p lakehouse-alerts --all-targets -- -D warnings` | clean |
| `cargo test -p lakehouse-api routes::alerts` | 35 passed, 0 failed |
| `cargo test -p lakehouse-api routes::ai` | 446 passed, 1 ignored, 0 failed |
| `cargo test -p lakehouse-api routes::gold` | 20 passed, 0 failed |
| `cargo test -p lakehouse-api --test gold_publication` | 14 passed, 0 failed (real `ClickHouse` 26.8 testcontainer; container removed afterwards) |
| `cargo test -p lakehouse-alerts` | 126 passed, 1 ignored, 0 failed (unchanged) |
| `cargo test -p lakehouse-api` | 2 942 passed, 0 failed, 18 ignored in the lib target; every integration target green |
| `cargo test -p lakehouse-store` | 528 passed, 0 failed, across 20 targets |
| `cargo test -p lakehouse-api --bin lakehouse-api boot_warning` | 3 passed |
| `docker compose --profile '*' config --quiet` | exit 0, with throwaway `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY`, `CONNECTOR_SECRETS_KEY` |

**One red seen and re-run, not hidden.** The first full `cargo test -p lakehouse-api` ended
`2941 passed; 1 failed`: `routes::alerts::alert_run_tenant_scope::every_mart_rule_is_refused_with_d6_and_the_next_rule_still_runs`
panicked inside `sqlx-core`'s test harness before the test body ran — `failed to connect test
pool: database "…" does not exist` (`sqlx-core-0.9.0/src/testing/mod.rs:199`), i.e. the
harness's own per-test database, not an assertion. The test passes alone
(`cargo test -p lakehouse-api routes::alerts::alert_run_tenant_scope` — 2 passed) and the
second full run was `2942 passed; 0 failed; 18 ignored`. Reported as a harness flake; no
test was weakened.

**T10: mutation checks (each `check` call removed, named test run, restored; the tree is
clean again).**

| Site | Command | Result with the check removed |
| --- | --- | --- |
| T2 tile runner (`support.rs` `gate.check`) | `cargo test -p lakehouse-api --test dashboard_tenant_scope` | 2 failed: `another_tenants_member_sees_the_tile_refused_and_the_engine_is_never_asked`, `a_refused_reader_is_not_served_from_the_remembered_tile` |
| T3 records (`dashboard.rs` `run_records`) | same | 1 failed: `the_records_of_another_tenant_are_refused` |
| T3 filter values (`values_with_limit`) | same | 2 failed: `the_filter_values_of_another_tenant_are_refused`, `the_filter_values_of_a_board_of_another_tenant_are_refused` |
| T3 chart and table downloads (`build_chart_file`) | same | 2 failed: `the_chart_download_of_another_tenant_is_refused`, `the_table_download_of_another_tenant_is_refused` |
| T3 SQL sources (`dashboard_sources.rs` `check_and_rewrite`) | same | 2 failed: `the_sql_source_of_another_tenant_is_refused`, `the_sql_source_preview_of_another_tenant_is_refused` |
| T5 embed | `cargo test -p lakehouse-api --test embed_tokens` | 1 failed: `an_embed_in_a_deployment_without_a_catalog_tenant_is_refused` |
| T6 public link view | `cargo test -p lakehouse-api --test public_links` | 3 failed: `a_link_reads_as_the_tenant_that_made_it`, `a_link_whose_creator_left_its_tenant_is_refused`, `a_link_without_a_creator_reads_nothing_in_a_deployment_with_several_tenants` |
| T7 alert gate (`ApiSqlGate::gate`) | `cargo test -p lakehouse-api alert_run_tenant_scope` | 1 failed: `every_mart_rule_is_refused_with_d6_and_the_next_rule_still_runs` |
| T7 rule save (`gate_rule_statement`) | `cargo test -p lakehouse-api save_tenant_scope` | 2 failed: the mart and the metric case |
| T8 publication (`gated_source_table` in `publication_body`) | `cargo test -p lakehouse-api another_tenants_member_cannot_read_a_publication` | 1 failed |

(The T5 and T6 mutations remove the runner's `check` in `support.rs`, which is the call the
embed and the public-link board make per tile — the gate is built in `embed.rs`/
`public_links.rs` and spent in the runner.)

**Not verified, with the reason.**

- **M2's wall-clock measurement** (`ops/bench/dashboard_open_bench.py`, before and after,
  as a member of the catalog tenant): **not measured.** No stack is running on this machine
  and no API image is built for either commit; measuring needs `docker compose` with the demo
  data and a build of both revisions, which this machine's rules (shared with another
  session, no full-workspace build) do not allow. The *structural* claim — one reach per
  request, not one per tile — is proven by
  `routes::query::sec50_gate_tests::the_gate_resolves_the_reach_once_for_a_request` and
  `a_refused_gate_refuses_the_next_statement_the_same_way`. The numbers are the planner's or
  CI's to take.
- **Full-workspace `cargo clippy --workspace --all-targets --all-features`**: not run. Local
  is scoped to the crates this slice touched (`lakehouse-api`, `lakehouse-alerts`,
  `lakehouse-store`), per the owner's 2026-10-30 rule ("blaze locally, CI is final").
- **The G-gates** (`g3a-dagster`, `gold-export`, `g8-*`, `g6-alerts`): not run — they need
  the compose stack. CI's.
- **The AI-15 eval** (`ops/ai_eval/`): not run — needs a live stack; per M13 it must run
  with `CATALOG_TENANT_ID` set or as a `*:*` user.
- **A real `docker compose up`**: not run. No compose file changed in T7–T9; `config
  --quiet` is the check for the files that did change (`.env.example` is not read by
  compose).

**Console (`src/`):** no TypeScript change, as the plan's out-of-scope section says; no
`bun` command was run because nothing under `src/` moved.

### Review round 1 (planner, 2026-11-02, at `f320f54d`)

- **T1–T6** (`db7a337d` … `c114567c`), built on the desktop before it went offline:
  - one gate carrier, which the tile runner, the other signed-in sites and the delegated readers all ask;
  - the embed reads as the deployment's catalog tenant (D3);
  - a public link reads as the tenant that made it (D4).
- **Migration `0120`.** It has a why-header. `tenant_id` is nullable with no backfill, so an unknown tenant is never guessed. It uses `ON DELETE SET NULL`, the same rule as `0099`.
- **T7.** The alert run asks the gate as its actor (D3/M6). **M7 is closed per D7a:** `lakehouse-alerts` exports `mart_statement`, `metric_statement` and `statement_for_input`, both the run and the save use them, and the save is gated as the person saving. There is one builder (rule 4).
- **T8 (M12).** The publication body gates the fingerprint read as the route's caller. The ungated token helper says why at its site.
- **T9.** The boot warning names every gated path. `.env.example`, `OPERATIONS.md` and the feature page are updated.
- **T10, measured.** Removing each site's check makes its tests fail: T2 2, T3 records 1, values 2, downloads 2, sources 2, T5 1, T6 3, T7 run 1 and save 2, T8 1. So the tests test the gate, not the fixture.
- **Counts (the developer's):**
  - `lakehouse-api` lib: 2942 passed, and every integration target is green;
  - store: 528; alerts: 126; `gold_publication` against real ClickHouse: 14.
  - One sqlx harness pool flake was re-run and reported, not hidden.
- **Not verified, with the reasons given:** the M2 wall-clock benchmark (covered structurally by the once-per-request test) and the AI-15 eval.

**Open: no BLOCKER, no SHOULD-FIX.** The merge waits for the PR's CI, which runs the full workspace and the G-gates.

### Review round 2 (planner, 2026-11-02, at `84029cb`): CI's `Rust · test (stable)` fails

**BLOCKER 1. `tests/dashboard_export.rs` fails 7 of 11 tests in CI** (run `41497498109`, job `125456516149`). Every one of them gets 404 where 200 is expected:
- `a_person_whose_every_role_is_switched_off_cannot_export` (`:644`)
- `the_cap_is_a_million_rows_and_a_smaller_result_is_not_cut` (`:510`)
- `the_csv_download_is_formatted_or_raw_as_asked` (`:754`)
- `the_excel_download_is_masked_keeps_numbers_as_numbers_and_is_audited` (`:682`)
- `the_export_carries_the_tiles_own_filters_and_refuses_one_that_no_longer_applies` (`:339`)
- `the_export_of_a_joined_table_lists_joined_columns_and_governs_each_table` (`:439`)
- one more, in the same file.

The handoff says every integration target was green, so either this target was not run, or it passes locally for an environment reason that CI does not share.

**Most likely cause, to verify, not to assume:** T3's download gate (or the tile runner's, from T2) refuses the test's principal. The fixture's principal is probably not a member of the catalog tenant the gate resolves, and the 404 is the gate's SEC-20 answer. These tests export a real dashboard on purpose, so the fixture must set up a principal the gate admits.

**The fix:**
- Make the fixture honest: a principal in the catalog's tenant, using the same helper the other gated tests use (rule 4).
- Weaken no assertion (rule 2). If a test was relying on the hole the gate closes, change that test only with the reason at the site.
- Then run `cargo test -p lakehouse-api --test dashboard_export`, and EVERY `--test` target. List them with `ls rust/crates/lakehouse-api/tests/*.rs` and quote each count, so the handoff's "every target green" is literally true.

### Handoff round 2 (developer, 2026-11-02)

Branch `feat/sec-50-slice-2`, pushed after every commit. No PR opened, nothing force-pushed
(one `git pull --rebase` was needed, and one amended commit was replayed on top of the one it
replaced; history is linear and every pushed commit is still there).

#### BLOCKER 1: the cause

Reproduced first, on the branch tip before any change: `cargo test -p lakehouse-api --test
dashboard_export` → **4 passed, 7 failed**, every failure `left: 404, right: 200`.

The 404 is the tenant gate's, not the route's. I read the response body of
`the_export_writes_every_visible_column_masked_filtered_sorted_and_audited` before touching
anything:

```
status=404 Not Found
body={"error":"Table not found or not available to your organisation: serving.mart_x"}
```

That is `query::table_not_available`'s fixed `SEC-20` sentence. The path is:

1. `table_export` / `chart_export` build the gate as the caller —
   `StatementGate::clickhouse(&state, GateActor::Caller { principal, headers })`
   (`routes/dashboard.rs:3939`, `:3970`);
2. `GateActor::Caller` → `reach_for_actor` → `resolve_reach` →
   `catalog::catalog_tenant_refusal` (`routes/query.rs:586`);
3. `catalog_tenant_refusal` (`routes/catalog.rs:207`) sees **four tenants** in the seeded
   identity migration (`count > 1`), finds `state.config.catalog_tenant_id == None` because the
   fixture set no `CATALOG_TENANT_ID`, and answers `Some(CATALOG_TENANT_REFUSAL_UNCONFIGURED)`
   — a refusal;
4. `reach_from(refused = true, tenant = None, ..)` is `TenantReach::NoTenant`, and
   `check_tables_against_reach` refuses the first named table with `table_not_available`
   → 404.

So the fixture's principal belonged to no tenant and the deployment named no catalog tenant:
the gate had nothing that could read the shared catalog, which is exactly what it is for. The
route is right; the fixture was wrong. The comparison that made it obvious: `render_data.rs`
and `gold_publication.rs` read the same shared catalog and already name `common::CATALOG_TENANT`
and call `common::in_the_catalog_tenant` — the export target was the one added in this slice
that never got that treatment.

#### The fix (no assertion weakened)

`tests/dashboard_export.rs`, `app()` only: the app starts with
`CATALOG_TENANT_ID = common::CATALOG_TENANT`, and the exporter is put in that tenant with
`common::in_the_catalog_tenant` — the helper the other gated tests use (`AGENTS.md` rule 4).
No test body changed, no assertion changed, no route changed. The seven tests pass, and the
four that already passed still pass.

#### Two more reds found by running every target, and fixed

The handoff for round 1 said "every integration target green". That was not true, and CI would
have hit both as soon as `dashboard_export` stopped failing (`cargo test` stops at the first red
target, and `dashboard_export` sorts before both). Both are introduced by this slice and red on
this branch only (neither exists on `origin/main`):

1. **`route_auth` — 71 passed, 1 failed**:
   `the_alert_routes_are_signed_in_at_the_router_and_the_handler_asks_for_the_right` expected
   **403** and got **404**, for an Analyst without `alert:write`. Same cause as BLOCKER 1, one
   step further on: slice 2 M7 put `gate_rule_statement` into `authorise_rule_create`
   (`routes/alerts.rs:315`), so the gate runs before
   `chart_alerts::authorise_save` asks for the permission, and the test's mart rule reads
   `serving.m` in a deployment with no catalog tenant. **Fixed as a fixture, not a route
   change**: the test now starts the app with `CATALOG_TENANT_ID` and puts both seeded readers
   (`sari@`, `bayu@`) in that tenant, so the only refusal left is the permission the test is
   about. No assertion changed. 72 passed.
2. **`sec11_guard` — 3 passed, 1 failed**: `no_route_builds_a_response_from_an_upstream_errors_text`
   flagged four slice-2 lines that put the gate's own sentence into a response:
   `routes/support.rs:243`, `routes/dashboard_sources.rs:538`, `routes/layers.rs:257`
   (`SpecRunFailure::Gated(err) => json!({ "error": err.to_string() })`) and
   `routes/query.rs:915` (`GateRefusal`'s `message: err.to_string()`). The guard's own message
   names the sanctioned answer for text written in this repository: an `ALLOWED` entry with the
   reason. Added four entries, each citing the same fact — the sentence is `SEC-20`'s 404
   naming the table or D6's 422 naming the setting, written here, never engine text (the
   `SpecRunFailure::Gated` doc comment says so). No pattern widened, no test weakened.
   4 passed.

Both fixes are their own commits with the finding cited. If the planner would rather fix the
`route_auth` one in `routes/alerts.rs` (permission check before the gate) than in the fixture,
that is a decision for the review, not for me; the fixture version keeps every documented
answer (403 for a missing permission) and the gate's order.

#### Commits

| Commit | What |
| --- | --- |
| `3ee7ce15` | BLOCKER 1: `tests/dashboard_export.rs` names `CATALOG_TENANT_ID` and puts the exporter in the catalog's tenant. |
| `7fa02f90` | Clippy `doc_markdown` on the sentence that commit quotes (doc comment only). It sits on top of `3ee7ce15` because that commit was already pushed and this branch is never force-pushed. |
| `7d297ba3` | `tests/route_auth.rs`: the alert-route case reads `serving.m` as the catalog's tenant, so the 403 it asserts is the permission's. |
| `08b3c9d1` | `tests/sec11_guard.rs`: four `ALLOWED` entries for the gate's own fixed sentence. |

#### Commands run, with counts (all on `08b3c9d1`, `-j 2`, through `~/rantai/bin/cargo-locked`)

| Command | Result |
| --- | --- |
| `cargo test -p lakehouse-api --test dashboard_export` | 11 passed, 0 failed (was 4 passed, 7 failed) |
| `cargo test -p lakehouse-api --test route_auth` | 72 passed, 0 failed (was 71 passed, 1 failed) |
| `cargo test -p lakehouse-api --test sec11_guard` | 4 passed, 0 failed (was 3 passed, 1 failed) |
| `cargo fmt --check` | clean |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean |
| `cargo test -p lakehouse-api --lib` | 2942 passed, 0 failed, 18 ignored |
| every `--test` target (95 of them, below) | every line `ok`, 0 failed |

Per-target, `ls rust/crates/lakehouse-api/tests/*.rs` minus `common/` (95 targets; `--test
<name>` each, one run of the loop the brief gives):

| Target | Passed | Ignored |
| --- | --- | --- |
| `agents_approval` | 17 | 0 |
| `agents_run` | 14 | 0 |
| `ai_ask_back_turn` | 5 | 0 |
| `ai_build_tool_list` | 10 | 0 |
| `ai_cache_rule` | 3 | 0 |
| `ai_chart_draft` | 9 | 0 |
| `ai_chat_stream` | 3 | 0 |
| `ai_citations` | 2 | 0 |
| `ai_export_tools` | 9 | 0 |
| `ai_next_step` | 5 | 0 |
| `ai_report_tools` | 12 | 0 |
| `ai_sql_help` | 17 | 0 |
| `ai_terms` | 7 | 0 |
| `ai_tool` | 25 | 0 |
| `catalog_grants` | 10 | 0 |
| `catalog_restore` | 9 | 0 |
| `chart_alerts` | 12 | 0 |
| `chart_lineage_routes` | 5 | 0 |
| `chart_png` | 5 | 0 |
| `column_lineage_pipeline` | 7 | 0 |
| `column_lineage_route` | 9 | 0 |
| `column_lineage_sql_source` | 3 | 0 |
| `connector_delete_deprovision` | 9 | 0 |
| `connector_discovery_launch` | 4 | 0 |
| `connector_discovery_routes` | 7 | 0 |
| `connector_ingestible_tenant` | 6 | 0 |
| `connector_not_testable` | 2 | 0 |
| `connector_oauth_flow` | 22 | 0 |
| `connector_probe_history_route` | 5 | 0 |
| `connector_repoint` | 15 | 0 |
| `connector_secret_reference` | 7 | 0 |
| `connector_secret_rotate` | 8 | 0 |
| `connector_tenant_access` | 5 | 0 |
| `connector_upload_table` | 7 | 0 |
| `crypto_provider` | 3 | 0 |
| `dashboard_access_routes` | 36 | 0 |
| `dashboard_comments_routes` | 12 | 0 |
| `dashboard_export` | 11 | 0 |
| `dashboard_import` | 9 | 0 |
| `dashboard_tenant_scope` | 13 | 0 |
| `debezium_properties` | 5 | 0 |
| `embed_tokens` | 47 | 0 |
| `explain_rewrite` | 7 | 0 |
| `gold_export_auth` | 7 | 0 |
| `gold_publication` | 14 | 0 |
| `home_layout` | 5 | 0 |
| `hostile_uploads` | 0 | 2 |
| `iceberg_door` | 15 | 0 |
| `identity_user_tenants` | 6 | 0 |
| `ingest_run` | 22 | 0 |
| `lakehouse_maintenance` | 6 | 0 |
| `login_throttle` | 8 | 0 |
| `maps_routes` | 12 | 0 |
| `parity` | 0 | 2 |
| `pdf_export` | 9 | 0 |
| `pdf_real_gotenberg` | 0 | 3 |
| `pipeline_enforce` | 18 | 0 |
| `pipeline_lag_check` | 7 | 0 |
| `pipeline_lag_page` | 6 | 0 |
| `pipeline_routing` | 2 | 0 |
| `public_links` | 16 | 0 |
| `quality_anomalies` | 5 | 0 |
| `quality_incidents` | 8 | 0 |
| `quality_on_load` | 16 | 0 |
| `quality_schedules` | 3 | 0 |
| `query_author_exception` | 9 | 0 |
| `query_cancel_route` | 4 | 0 |
| `query_download` | 24 | 0 |
| `query_estimate_read_only` | 4 | 0 |
| `query_history` | 17 | 0 |
| `query_history_statement` | 4 | 0 |
| `query_limits_route` | 4 | 0 |
| `query_result_cap` | 9 | 0 |
| `query_run_limits` | 2 | 0 |
| `query_settings_rule` | 7 | 0 |
| `query_tenant_scope` | 19 | 0 |
| `render_data` | 13 | 0 |
| `reporting_settings_route` | 14 | 0 |
| `report_routes` | 14 | 0 |
| `report_runner` | 24 | 0 |
| `route_auth` | 72 | 0 |
| `sec11_guard` | 4 | 0 |
| `sec41_verify_ca` | 1 | 0 |
| `security_regressions` | 11 | 0 |
| `semantic_api` | 18 | 0 |
| `tenant_provisioning` | 8 | 0 |
| `test_connection_route` | 4 | 0 |
| `tier1_saved_query_guard` | 4 | 0 |
| `tier1_writehigh_approval` | 5 | 0 |
| `tier2_governance_drafts` | 4 | 0 |
| `tier2_writehigh_approval` | 3 | 0 |
| `upload_folder_routes` | 7 | 0 |
| `upload_routes` | 175 | 0 |
| `upstream_errors` | 7 | 0 |
| `--lib` | 2942 | 18 |

Totals: 2 940 passed across the 95 integration targets, 0 failed; `--lib` 2 942 passed, 18
ignored, 0 failed. Three targets report `0 passed` because all their cases are `#[ignore]`
(`hostile_uploads` 2, `parity` 2, `pdf_real_gotenberg` 3 — they need a live stack), which is
their normal state, not a skip I introduced.

**Not verified, with the reason.** The full workspace (`cargo clippy --workspace --all-targets
--all-features`, `cargo test --workspace`), the TypeScript, Python and compose blocks, and the
G-gates: not run here. This round changed three test files in `lakehouse-api` and no product
code, no `src/`, no `dagster/`, no compose file, so CI's full block is the first full run, as
the owner's 2026-10-30 rule allows. The M2 wall-clock benchmark and the AI-15 eval remain
unmeasured for the reasons round 1 gives.

**Worktree:** removed; no container was started by this round (the targets use `sqlx::test`'s
own per-test databases and `wiremock`).

### Review round 3 (planner, 2026-11-02, at `de0774dd`)

- **Round 2 is accepted.**
  - BLOCKER 1: the `dashboard_export` fixture now names `CATALOG_TENANT_ID` and puts the exporter in that tenant, as `render_data` and `gold_publication` already did. No assertion changed.
  - The `route_auth` fixture is fixed the same way.
  - The four `sec11_guard` entries were checked by the planner: `SpecRunFailure::Gated` and `GateRefusal` carry only the gate's own sentences, and a store failure on the reach path displays the classified `"database error"`.
  - All 95 targets and `--lib` are green, and listed.

**BLOCKER 3. The G4 gate fails in CI, and the cause is D3 working as intended** (runs `41506028036` and `41517431277`). Its WAL-slot alert reads `serving.replication_slot_health`. The gate's stack seeds four tenants and sets no `CATALOG_TENANT_ID`, so the alert run, which acts as `CatalogTenant`, is honestly skipped: "This job reads the shared catalog, and no tenant owns it here. Set CATALOG_TENANT_ID…".

**The fix:** each `ops/g*` harness, and its CI job, that runs a job or a non-`*:*` reader over the shared catalog names the catalog's tenant, exactly as an operator must after D7's boot warning.
- Set `CATALOG_TENANT_ID` in the gate's env, to the seeded tenant that owns the catalog. Use the same id as `tests/common::CATALOG_TENANT` if the seed is the same.
- Do it for G4 first, then check G3, G3a, G6, G8 and G9 the same way: grep each harness for alert runs, embeds, public links and gold publication.
- Assert nothing weaker. The gate's skip message must still appear when the variable is unset; keep or add one unit test for that.
### Handoff round 3 (developer, 2026-11-02)

Branch `feat/sec-50-slice-2`, commits `8f3653e5` (the fix) and the commit that
adds this entry, pushed after each. No PR opened, nothing force-pushed.

#### BLOCKER 3: the cause, at the code

The G4 WAL step
(`ops/g4/g4_test.py::step_wal_breach_produces_a_silenceable_alert_instance`,
`:498`) calls `POST /api/alerts/run`. That route is a job with no user: it
builds its gate as `GateActor::CatalogTenant` (`routes/alerts.rs:1250`), so
`reach_for_actor` → `catalog_tenant_reach(tenant_count, catalog_tenant_id)`
(`routes/query.rs:424`, `:493`) asks who owns the shared catalog. The gate's
stack seeds four tenants (`0002_seed_identity.sql`) and the job's `.env` named
none of them, so the answer was D6's refusal (`routes/query.rs:510`,
`JOB_NEEDS_CATALOG_TENANT`); the run recorded it as the rule's `skipped` reason
— and the step failed on the sentence after it, "expected a fired result",
which is why the log never named the cause.

#### The fix (no assertion weakened)

1. `.github/workflows/ci.yml`, job `g4-cdc`, the CI-only `.env` (`:1051`): set
   `CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001` with the why
   written above it. That is the same setting, the same seeded value and the
   same place the two G8 jobs (`:2912`, `:3103`), g3a-dagster's BLD-1 step
   (`:844`) and gold-export (`:2804`) already use (`AGENTS.md` rule 4), not a
   new mechanism. `lakehouse-api`'s compose block already forwards it
   (`docker-compose.yml:702`, `CATALOG_TENANT_ID: ${CATALOG_TENANT_ID:-}`), so
   neither `docker-compose.yml` nor `ops/g4/docker-compose.g4.override.yml`
   changed: the override file is for what a gate's stack gets and a real
   deployment must not (`CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`), and this is a
   deployment setting.
2. `ops/g4/g4_test.py`: a rule the run skipped is reported as itself
   (`the alert run skipped <id> instead of evaluating it: <skipped>`), so the
   skip's own sentence — the one that names the setting — reaches the log. The
   wording claims no cause, because `skipped` also carries other states
   (`lakehouse-alerts` `RunResult::skipped`).

#### The other gates, checked the same way

Each harness and its CI job were grepped for alert runs, embeds, public links
and Gold publication, and every `/api/*` read was traced to its session:

| gate | CI job | reads the shared catalog as | verdict |
| --- | --- | --- | --- |
| G3 | `g3-maintenance` (`:913`) | bootstrap admin (`ops/g3/g3_test.py:104`) → `GET /api/pipelines` (`:220`), `GET /api/governance/maintenance` (`:276`) | no change; `catalog_tenant_refusal` never refuses `"*:*"` (`routes/catalog.rs:212`), and no job-with-no-user path is called |
| G3a | `g3a-dagster` (`:723`) | bootstrap admin (`ops/g3a/g3a_test.py:119`) → `GET /api/catalog`, `/api/lakehouse/tables/bronze/…`, `GET /api/pipelines` | no change; the job already sets `CATALOG_TENANT_ID` (`:844`) for the G7 BLD-1 step it also runs |
| G6 (all seven harnesses) | `g6-ingest` … `g6-saas-b` | bootstrap admin (`ops/g6/g6_ingest_matrix_test.py:243`; src11/src12/src13/src17 and the oracle/saas/linklocal gates reuse it) | no change; connectors, ingest runs and `GET /api/catalog/` only, all in that session |
| G9 | `g9-upload` (`:1653`) | bootstrap admin (`ops/g9/upload_test.py:321`) → `/api/query/run`, uploads, `POST /api/pipelines` | no change; the authored pipeline it builds runs as its AUTHOR through `GateActor::AsTenant`, and that author is the `"*:*"` admin, which `reach_for_actor` short-circuits to `TenantReach::All` before any tenant is looked at (`routes/query.rs:404`) |
| G7, G8, gold-export | — | — | already set (`:844`, `:2912`, `:3103`, `:2804`) |

Greps: `/api/embeds` and `/api/public…` — no hits anywhere under `ops/`;
`/api/alerts/run` — `ops/g4` only; `/api/gold…` — `ops/g8/g8_governance_test.py:440`
only, posted with the admin session (a person, so `GateActor::Caller`,
`routes/gold.rs:289`) in a job that sets the setting anyway. No gate other than
G4 calls a job-with-no-user path.

#### The skip when the variable is unset (kept, not weakened)

Both are existing `#[cfg(test)]` unit tests, unchanged by this round:

- `routes::query::tests::a_job_with_no_user_is_refused_when_no_tenant_owns_the_shared_catalog`
  (`rust/crates/lakehouse-api/src/routes/query.rs:5589`) — the D6 422 and its
  exact sentence, including that the sentence names the setting.
- `routes::alerts::alert_run_tenant_scope::every_mart_rule_is_refused_with_d6_and_the_next_rule_still_runs`
  (`rust/crates/lakehouse-api/src/routes/alerts.rs:2562`) — the skip this
  blocker was about: several tenants and no catalog tenant → every mart rule's
  `result["skipped"]` is `JOB_NEEDS_CATALOG_TENANT`, the run goes on to the next
  rule, and no `serving.*` statement reaches the engine.

No Rust changed this round (`git diff --stat 7d28a262..HEAD` on the fix commit:
`.github/workflows/ci.yml`, `ops/g4/g4_test.py`), so neither was re-run here;
both were green in round 2 and CI runs them on the PR.

#### Checks run

```
python3 -m py_compile ops/g4/g4_test.py                                        # ok
docker compose -f docker-compose.yml -f ops/g4/docker-compose.g4.override.yml \
  --profile dagster config --quiet                                             # ok
docker compose --profile '*' config --quiet                                    # ok (with ops/init-env.sh's .env)
python3 -c "import yaml; …"                                                    # g4-cdc's .env carries the setting
```

The gate itself, locally, the way `g4-cdc` runs it — one gate, its own project
name, torn down afterwards:

```
docker compose -p g4ci --profile dagster build lakehouse-api                   # rantai-lakehouse-api:ci, this branch
docker compose -p g4ci -f docker-compose.yml -f ops/g4/docker-compose.g4.override.yml \
  --profile dagster build oidc-mock
docker compose -p g4ci -f docker-compose.yml -f ops/g4/docker-compose.g4.override.yml \
  --profile dagster up -d --no-build postgres clickhouse rustfs rustfs-bucket-init \
  lakekeeper-db-init lakekeeper-migrate lakekeeper lakekeeper-warehouse-init \
  g4-source-init debezium-server lakehouse-api dagster-code-location dagster-webserver dagster-daemon
docker compose -p g4ci -f docker-compose.yml -f ops/g4/docker-compose.g4.override.yml \
  --profile dagster run --rm -T g4-test-runner                                 # exit 0
docker compose -p g4ci -f docker-compose.yml -f ops/g4/docker-compose.g4.override.yml \
  --profile dagster down -v                                                    # 0 containers, volumes removed
```

`docker inspect g4ci-lakehouse-api-1` carried
`CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001`, and the step that
failed in CI reported:

```
[g4] replication_slot_check_job run 824e6a7a-… finished: visible after 10.1s
[g4] WAL breach produced a fired, silenceable alert_instance row; re-run after silence stayed suppressed
[g4] slot 'p5cdc_slot' no longer exists after deprovisioning — WAL is no longer pinned
[g4] OIDC authorization-code + PKCE round trip: session minted, Analyst role's query:read confirmed
[g4] PASS
```

The Dagster image was reused (`rantai-lakehouse-dagster:local`, 2026-10-31):
this slice changes no Dagster code (`git log -1 --
dagster/dispar_orchestrate/replication_metrics.py` is `24be9133`, 2026-09-30).
The API image was rebuilt from this branch, not reused.

#### What I did not verify

- CI itself: `g4-cdc` on the PR is the proof; the run above is the same steps
  on this machine.
- G3, G3a, G6 and G9 were read, not run. No change was needed in any of them
  (table above), but that is `not verified` by execution.
- The two Rust unit tests named above: not re-run (no Rust changed).
