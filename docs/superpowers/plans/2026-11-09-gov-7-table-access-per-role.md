# GOV-7: read access per schema and table, enforced everywhere data is read

Backlog `GOV-7` (Feature, Governance, P1). Spec `docs/core/specs/gov-7.md`, feature page
`docs/core/features/table-access-per-role.md`. Base: `main` at `dcaae8a5` (anchors verified
there). **Waits for `ADM-13`**, because both change `POLICY_TABLE`, `route_auth.rs`, the role
form and the migration sequence.

Two PR slices on one branch line:

- **S1, enforcement and API.** Rust only. Branch `feat/gov-7-s1-enforce`.
- **S2, console.** Branch `feat/gov-7-s2-console`, cut from S1 after S1 merges.

Planner and reviewer: Claude Opus. Developer: another agent.

## 1. Decisions

Owner, 2026-11-09: D1 and D2. Planner: the rest.

- **D1. Default is "All tables".** Existing and new roles keep reading every table until an
  administrator narrows them, so an upgrade changes nobody's access.
- **D2. Hidden entirely.**
  - A table a person may not read is missing from every list.
  - A statement naming it is refused with the **same** status and text as the tenant gate's
    refusal: `"Table not found or not available to your organisation: {table}"`
    (`routes/query.rs:711`). It never says "forbidden".
- **D3. Storage.** Migration `NNNN_table_access_per_role.sql`: the next free number on
  `origin/main` after `ADM-13`'s, with a why-header.
  - `ALTER TABLE role ADD COLUMN read_scope TEXT NOT NULL DEFAULT 'all'`, with a named CHECK
    `read_scope IN ('all','listed')`.
  - `CREATE TABLE role_read_grant`, with:
    - `role_id UUID NOT NULL REFERENCES role(id) ON DELETE CASCADE`;
    - `schema_name TEXT NOT NULL`;
    - `table_name TEXT NOT NULL` (`'*'` means the whole schema);
    - `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`;
    - `created_by UUID NULL REFERENCES app_user(id) ON DELETE SET NULL`;
    - `PRIMARY KEY (role_id, schema_name, table_name)`.
  - Every existing role becomes `'all'` through the default.
- **D4. The rule.**
  - New module `rust/crates/lakehouse-api/src/table_access.rs`. Its module doc states the
    invariant and links the feature page.
  - `pub enum TableAccess { All, Listed(HashSet<(String, String)>, HashSet<String>) }`: the
    tables, and the whole schemas.
  - `pub async fn for_roles(pg: &PgPool, roles: &[String]) -> Result<TableAccess, StoreError>`
    reads the principal's roles **by name**. That covers OIDC-mapped role names on the
    session (`0045`).
  - `All` when any of these holds:
    - `roles` is empty;
    - any named role is `'all'`;
    - any named role's permissions contain `*:*`.
  - Otherwise it returns the union of the listed grants.
  - An empty `roles` list means a service identity or a system run with no principal
    (`gold.rs:462`, `service_token.rs:226`). It returns `All`, with a comment: services are
    governed by their scopes (spec "Not included").
  - A role name that is unknown in the database contributes nothing. It is not `All`.
  - `fn allows(&self, schema: &str, table: &str) -> bool`. Compare names **exactly as the mask
    matcher compares `PolicyCondition.table`** (`policy_engine.rs:48-95`). Reuse that
    normaliser, including Trino's catalog prefix (`DATA-21`). Do not write a second one
    (rule 4).
- **D5. One choke point for statements: `policy_engine::rewrite_sql_for_roles`
  (`policy_engine.rs:255`).**
  - It already receives `roles` on every governed read path. Section 4 of the research lists
    them: Query Studio, dashboards, metrics, layers, sources, embeds and public links (fixed
    `Dashboard Viewer`), PDF render, alerts and chart alerts (owner), reports (owner),
    authored pipelines (author), Gold export, and the assistant's `run_sql`,
    `run_saved_query`, `query_metric`, `run_alert_rule` and `export_gold_mart`.
  - After `referenced_tables`, it loads `TableAccess` through `obligations_source`'s pool and
    refuses with a new `EnforcementError::TableNotReadable(String)` if any table is not
    allowed. That happens before the mask prefetch.
  - With no pool (`obligations_for_async`, `policy_engine.rs:497-499`), it returns `All`. Put
    the same comment there as for masks.
  - **A narrowed principal (`Listed`) is also refused**, with `LISTING_NOT_AVAILABLE`
    (`query.rs:726`) or the not-found text, for:
    - any statement whose sources the rewrite cannot resolve to concrete `schema.table`
      names: table functions (`s3`, `url`, `remote`, `file`, `iceberg`, `cluster`, and so on),
      system or `information_schema` tables, `SHOW TABLES`/`SHOW DATABASES` and
      `DESCRIBE` of a hidden table;
    - any `CTE` that cannot be followed to its base tables.
  - Find what the classifier already refuses (`sql_rewrite.rs` `Classifier`) and add only the
    gap. List the gap in the handoff.
  - **Map the new error in one place.** Find where `EnforcementError` becomes an `ApiError`.
    If more than one site does it, add one helper and use it from every site. Every caller
    then answers `TableNotReadable` exactly as the tenant gate's not-found: same status, same
    text. The handoff lists every call site of `rewrite_sql_for_roles` with how it now maps
    the error.
  - An alert or report run that hits it records the not-found message on its run history.
    It does not record a generic failure.
- **D6. Lists that skip the rewrite are filtered per caller.** Each call gets `TableAccess`
  from `for_roles(principal.role_names)`. These are the endpoints, listed in section 4 of the
  research:
  - `GET /api/catalog` (also `?q=`) (`catalog.rs:262`), `GET /api/catalog/query` (`:406`):
    filter the 30-second shared snapshot **after** it is read. Never cache a per-person
    result in the shared snapshot.
  - `GET /api/catalog/{id}` (`:1713`), `/sample` (`:2311`), `/profile`
    (`catalog_profile.rs:347`), and every other `/api/catalog/{id}/…` GET: a hidden asset
    answers the **same** 404 as an unknown id.
  - `/api/lakehouse/namespaces`, `tables` and `tables/{ns}/{table}` (`lakehouse.rs:533,586,633`):
    filter, and hidden gives 404.
  - `GET /api/dashboard/fields`, `join-suggestions` (`dashboard.rs:2270,2616`): drop hidden
    tables.
  - `/api/semantic*`: if it lists tables or columns, filter. If it does not, say so in the
    handoff.
  - The assistant's DATA MAP (`routes/ai/data_map.rs`). The cache stays shared. The map text
    for a caller is built from it with hidden tables removed, and that happens **before**
    anything reaches the model. Add a test that a narrowed caller's prompt does not contain
    a hidden table's name.
  - The assistant's catalog read tools (`list_datasets`, `lakehouse_overview`,
    `describe_dataset`) go through the filtered endpoints or the same filter.
  - Lineage graph reads (`lineage:read` routes): hide nodes for hidden tables. If this turns
    out to be large, stop and report instead of skipping it.
