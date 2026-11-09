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
