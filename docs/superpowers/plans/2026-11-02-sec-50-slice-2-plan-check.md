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
