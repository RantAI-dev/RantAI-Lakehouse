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

## 6. Review (planner)

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