- **D7. API to set it.**
  - `GET /api/identity/roles/{id}/table-access` (`identity:read`) and `PUT` (`identity:write`).
    Body and response: `{ "scope": "all" | "listed", "grants": [{ "schema": "finance", "table":
    "*" | "<name>" }] }`.
  - `PUT` rules:
    - 404 for an unknown role;
    - 400 for a role whose permissions contain `*:*` ("A role with full access cannot be
      narrowed");
    - 400 for a schema or table name that fails the identifier check the rewrite uses (reuse
      it);
    - write the scope and the grants in **one transaction**;
    - audit `identity.role.table_access.update` with role id, name, and before and after
      scope and grants. Follow the audit helper used in `routes/identity.rs`.
  - Names need not exist yet: a grant may name a table that a pipeline creates later.
  - Both rows go into `POLICY_TABLE`, asserted both ways in `route_auth.rs`. Handlers return
    `ApiResult<ApiJson<T>>`. Errors go through `StoreError` to `"database error"`, never
    `err.to_string()`.
- **D8. Console (S2).** Role edit gets a section **Tables this role can read**:
  - A radio: **All tables** or **Only these**.
  - With **Only these**: a list of schemas from `assetService.listAssets`, grouped by schema.
    Each schema has a "Whole schema" checkbox and, expanded, a checkbox per table.
  - A free-text "Add a table not yet created" row (`schema.table`).
  - The Save button calls `PUT …/table-access`.
  - A role holding `*:*` shows the section disabled with the reason.
  - The roles list shows "All tables" or "N schemas, M tables".
  - It goes where the Edit role form lives when S2 is built: the side panel, or the page if
    `WEB-7` merged first.

## 2. Anchors (verified at `dcaae8a5`)

- `rust/crates/lakehouse-api/src/policy_engine.rs:48-95` `PolicyCondition`, `:255`
  `rewrite_sql_for_roles`, `:493-506` `obligations_for_async`.
- `rust/crates/lakehouse-api/src/sql_rewrite.rs:5347` `enforce`, `referenced_tables`,
  `Classifier` (`~:4791`).
- `rust/crates/lakehouse-api/src/routes/query.rs:711`, `:726`, `:959`, `:1035`, `:1190`.
- `rust/crates/lakehouse-api/src/routes/support.rs:316`, `routes/embed.rs:44,1439`,
  `routes/alerts.rs:1071,1099`, `routes/authored_pipelines.rs:1460`, `routes/gold.rs:276,462`.
- `rust/crates/lakehouse-api/src/routes/catalog.rs:262,406,1713,2311`,
  `routes/catalog_profile.rs:347`, `routes/lakehouse.rs:533,586,633`,
  `routes/dashboard.rs:2270,2616`, `routes/ai/data_map.rs`.
- `rust/migrations/0001_init.sql:44` `role`.

## 3. Tasks (one commit each, in order)

S1:

- **T1 (D3, D4).** The migration and `table_access.rs`.
  - `sqlx::test` cases:
    - no roles gives `All`;
    - an `'all'` role among listed ones gives `All`;
    - a `*:*` role gives `All`;
    - two listed roles give the union;
    - a whole schema allows any table in it;
    - an unknown role name gives an empty `Listed`;
    - Trino's catalog-prefixed name matches as masks match.
  - *Accept:* `cargo test -p lakehouse-api table_access`.
- **T2 (D5).** The choke point, the refusal of unresolvable sources, and the single error
  mapping.
  - Tests through the real handlers:
    - Query Studio `run` on a hidden table gives not-found, and on an allowed table returns
      rows;
    - `estimate` and `download` behave the same;
    - a dashboard tile on a hidden table gives not-found while the other tiles render;
    - an embed follows the Dashboard Viewer role's access;
    - an alert owned by a narrowed user records not-found;
    - an authored pipeline enforce call for a narrowed author gives not-found;
    - a narrowed principal's `SELECT * FROM system.tables`, a table function and `SHOW
      TABLES` are refused, and an unrestricted principal is unchanged;
    - masks still apply on an allowed table.
  - *Accept:* `cargo test -p lakehouse-api query::`, `cargo test -p lakehouse-api
    policy_engine`, `cargo test -p lakehouse-api embed`, `cargo test -p lakehouse-api
    alerts`, `cargo test -p lakehouse-api authored_pipelines`, and clippy.
- **T3 (D6).** The filtered lists. Tests for each endpoint in D6: the list omits the hidden
  table, the detail gives the same 404 body as an unknown id, and the assistant prompt does
  not name a hidden table.
  - *Accept:* `cargo test -p lakehouse-api catalog`, `cargo test -p lakehouse-api lakehouse`,
    `cargo test -p lakehouse-api data_map`, `cargo test -p lakehouse-api dashboard::`.
- **T4 (D7).** The routes, `route_auth`, the audit row, and the refusal for a `*:*` role.
  - *Accept:* `cargo test -p lakehouse-api identity`, `cargo test -p lakehouse-api --test
    route_auth`, `cargo test -p lakehouse-api --lib route_policy_tests`.
- **T5.** A latency measurement for D4.
  - Time `for_roles` plus `allows` over 1,000 calls against the `sqlx::test` database with 10
    roles and 200 grants. Report p50 and p95 in the handoff. Do not invent a number; if it
    is not run, write *not verified*.
  - If p95 is over 5 ms, report it. Do not add a cache unasked.

- **D9. Bronze tables are named in the lists (planner, 2026-11-11, from S2 round 1).**
  - A Bronze row of the catalog list is addressed by a registry slug and named no table, so
    a narrowed role never saw a Bronze row, whatever it was granted, and the picker could
    not offer one.
  - `bronze_catalog_row` emits `tableKey`: `bronze.<table_name>`, or `null` when the registry
    records no table. It is the same key `bronze_asset_detail_body` already emits.
    `asset_table_key` is unchanged and now finds it.
  - The picker reads `tableKey` from the list row. A Bronze table is listed by name under the
    schema `bronze`. A row with no key counts as unnamed under `bronze`.
  - The catalog's `namespace` (`primer`, `sekunder`) is a display grouping. It is never
    offered or sent as a schema: no table lives in a schema of that name, so such a grant
    reads nothing.

S2:

- **T6 (D8).** The contract (`RoleTableAccess` type, `getRoleTableAccess`,
  `setRoleTableAccess`), the client, the service, the section, and the roles-list summary.
  - Bun tests:
    - choosing "Only these" with one schema sends the right body;
    - a `*:*` role shows the section disabled;
    - a not-yet-created table row is sent as typed.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin`.
- **T7.** Docs: `docs/OPERATIONS.md` gets "Limiting a role to some schemas or tables", with
  the limits from the feature page.
- **T8 (D9; S2 round 1 B2).** Rust, `lakehouse-api` only. `bronze_catalog_row` takes the
  table name and emits `tableKey`.
  - Tests in `tests/table_access.rs`:
    - a narrowed role granted one Bronze table sees that row in `GET /api/catalog` and in
      `GET /api/catalog/query`, and does not see another Bronze row;
    - a narrowed role granted only a Silver table still sees no Bronze row.
  - The first test must fail on the branch before the change. Say so in the handoff.
  - *Accept* (every cargo command through `/home/shiro/rantai/bin/cargo-locked`):
    `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
    `cargo test -p lakehouse-api --test table_access`,
    `cargo test -p lakehouse-api --lib catalog`. Never a bare
    `cargo test -p lakehouse-api <filter>`.
- **T9 (D9; S2 round 1 B1, S1, S2).** Console.
  - `schemaChoices` takes the table from `tableKey` when the row has one, else from an `id`
    that holds a dot. A row with neither counts as unnamed under `bronze`. `namespace` is not
    read.
  - A grant's table is compared without regard to case everywhere in the section, as the
    rule compares it.
  - Save sends the table-access request only when the scope or the grants differ from what
    was loaded.
  - Bun tests:
    - a Bronze row with `tableKey: "bronze.orders"` is offered as `orders` under `bronze`;
    - a Bronze row with no key gives a `bronze` group with the sentence, and no `primer`
      group;
    - a stored grant `silver.Orders` shows the catalog's `orders` ticked, and unticking it
      removes the grant from the body;
    - saving the role with the section untouched sends no table-access request.
  - `docs/OPERATIONS.md` says how Bronze tables are named in the section.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin src/services`.

## 4. Out of scope

- Column hiding.
- `DATA-15` grants adding tables.
- Access requests for hidden tables.
- Service identities and the Iceberg door.
- Dagster's own builds and quality checks.
- An assistant tool for setting table access.
- A cache for `TableAccess`, unless T5 shows a need and the planner agrees.

## 5. Handoff (developer appends below, one entry per slice)

Give the commits, the exact commands with their counts, the D5 call-site list with how each
maps the error, the D5 gap list, the T5 numbers or *not verified*, and the output of
`git status --short`, `git log --oneline origin/main..HEAD` and `git ls-remote --heads origin
<branch>`.

### Handoff (developer, 2026-11-11, S1)

Branch `feat/gov-7-s1-enforce`, base `origin/main` at `4189c0b1` (`ADM-13`
merged), plus one `git merge origin/main` (`cf5a9923`, WEB-7 S1 — TypeScript
only).

#### Commits

- `d64b5055` T1 — migration `0135_table_access_per_role.sql` and
  `table_access.rs`.
- `0b2afb50` T2 — the enforcement choke point, the unresolvable-statement
  refusal, the single error mapping.
- `47b68b6a` — the PR #459 CI fix: the tile-cache fixture creates the `Admin`
  role it names. D4 stands; see "Fixture rule" below.
- `ef90d402` — `wip:` commit made so T3 could be merged with `main` without
  losing uncommitted work; superseded by `d0459e0a`.
- `cf5a9923` — merge of `origin/main` (WEB-7 S1, `ai/prompt.rs` and its
  tests), no Rust conflict.
- `d0459e0a` T3 — the filtered lists and details.
- `5817f0d3` T4 — the `table-access` routes, `POLICY_TABLE`, audit.
- `9097acea` T5 — the latency measurement (report only, no cache).

#### Moved anchors

Every `file:line` in section 2 was verified at `dcaae8a5`; `main` moved all of
them. Re-found by name, current locations:

- `policy_engine.rs`: `PolicyCondition` :48, the mask matcher
  (`c.table.eq_ignore_ascii_case`) inside `obligations_for_async` :551,
  `rewrite_sql_for_roles` :308, `obligations_for_async`'s no-pool branch :565.
- `sql_rewrite.rs`: `referenced_tables` :118, `Classifier` :4769, `enforce`
  :5347, `gate_targets` :205 (new dependency of this slice, reused rather
  than copied).
- `routes/query.rs`: `table_not_available` :709, `LISTING_NOT_AVAILABLE` :730
  (both now delegate to `policy_engine`, one definition), the rewrite :352,
  the gate :482.
- `routes/support.rs`: `run_spec_sql` :227, `enforcement_message_for` :290,
  `enforcement_refusal` (new) :310, `try_run_spec_sql` :345.
- `routes/alerts.rs` `ApiSqlGate` :1072, `routes/embed.rs` rewrite via
  `support`, `routes/authored_pipelines.rs` :1569, `routes/gold.rs` the
  `PolicyRefused` mapping :97.
- `routes/catalog.rs`: `list` :262 and `query` :406 stayed; `detail` :1726,
  `sample` :2340, the new helpers
  (`table_access_for`, `asset_table_key`, `asset_visible`, `retain_visible`,
  `ensure_asset_readable`) beside `not_found` :3165;
  `catalog_profile::profile` :347, `bronze_table_name` :302.
- `routes/lakehouse.rs`: `namespaces` :533, `tables` :602, `table_detail`
  :656.
- `routes/dashboard.rs`: `fields` :2270, `join_suggestions` :2616.
- `migrations/0001_init.sql` `role` :41.

#### D5: every call site of `rewrite_sql_for_roles` and its mapping

The new `EnforcementError::TableNotReadable(String)` and
`EnforcementError::Unresolvable` are mapped in ONE place,
`routes::support::enforcement_refusal(sql, err) -> ApiError`: the first is the
tenant gate's own 404 and sentence (`policy_engine::table_not_available_message`),
the second the gate's 422 sentence (`policy_engine::LISTING_NOT_AVAILABLE`).
`policy_engine::enforcement_error_message` answers the same two sentences
(now `String`), for the sites whose answer is a message rather than an
`ApiError`.

| Call site | Mapping |
| --- | --- |
| `routes/query.rs::rewrite_sql_for_principal_inner` | `enforcement_refusal`; the DATA-21 past-timestamp sentence still wins first |
| `routes/support.rs::try_run_spec_sql` (tiles, embeds, public links) | `SpecRunFailure::Refused(enforcement_message_for(..))` — the tile's error slot carries the not-found sentence; a tile has no status |
| `routes/dashboard.rs::run_records`, `export_tile_statement`, `filter_values` | `enforcement_refusal` |
| `routes/dashboard_sources.rs::check_and_rewrite` | `enforcement_refusal` |
| `routes/alerts.rs::ApiSqlGate::rewrite` | `GateRefusal::Refused(enforcement_refusal(..).to_string())` — the run result's `skipped` records the sentence (AC10) |
| `routes/authored_pipelines.rs` enforce | `enforcement_refusal` (404 for a hidden table; the GATE's own refusal stays 422 via `gate_refusal_as_unprocessable`, unchanged) |
| `gold_export.rs::export_batch_sql` | `GoldExportError::PolicyTableRefused(ApiError)`, mapped straight through in `From<GoldExportError> for ApiError` (the route answers 404, not its usual 422) |
| `routes/ai/tools/data.rs::run_sql` | covered by the Query Studio site it calls (`rewrite_sql_for_principal`) |

#### D5: the gap added

No new `sql_rewrite::Classifier` variant. The classifier already refuses
table functions (empty allowlist) and sensitive `system.*` tables for every
principal. What it could NOT see — and what a narrowed principal would
otherwise pass — is a statement **whose tables do not appear in
`referenced_tables`**: `DESCRIBE x` / `DESC x` (`Statement::ExplainTable`),
`SHOW COLUMNS FROM x`, `SHOW TABLES`/`SHOW DATABASES` (`ListsObjects`), and a
statement whose only sources are `system.*`/`information_schema.*`. Those now
go through `sql_rewrite::gate_targets` — the tenant gate's own resolver, one
parse, same collector — inside `rewrite_sql_for_roles`, before the mask
prefetch: every named table must be allowed (else `TableNotReadable`), and
`ListsObjects` is refused (`Unresolvable`). A statement naming no table at all
(`SELECT 1`) still runs. A CTE is followed to its base tables by the same
collector; a CTE body the collector cannot parse makes the whole statement
unparseable, which the rewrite already refuses.

#### D6: what `/api/semantic*` lists

It lists **entries**, not tables: each entry carries an `asset` (the table
key), a `column`, a description, synonyms, a role, status and staleness.
`GET /api/semantic` now drops the entries whose `asset` the caller's roles
hide; `GET /api/semantic/{asset}` answers the same empty list an asset with
no entries answers, so it is not an oracle. Lineage was NOT large: the graph
node ids already carry the table (`table:`, `iceberg:`, `bronze:`), so
`lineage::node_table_key` filters nodes (and their edges) in
`lineage::build`, `lineage::charts::read` and `lineage::used_by::read`
(the last answers the same 404 a hidden asset answers anywhere else).

#### T5: measurement

`table_access::tests::the_access_check_costs_what_the_plan_budgeted` (in
`src/table_access.rs`), `sqlx::test` database, 10 narrowed roles and 200
grants, 1,000 iterations of `for_roles` + ten `allows` calls, unoptimized
`cargo test` build on the dev machine (11.9 GB RAM, shared with another
developer's build), three runs:

- run 1: p50 **3.420 ms**, p95 **5.144 ms**, max 8.393 ms
- run 2: p50 **3.460 ms**, p95 **4.837 ms**, max 85.943 ms (one outlier)
- run 3: p50 **3.391 ms**, p95 **4.577 ms**, max 9.003 ms

With no roles (a service identity or system run) the call is a constant:
`for_roles` returns `All` without touching Postgres, 0.0001 ms/call. So the
cost is one Postgres round trip per statement, as D4 intends. **p95 sits ON
the 5 ms budget** (4.58–5.14 ms across runs, one run over), in a debug build;
this is reported as the plan asks, and **no cache was added** (the plan says
to report first). If the planner wants it under budget with margin, the
follow-up is a per-request `TableAccess` memo — named here, not built.

#### Fixture rule found by CI (D4)

D4 resolves access by role NAME and an unknown name is not `All`, so a test
fixture that invents a role name is now refused. One existed:
`routes::support::run_spec_sql_enforcement::a_masked_and_an_unmasked_viewer_never_share_a_remembered_result`
passes `"Admin"`. Fixed in `47b68b6a` by creating the role with the migration
default `read_scope = 'all'`; the assertion is unchanged. Grep of both test
trees (`rewrite_sql_for_roles(` and every role vector in `src/**` test modules
and `tests/`) found no other: the remaining role literals are the seeded
`Analyst` and the fixed `Dashboard Viewer`. Production cannot hit this —
`principal.role_names` always comes from `role` rows.

#### Commands, as run (counts are the test binary's own)

Rust build/format/lint, from `rust/`, with `/home/shiro/rantai/bin/cargo-locked`:

- `fmt --check` — pass.
- `clippy -p lakehouse-api --all-targets -- -D warnings` — pass (last at
  `9097acea`).
- `clippy -p lakehouse-store --all-targets -- -D warnings` — pass (T4 touched
  it).
- `check -p lakehouse-api --all-targets` — pass between edits (not a
  verification claim).

Unit tests, `--lib <filter>`:

- `query::` 187 passed; `policy_engine` 9; `embed` 20; `alerts` 74;
  `authored_pipelines` 60 (T2 Accept, run as the plan wrote them — see the
  note below).
- `catalog` 301; `lakehouse` 55; `data_map` 79; `dashboard::` 181 (T3 Accept).
- `identity` 35; `route_policy_tests` 2 (T4 Accept).
- `run_spec_sql_enforcement` 9 (the CI fix).
- `table_access` 11 (T1's 10 cases + T5's measurement), also used for T5.
- **whole lib: 3430 passed, 0 failed, 18 ignored** (`--lib`).

Integration tests, one file at a time (`--test <file>`):

- `table_access` 12 — T2's and T3's end-to-end cases (Query Studio
  run/estimate/download, dashboard tiles, embed as Dashboard Viewer, alert
  owner, authored pipeline, listings, masks, catalog list/query/detail, mart
  picker).
- `identity_table_access` 8 — T4's routes, refusals and audit row.
- `route_auth` 87 — the sweep plus the new both-ways case.
- Because their behaviour this change can alter, also: `query_tenant_scope`
  19, `dashboard_tenant_scope` 13, `embed_tokens` 48, `chart_alerts` 19,
  `pipeline_enforce` 18, `explain_rewrite` 7, `alert_test_route` 6,
  `catalog_grants` 10, `catalog_restore` 9, `lakehouse_maintenance` 6,
  `chart_lineage_routes` 5, `dashboard_export` 11, `public_links` 16,
  `ai_tool` 25, `ai_sql_help` 26, `ai_citations` 2, `iceberg_door` 15,
  `column_lineage_route` 9.

Note on the Accept lines: `cargo test -p lakehouse-api table_access` (and
`query::`, `catalog`, …) without `--lib` builds all 110 integration binaries
first — 47 GB of disk and several minutes — before the filter applies. This
was run that way once (T1) before the cost was known; every later run used
`--lib` or `--test <file>`. The plan's Accept lines should say so.

`ops/` and `dagster/`: no reference to `read_scope`, `role_read_grant`, the
new routes or the access rule (grep). `ops/g4` and `ops/g10` assert
`query:read` for service identities, whose scopes (not roles) govern them, so
nothing changes there. No assistant tool or schema changed
(`tests/fixtures/tool_schemas.json` is untouched).

#### `git status --short`, `git log --oneline origin/main..HEAD`, remote

```
$ git status --short
(clean)

$ git log --oneline origin/main..HEAD
9097acea test(gov-7): measure the access check the read path pays (T5)
5817f0d3 feat(gov-7): the API that sets a role's read access (T4)
d0459e0a feat(gov-7): filter every list and detail by the caller's table access (T3)
cf5a9923 Merge remote-tracking branch 'origin/main' into feat/gov-7-s1-enforce
ef90d402 wip(gov-7): T3 filtered lists in progress (catalog tests still red)
47b68b6a fix(gov-7): the tile-cache fixture creates the role it names (D4)
0b2afb50 feat(gov-7): enforce the access check on every governed read (T2)
d64b5055 feat(gov-7): read access per schema and table, stored per role

$ git ls-remote --heads origin feat/gov-7-s1-enforce
9097acea6cd881516d76de7a1f8dd0fede8dc34b	refs/heads/feat/gov-7-s1-enforce
```

(`origin/main` still ends at `0133_pages_per_role.sql`, so
`0135_table_access_per_role.sql` is free; ADM-12's `0134` is unmerged.)

**not verified locally: the rest, left to CI** — the other ~90 integration
test files, the workspace-wide `cargo clippy --workspace`/`cargo test
--workspace`, and S2 (the console). No product behaviour was changed for a
service identity, the Iceberg door, or Dagster, as "Out of scope" says.

No mismatch between plan and code was found except the two noted above (the
Accept command shape, and the T5 p95 landing on the budget).

### Handoff (developer, 2026-11-11, S1 round 1 fixes)

Branch `feat/gov-7-s1-enforce`, from `5ad8c129`; S2 then S1, one commit each,
pushed after each.

#### Commits

- `55a9c58e` — S2: `git merge origin/main` (merge commit). `origin/main` was
  one commit ahead, `ba2d8f0c` WEB-7 S2 (the console, TypeScript only).
- `8974f4e8` — S1: `TableAccess` no longer derives `Default`.

#### S2: what the merge had

**No conflicts** — the merge resolved on its own and nothing was hand-edited.

`origin/main` still ends at `rust/migrations/0133_pages_per_role.sql`: ADM-12
(#455) has not merged, so there is no `0134_sso_settings.sql` and no other
`0135` either. `0135_table_access_per_role.sql` is unchanged and still the
next free number. The only `main` change was `src/**` (console), so no Rust
file moved and no migration is affected.

#### S1: the call sites touched

The `Default` derive and `#[default]` on `All` are removed, and the type
carries the one-line reason ("No `Default` on purpose (`GOV-7` S1 review
round 1, S1): a default of `All` would make `unwrap_or_default()` on a failed
read mean 'every table'.").

**No call site needed changing**, because none used it — checked, not assumed:

- `TableAccess::default(` — no match in `src/` or `tests/`.
- `unwrap_or_default()`/`Default::default()` applied to an access value — no
  match; the matches in `catalog.rs`, `dashboard_access.rs` and the tests are
  `String`/`Option<String>`/fixture defaults, unrelated to `TableAccess`.
- `TableAccess` outside `lakehouse-api` — only `lakehouse-store`'s unrelated
  `RoleTableAccess` struct.

For the record, the sites whose answer is "no narrowing" say so explicitly
already and were left as they are: `table_access::for_roles` (`Ok(All)` for no
pool or no roles), `routes::catalog::table_access_for` (`?`, a refusal), and
the assistant's three readers — `routes::ai::mod` (a failed read hides every
table from the map, `All` only when there is no principal at all),
`ai::tools::data` (a failed read is the tool's error, `All` only for no
principal) and `ai::tools::dashboards` (the same).

**One doc line changed with it, and only it**: the module doc's "The default
is [`TableAccess::All`]" now reads "The *stored* default", so the sentence
names the `role.read_scope = 'all'` column default rather than a Rust
`Default` that no longer exists.

#### Commands, as run (counts are the test binary's own)

From `rust/`, with `/home/shiro/rantai/bin/cargo-locked`:

- `check -p lakehouse-api --all-targets` after the merge — pass (3m 01s).
- `fmt --check` — pass.
- `check -p lakehouse-api --all-targets` after S1 — pass (1m 50s), the compile
  proof that nothing relied on the removed `Default`.
- `clippy -p lakehouse-api --all-targets -- -D warnings` — pass (3m 40s).
- `test -p lakehouse-api --lib table_access` — **11 passed, 0 failed**
  (3,437 filtered out).
- `test -p lakehouse-api --lib policy_engine` — **9 passed, 0 failed**
  (3,439 filtered out).
- `sg docker -c '… test -p lakehouse-api --test table_access'` — **12 passed,
  0 failed** (0 filtered out).

**not verified locally: the rest, left to CI** — the other integration test
files, the workspace-wide `clippy`/`test`, the console (`bun run typecheck`,
`lint`, `test` for the WEB-7 S2 merge, TypeScript only and already green on
`main`).

#### `git status --short`, `git log --oneline 5ad8c129..HEAD`, remote

```
$ git status --short
(clean)

$ git log --oneline 5ad8c129..HEAD
8974f4e8 refactor(gov-7): TableAccess has no Default, so a failed read cannot mean all (S1)
55a9c58e Merge remote-tracking branch 'origin/main' into feat/gov-7-s1-enforce
ba2d8f0c feat(console): Query Studio saves, chart builder and alert rule become full pages (WEB-7 S2) (#463)

$ git ls-remote --heads origin feat/gov-7-s1-enforce
8974f4e8cd0498543bc8d53290e5d1e913981b4f	refs/heads/feat/gov-7-s1-enforce
```

(`ba2d8f0c` is `origin/main`'s tip, on this branch only through the merge.
This handoff's own docs commit sits on top of the three lines above.)

### Handoff (developer, 2026-11-12, S2)

Branch `feat/gov-7-s2-console`, cut from `origin/main` at `de8430be` (`GOV-7` S1, #459,
merged). TypeScript and docs only: no Rust, no migration, no API change, no new page and no
`ROUTE_ACCESS` entry.

#### Commits

- `d599bf2b` T6 — the contract (`RoleTableAccess`, `RoleTableGrant`,
  `SetRoleTableAccessInput`), the client and `services/index.ts`, the section
  (`role-table-access.tsx`), the Edit page's Save, the roles-list summary, and the tests.
- `e80bfed3` T7 — `docs/OPERATIONS.md`, "Limiting a role to some schemas or tables".
- `b4a19189` — two comments at the sites where grants are kept (no behaviour), so the
  reason sits at the code (rule 1).

#### The Save decision: one Save for the page

The plan's "The Save button calls `PUT …/table-access`" is read as the page's single
**Save**. `RoleEditForm` (`role-form-page.tsx`) writes the role first
(`PUT /api/identity/roles/{id}`), then the tables
(`PUT /api/identity/roles/{id}/table-access`), and:

- if the role write fails, no table-access request is sent and the error says
  "The role was not saved: …";
- if the role write succeeds and the table-access write fails, the page says
  "The role was saved, but its tables were not: <the server's own sentence>" (a 400's
  text arrives unchanged through `ServiceError.message`) and does **not** navigate;
- a role holding `*:*` sends no table-access write at all: the section is disabled with
  the API's sentence ("A role with full access cannot be narrowed") and the role save
  navigates as before;
- if the role's current access could not be read, the role is saved, no table-access
  write is sent (a write now would replace grants nobody saw), and the page says so and
  stays. **Save is disabled while that read is in flight**, so this can only follow a
  failed read, which the section shows with a Retry.

The section's own state is the page's draft, seeded from `GET …/table-access`; grants the
catalog does not list are rendered ("Not in the catalog (kept until removed)") inside a
schema group that opens by default, and are sent back unchanged until removed.

#### What the roles-list summary reads

`GET /api/identity/roles` returns the store's `Role` (`id`, `name`, `members`,
`permissions`, `description`); `ROLE_SELECT` does not read `role.read_scope`, so the list
carries no table access and the API was **not** changed. `roles-page.tsx` therefore reads
`GET …/table-access` **once per role**, in parallel (`loadRoleTableAccesses`): N requests
for N roles, on a page an administrator opens by hand. A role whose read fails is `null`
and its row says "—" (with a title) rather than "All tables"; while the reads are in
flight the cell also says "—", never a guessed answer.

The summary is computed into the row (`RoleRow` in `roles-columns.tsx`) rather than into
the cell from a map, so the `columns` array keeps its identity when the reads land: a
rebuild remounts every cell and closes a row's open menu under the reader. While loading,
a role whose permissions hold `*:*` already says "All tables", because D4 makes the rule
ignore its stored scope.

#### Where the picker's schemas come from, and its one gap (reported, not improvised)

`assetService.listAssets({})` reads `GET /api/catalog`. A Silver or Gold row's `id` **is**
its `<schema>.<table>` key, so the section groups by the part before the first dot and
lists the part after it — the names a statement uses.

**A Bronze row cannot be named from the list.** Its `id` is a registry slug and
`bronze_catalog_row` emits no `tableKey`/`tableName`; only the detail body does
(`bronze_asset_detail_body`, from `dataset_sync`). The console already documents this as
"the Bronze slug gap" (`src/lib/table-key-href.ts`). So, for a Bronze asset, the section
offers its **namespace** (`primer` / `sekunder`) with the "Whole schema" checkbox and the
sentence "The catalog names these assets by slug, not by table name. Choose the whole
schema, or add a table below."; the free-text row adds `bronze.<table>` by hand. No table
name is invented, and nothing the role already holds is dropped. Listing Bronze tables by
name would need an API change, which S2 does not make.

When the catalog read fails, the section says so and keeps the grants it has (still
removable) and the free-text row; it never shows an empty list as if there were no tables.

#### Tests added

- `src/features/admin/role-form-page.test.tsx` — 11 tests (6 of them S2): "Only these"
  with one whole schema sends `{"scope":"listed","grants":[{"schema":"silver","table":"*"}]}`
  after the role's own PUT; a `*:*` role shows the section disabled with the server's
  sentence and writes no table access; `bronze.orders_new` typed by hand is sent exactly
  as typed; a grant the catalog does not list is shown and kept through a save; a refused
  table-access write says the role was saved but its tables were not and stays on the page;
  a failed catalog still shows the grant and saves it.
- `src/features/admin/roles-page.test.tsx` — 9 tests (3 S2): "All tables", "2 schemas,
  1 table", and "—" (never "All tables") when a role's access cannot be read.
- `src/services/clients/identity.test.ts` — 4 tests: the GET, the PUT body, the 400
  sentence kept as `invalid_request`, and 404 as `not_found`.

#### Commands, as run (foreground, final code commit `b4a19189`)

- `bun install --frozen-lockfile` — "Checked 779 installs across 753 packages (no
  changes)", 1.52 s.
- `bun run typecheck` — pass (no output).
- `bun run lint` — **11 problems, 0 errors, 11 warnings**, all pre-existing and none in a
  file this slice touched (data-table, sidebar, sso-page, alerts-page, login-page,
  use-copilot, use-data-table, dashboard-specs, uploads.test, services/http.ts). The one
  warning this slice introduced (an unused `permissions` in the test stub) was removed.
- `bun test src/features/admin` — **133 passed, 0 failed**, 20 files.
- `bun test src/services` — **216 passed, 0 failed**, 20 files.
- `bun test src/components/app-shell` — **27 passed, 0 failed**, 4 files (run though no
  page or route changed).
- No cargo command: no Rust file changed.

One run of `bun test src/features/admin` (chained after the other two folders in one
shell command) reported **132 pass / 1 fail** and the failing name was not captured; ten
subsequent runs — eight alone and two in the same chained sequence — were **133 pass /
0 fail**. Recorded as observed. CI runs the full suite on the pull request.

**not verified locally: the rest, left to CI, and the section in a real browser.** Locally
not run: the whole `bun run test`, and the page against a live API (a real `/api/catalog`
and the two table-access routes), so the picker's rendering and the two-request Save were
verified only through the component tests and the client test.

**For the planner:** the feature page's "Limits to tell a customer"
(`docs/core/features/table-access-per-role.md`) still lists only its original four limits.
The three from the S1 review — a scheduled agent run is not narrowed, names are compared
without regard to case, the check is one database read per request — are in
`docs/OPERATIONS.md` after T7, but not yet on the page. The page is the planner's file, so
it was not edited here.

No mismatch between the plan and the code was found, except the Bronze naming gap above:
the plan says "the schemas from `assetService.listAssets`", and the list names no Bronze
table. Handled honestly, reported here, no decision changed.

#### `git status --short`, `git log --oneline origin/main..HEAD`, remote

```
$ git status --short
(clean)

$ git log --oneline origin/main..HEAD
b4a19189 docs(gov-7): say at the code why a `*:*` role and whole-schema picks keep their grants (T6)
e80bfed3 docs(gov-7): how to limit a role to some schemas or tables (T7)
d599bf2b feat(gov-7): pick the schemas and tables a role can read (T6)

$ git ls-remote --heads origin feat/gov-7-s2-console
b4a191891ba256039650affdc8b2a7a0788aa829	refs/heads/feat/gov-7-s2-console
```

(This handoff's own docs commit sits on top of the three lines above, as the S1 handoffs'
did.)


### Handoff (developer, 2026-11-11, S2 round 1 fixes)

Fixes `GOV-7` S2 review 1: T8 (B2, Rust) and T9 (B1, S1, S2, console). Decisions unchanged;
no mismatch between the plan and the code was found.

#### Commits (pushed after each)

- `55c757dd` fix(catalog): a Bronze list row names its table so a narrowed role sees it (T8)
- `da16e9ea` fix(admin): name Bronze tables under bronze, compare grants without case, write
  only a changed section (T9)

#### T8 (B2), `rust/`, every command through `/home/shiro/rantai/bin/cargo-locked`

The new test was written first and run against the unfixed branch. It failed with:

```
thread 'the_catalog_lists_the_bronze_table_a_narrowed_role_is_granted' panicked at
crates/lakehouse-api/tests/table_access.rs:1132:5:
a granted Bronze table must be listed: {"assets":[],"namespaces":[]}
```

Commands, as run on the final T8 code (foreground, shared `CARGO_TARGET_DIR`):

- `cargo-locked fmt --check` — pass (no output).
- `cargo-locked clippy -p lakehouse-api --all-targets -- -D warnings` — pass,
  `Finished` with no warning.
- `cargo-locked test -p lakehouse-api --test table_access` — **14 passed, 0 failed**.
- `cargo-locked test -p lakehouse-api --lib catalog` — **301 passed, 0 failed**,
  3,172 filtered out.

`bronze_catalog_row` now takes `table_name` and emits `"tableKey"`:
`bronze.<table_name>`, or `null` when the registry records none;
`assemble_bronze_rows` passes `str_col(c, "table_name")`. `asset_table_key`'s logic is
unchanged, and its doc comment plus the row builder's say what the list carries now. The
function gained a ninth argument, so it carries `#[allow(clippy::too_many_arguments)]` with
a reason, the same one `bronze_detail_body` gives. `CatalogFake` now answers the Bronze
`dataset_catalog` read with two datasets; no existing assertion was weakened.

#### T9 (B1, S1, S2), console

- **B1:** `schemaChoices` reads a row's table from `tableKey`, else from a dotted `id`; a row
  with neither counts as unnamed under the one `BRONZE_SCHEMA` constant (whose comment names
  `catalog.rs` `ensure_asset_readable`/`bronze_asset_detail_body`). `namespace` is not read.
  The module comment, the contract's `tableKey` comment and the unnamed-assets sentence say
  what is true now.
- **S1:** `hasGrant` and `toggleTable` compare a grant's table without regard to case, so a
  stored `silver.Orders` is the catalog's `orders` and unticking removes it.
- **S2:** `tableAccessChanged` compares the draft's scope and grant set (case-insensitive,
  order-insensitive) with what was loaded; Save sends the table-access request only when it
  differs. An unloaded section is untouched: the role save completes and the page goes back.
  The `accessSkipped` state and its message are removed.

Existing tests changed to make a change before Save (they had assumed a write on every
Save; no assertion was deleted):

- `loads the role by id, warns about a rename, and saves the whole role` — now picks
  "Only these" first, so the expected body is `{"scope":"listed","grants":[]}`.
- `shows a grant the catalog does not list and keeps it through a save` — now also ticks
  "Whole schema archived"; the body carries the unlisted `old_orders` plus `archived.*`.
- `says the role was saved when only its tables were refused, and stays` — now unticks
  "Whole schema silver" so the refused write is attempted.
- `says so when the catalog fails and still keeps the grant it has` — now ticks "Whole schema
  silver"; the body carries the kept `orders` plus `silver.*`.

New tests (full-sentence names): a Bronze row with `tableKey: "bronze.orders"` is offered as
`orders` under `bronze`, and `primer` is not shown; a row with no key gives a `bronze` group
with the sentence and no `primer` group; a stored `silver.Orders` ticks the catalog's
`orders` and unticking removes it from the request body; an untouched Save sends no
table-access request; a role whose tables failed to load still saves and goes back.

`docs/OPERATIONS.md` ("Limiting a role to some schemas or tables"): one paragraph on Bronze
tables (named under `bronze`; a dataset with no registry table name is not listed and stays
hidden from a narrowed role's lists), and the Save bullet now says a write happens only when
the section changed.

Commands, as run (foreground, final T9 code):

- `bun install --frozen-lockfile` — "678 packages installed", 2.11 s.
- `bun run typecheck` — pass (no output).
- `bun run lint` — **11 problems, 0 errors, 11 warnings**, all pre-existing, none in a file
  this round touched.
- `bun test src/features/admin src/services` — **354 passed, 0 failed**, 40 files.

**Not verified, and why:** the whole `bun run test` and the whole Rust workspace were not
run locally (the brief scopes them to CI on #468); the page was not opened against a live
API, so the picker and the two-request Save are covered by the component tests only.

#### `git status --short` and the remote head before this docs commit

```
$ git status --short
(clean)

$ git ls-remote --heads origin feat/gov-7-s2-console
da16e9eaabf8a3388b7f87fb7c9ddeb2b02f787c	refs/heads/feat/gov-7-s2-console
```


## 6. Review (planner)

### S1, round 1 (planner, 2026-11-11) — branch at `3f6b5569`

Read in full: the migration, `table_access.rs`, the change to
`policy_engine::rewrite_sql_for_roles` and its error mapping, `sql_rewrite::gate_targets`,
the two routes in `routes/identity.rs`, and every place that builds `TableAccess::All` or
handles a failed read of the rule. Sampled, not read line by line: the list filters of D6
(`catalog.rs` entry and helper, the assistant's map and tools); their tests are in
`tests/table_access.rs` (1,141 lines), which I did not read either. I ran no check locally;
#459 runs the suite. (The S1 handoff sat under this heading; it is moved up into section 5,
unchanged.)

- **The rule fails closed where it must.** A database that cannot be read refuses the
  statement (`PolicyEngineError::Store`), returns the route's error on the lists, and hides
  every table from the assistant's map. A role name with no row gives nothing, never all.
- **The refusal is the tenant gate's own sentence and status** (`table_not_available_message`,
  one definition used by both).
- **Statements that name no table are covered** through the tenant gate's resolver:
  `DESCRIBE`, `SHOW COLUMNS`, listings, `system.*`. Table functions were already refused for
  everyone.
- **The write route** checks the scope, refuses a `*:*` role, validates names with
  `Ident::new`, and audits before and after.
- The CI failure on T2 was the test fixture naming a role with no row; the fixture now
  creates the role and the assertion is unchanged, which is the right fix under D4.

**No BLOCKER.**

**SHOULD-FIX**

- **S1. `TableAccess` derives `Default`, and the default is `All`.** Nothing uses it today,
  but it makes `unwrap_or_default()` on a failed read mean "every table", the one mistake
  this type must not allow. Remove the derive and the `#[default]`.
- **S2. Merge `origin/main`** (the branch is behind). `ADM-12` (#455) merges first and
  carries migration `0134`; this branch keeps `0135`.

**The plan's own mistake (planner).** The *Accept* lines in section 3 are written as
`cargo test -p lakehouse-api <filter>`, which builds all 110 integration test files before
filtering (47 GB on 2026-11-11). Read each as `--lib <filter>` or `--test <file>`. S2's
brief says so.

**For the product owner**

- **The check costs about 3.4 ms at the median and 4.6 to 5.1 ms at p95** per governed read
  (developer's measurement: unoptimised test build, shared machine, 10 roles and 200 grants).
  The budget on the feature page is 5 ms *(proposed)*. It is one database read per request
  and was not cached, as the plan says. A release build should be faster; that is not
  measured.
- **A run with no person behind it reads every table.** A scheduled agent run and a service
  identity carry no roles, so the rule does not narrow them (spec, "Not included"). A person
  who may manage agents could have an agent read a table their own role hides. A backlog item
  is filed at merge.
- **Names are compared without regard to case**, as masks are (D4). Two tables that differ
  only by case share one grant. Added to the feature page's limits at merge.

### S1, round 2 (planner, 2026-11-11) — branch at `ba616e13`

Read the fix (`8974f4e8`, `table_access.rs`).

- **S1: closed.** `TableAccess` has no `Default`; nothing relied on it, so no call site
  changed.
- **S2: closed** (`55a9c58e`, no conflict).

**No open BLOCKER and no open SHOULD-FIX.** I ran no check locally. Merged only when #459 is
green on its last commit, and after `ADM-12` (#455), so the migrations land as 0134 then
0135; the branch is updated once #455 is in.

### S2, round 1 (planner, 2026-11-11) — branch at `171275cf`, pull request #468

Read in full: `role-table-access.tsx`, the changes to `role-form-page.tsx`, `roles-page.tsx`,
`roles-columns.tsx`, the client and the contract, and the new section of
`docs/OPERATIONS.md`. Read on the server to check the console against it:
`table_access.rs` (`allows`, `allows_key`), `routes/catalog.rs` (`bronze_catalog_row`,
`asset_table_key`, `asset_visible`, `ensure_asset_readable`), `tenant.rs` (the namespace
constants) and `put_role_table_access`. The three test files were sampled, not read line by
line.

Run by me in a detached worktree at `171275cf`: `bun install --frozen-lockfile`,
`bun run typecheck` (pass), `bun run lint` (0 errors, 11 warnings, none in a file of this
slice), `bun test src/features/admin src/services` (349 pass, 0 fail, 40 files). The one
failure the handoff saw once did not appear. The whole `bun run test` and the Rust suite:
not run by me; #468 runs them.

What holds:

- **Nothing a role holds is dropped without the person doing it.** The draft starts from the
  stored grants, a grant the catalog does not list is shown and sent back, and a section
  that could not be loaded writes nothing.
- **A failed save stays on the page and says which half failed.**
- **A `*:*` role** shows the section disabled with the server's sentence and sends no write.
- **A read that failed is "—" in the Roles list**, never "All tables".
- The Roles list reads each role's access with one request per role. Accepted for a page an
  administrator opens by hand; the comment at the site says why.

**BLOCKER**

- **B1. The picker offers `primer` and `sekunder` as schemas, and such a grant reads
  nothing.**
  - `schemaChoices` puts a Bronze row under its `namespace`, with a "Whole schema" checkbox
    and the sentence "Choose the whole schema, or add a table below."
    (`role-table-access.tsx`, the branch after the `id` has no dot).
  - `namespace` is a catalog grouping (`tenant.rs` `NAMESPACE_PRIMER`, `NAMESPACE_SEKUNDER`).
    A Bronze table's key is `bronze.<table>` (`catalog.rs` `ensure_asset_readable`,
    `bronze_asset_detail_body`), and the rule compares the schema part of that key
    (`table_access.rs` `allows`).
  - So `{ "schema": "primer", "table": "*" }` is stored, shown as ticked, and matches no
    table. An administrator who ticks it believes the role reads the Bronze data. The role
    reads none of it.
  - The handoff reported the naming gap and called this handling honest. It is not: the
    control states an access that does not exist (principle 2).
  - Fix: D9 and T9. `namespace` is never a schema.
- **B2. A narrowed role never sees a Bronze row in the catalog lists, whatever it is
  granted.** This is in S1, already merged (#459), and my S1 review missed it: I sampled the
  list filters and did not read them.
  - `retain_visible` keeps a row only when `asset_table_key` names its table.
    `bronze_catalog_row` emits no `tableKey` and its `id` is a slug with no dot, so the key
    is `None` and the row is dropped for every narrowed role.
  - It fails closed: nothing is shown that should be hidden. But a role granted
    `bronze.orders` can query the table and open its page by address, and cannot find it in
    the catalog or its search. `tests/table_access.rs` has no Bronze row in a list test, so
    nothing caught it.
  - Read from the code, not run. T8's first test proves it by failing before the fix.
  - Fix: D9 and T8.

**SHOULD-FIX**

- **S1. A grant whose table differs from the catalog's name only by case is invisible.**
  - `schemaChoices` compares names without regard to case, and `hasGrant` and `toggleTable`
    compare the table exactly.
  - With a stored `silver.Orders` and a catalog `orders`: the grant is not listed as "not in
    the catalog" (the names match without case), and `orders` is not ticked (they differ
    with case). The grant is sent back on every save and cannot be seen or removed here.
  - Typing `silver.Orders` into the free-text row does the same: a grant is added and nothing
    on the page changes.
  - The module comment says an existing grant is always shown. Compare the table without
    regard to case, as the rule does (`table_access.rs` `allows`).
- **S2. Every Save of the role page writes the table access, changed or not.**
  - The server audits `identity.role.table_access.update` on every write
    (`put_role_table_access`), so renaming a role records a table-access change nobody made.
  - It also replaces the stored grants with the copy this page loaded. If another
    administrator changed them in between, an untouched section silently reverts their
    change.
  - Send the request only when the scope or the grants differ from what was loaded. A
    section that could not be loaded is then an untouched section: the role save completes
    and nothing more needs saying.

**For the product owner**

- **A Bronze dataset whose registry records no table name stays hidden from a narrowed
  role's lists** after D9 as well: nothing names the table, so nothing proves a grant covers
  it. It goes on the feature page's limits at merge.
- The three limits the S1 review found (a scheduled agent run is not narrowed, names are
  compared without regard to case, one database read per request) are in
  `docs/OPERATIONS.md` now and go onto the feature page at merge.

### S2, round 2 (planner, 2026-11-11) — branch at `a2ff9f66`

Read the two fix commits in full (`55c757dd`, `da16e9ea`), with the two new tests in
`tests/table_access.rs`.

Run by me in a detached worktree at `a2ff9f66`: `bun run typecheck` (pass), `bun run lint`
(0 errors, 11 warnings, none in a file of this slice), `bun test src/features/admin
src/services` 18 times: 354 pass in 16 runs, 353 pass and 1 fail in 2. The failing test,
captured once, is `AssistantSettingsForm (ADM-9) > never pre-fills the key, and only asks
for it when it is replaced`. It is not in a file this slice touches and it is the failure
the S2 handoff saw once without its name. It gets a backlog item at merge. The Rust checks:
not run by me; #468 runs them.

- **B1: closed.** `schemaChoices` no longer reads `namespace`. A Bronze table comes from the
  row's `tableKey` and is listed under `bronze`; a row with no key is counted there as
  unnamed, with a sentence that says no grant covers it.
- **B2: closed.** `bronze_catalog_row` emits `tableKey`. The test failed before the change
  with `a granted Bronze table must be listed: {"assets":[],"namespaces":[]}`, which is the
  gap as read. It now checks both lists, and that a second Bronze row and a Silver-only
  grant stay apart.
- **S1: closed.** `hasGrant` and `toggleTable` compare the table without regard to case.
- **S2: closed.** Save writes the table access only when the scope or the set of grants
  differs from what was loaded. The comparison is of what would be sent, so a scope of
  `all` with leftover ticks is not a change.

**SHOULD-FIX**

- **R2-S1. The contract does not declare the field the picker reads, and two comments say
  more than the server does.**
  - `tableKey` is declared on `AssetDetail` only (`src/services/contracts/assets.ts`). Its
    comment now says the list row carries it, and `listAssets` returns `Asset[]`. The picker
    reads it through its own structural type, so the compiler does not hold the contract to
    it. Declare `tableKey` on `Asset`.
  - `tableKeyOf`'s comment says the list row carries "the id itself on Silver/Gold", and
    `schemaChoices`' comment says a Silver or Gold row's "`tableKey` says the same". The
    Silver and Gold list rows emit no `tableKey` (`catalog.rs`, the `silver` and `serving`
    row builders); only the Bronze row and the detail bodies do. The `id` is what names
    them, which the code handles. Make the comments say that (rule 1).

**No open BLOCKER.** R2-S1 is fixed before merge because #468 waits in the queue behind
#466 and #467 anyway.
