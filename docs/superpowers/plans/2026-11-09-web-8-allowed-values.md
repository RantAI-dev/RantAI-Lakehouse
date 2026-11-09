# WEB-8: forms offer the allowed values instead of free text

Backlog `WEB-8` (Task, Platform, P2). Spec `docs/core/specs/web-8.md`. Base: `main` at `dcaae8a5`
(anchors verified there); the developer branches off the latest `origin/main` and re-checks
each anchor, reporting any that moved. Planner and reviewer: Claude Opus. Developer: another
agent (`AGENTS.md`).

Two PR slices:

- **S1, the central permission list.** Branch `feat/web-8-s1-permission-catalog`. It goes
  first because `ADM-13` and `GOV-7` add permissions to the same list.
- **S2, the remaining pickers.** Branch `feat/web-8-s2-pickers`. **Waits for `WEB-7`**,
  because the forms move to pages there; S2 changes them in their new place.

The Invite user roles and tenants checkboxes are delivered by `ADM-11` (same form, same PR).
S2 does not touch them.

## 1. Decisions

- **D1. One permission list, in `lakehouse-auth`.**
  - New module `rust/crates/lakehouse-auth/src/permission_catalog.rs`.
  - `pub struct PermissionInfo { pub name: &'static str, pub area: &'static str, pub
    description: &'static str }` and `pub const PERMISSIONS: &[PermissionInfo]`.
  - One entry for every permission string the API checks today. Section 1 of the research
    lists them: the 25 in `POLICY_TABLE`, plus the strings checked in handlers: `alert:write`,
    `ops:logs`, `identity:sessions:manage`, `audit:read`.
  - Descriptions are plain English, one line, saying what the permission opens: "Run SQL and
    read table data in Query Studio", not "query read".
  - Areas group the picker: Data, Dashboards, Pipelines, Governance, Administration,
    Assistant, Operations.
  - The four strings seeded but never checked (`policy:review`, `residency:*`,
    `feature:write`, `notebook:run`) are **not** in the list. Section 4 of the research has
    them.
- **D2. Wildcards stay valid.** `area:*` is valid when some listed permission has that
  resource. `*:*` is always valid. `pub fn is_known(token: &str) -> bool` answers for a
  single token, with the same parsing as `PermissionSet::parse` (`permissions.rs:54`).
- **D3. The server refuses new unknown permissions.**
  - `POST /api/identity/roles`, `PUT /api/identity/roles/{id}` and service identity
    create/update answer **400** with `{"error":"unknown permissions","unknown":[…]}` when a
    token is not known.
  - On update, a token the role or identity **already** holds is kept even if unknown, so
    editing an old role never fails (rule 14: say "not recognised", do not drop silently).
  - Grep for an existing validation helper in `routes/identity.rs` first (rule 4).
- **D4. `GET /api/identity/permissions`.**
  - Policy `RequiresPermission("identity:read")`.
  - Returns `[{ name, area, description }]` in list order.
  - Add it to `POLICY_TABLE` and assert it both ways in `tests/route_auth.rs`.
  - Handler returns `ApiResult<ApiJson<Vec<…>>>`.
- **D5. A drift test keeps the list honest.** A `#[test]` in `lakehouse-api` fails when any of
  these is missing from the list:
  - every `Policy::RequiresPermission(p)` in `POLICY_TABLE`;
  - every non-empty `ToolSpec.permission` in `routes/ai/registry.rs` `TOOLS`;
  - every string literal passed to `.has("…")` under `rust/crates/lakehouse-api/src`. Read the
    files at test time from `env!("CARGO_MANIFEST_DIR")` and match `\.has\("([^"]+)"\)`. Also
    match the named constants `SAMPLE_PERMISSION`, `ENTRY_PERMISSION` and `POLICY_READ` by
    adding their values to an explicit list in the test, with a comment.
- **D6. A JSON fixture for the console.**
  - `rust/crates/lakehouse-api/tests/fixtures/permissions.json` is the list as JSON.
  - A Rust test asserts that the fixture equals `PERMISSIONS`. Copy the regenerate pattern of
    `tests/ai_build_tool_list.rs:143` (`tool_schemas.json`).
  - `ADM-13`'s TypeScript test reads this fixture, the same way
    `src/features/copilot/capabilities.test.ts` reads `tool_schemas.json`.
- **D7. Console pickers for S1.**
  - The role permission field becomes grouped checkboxes from `GET /api/identity/permissions`.
    This covers Create and Edit (`roles-page.tsx:185` `RoleFields`, input at `:209-215`).
  - Each area has an "All in this area" checkbox (`area:*`). Holders of `*:*` see one "Full
    access (`*:*`)" checkbox.
  - Tokens on the role that are not in the list show as removable chips labelled "Not
    recognised".
  - The service identity scopes field (`service-identities-page.tsx:205-218`) uses the same
    picker component.
  - The `query:run` placeholder is gone with the text field.
  - Layering: contract `Permission` type and `listPermissions` on `IdentityService`, then the
    client, then `services/index.ts`, then the feature. The picker is
    `src/features/admin/permission-picker.tsx` (kebab-case, `"use client"`).
- **D8. S2 pickers**, each in its `WEB-7` page location:
  - **SSL mode.**
    - Move `CDC_SSL_MODES` and `ORACLE_SSL_MODES` out of `cdc-dial-form.tsx:25-38` into
      `src/features/connectors/dial-forms/ssl-modes.ts`.
    - `sql-dial-form.tsx:112-121` and `oracle-dial-form.tsx:87-94` render the ui `Select`.
    - The SQL form keeps a "Default" entry that sends nothing, which is today's blank.
      Verify that the server treats a missing value as blank before relying on it.
  - **Pipeline zones.**
    - The source zone (`pipeline-create-page.tsx:449-454`) and the target zone (`:815`) are a
      `Select` over `namespaces` (`:223`).
    - The source table keeps its `<datalist>`, because `DATA-19` D4 says it stays free text
      (`:211-221`).
    - The target table stays free text, because a pipeline creates it.
  - **Pipeline columns.**
    - Applies to the incremental column (`:432`), key, column, value, from and to (`:663-729`)
      and the columns list (`:759`).
    - When the source table exists, they are a `Select` over
      `lakehouseService.getTableDetail(zone, table).schema`. The columns list is a multi-select
      with the `Faceted multiple` primitive.
    - When the table does not exist or the detail call fails, they stay free text, with the
      line "Columns appear here once the table exists".
  - **Table and column pickers.**
    - Applies to data quality (`data-quality-page.tsx:382`, `quality-check-fields.tsx:86-104`
      with real `columns`, `quality-rule-edit-dialog.tsx:105`), freshness
      (`freshness-target-dialog.tsx:113`) and alert rule tables (`alerts-page.tsx:644,659`).
    - Tables are a combobox over `assetService.listAssets`, showing `schema.table`.
    - Columns come from the asset detail schema.
  - **Time zone.** `subscriptions-dialog.tsx:319` uses `listTimeZones()`
    (`src/lib/time-zones.ts:8`), as `reporting-settings-page.tsx:206-214` does.
  - **Row filter hint.**
    - `asset-access.tsx:173-179` gets a help line naming the functions and operators the row
      filter accepts.
    - Take the list from the server: find the allowlist that `sql_rewrite.rs` uses to accept a
      row filter. If it is a Rust constant, add it to `GET /api/governance/row-filter-grammar`.
      That route needs `policy:read`, must go in `POLICY_TABLE`, and needs both-way
      `route_auth` asserts.
    - Do not hard-code a copy in TypeScript (rule 4).
    - If no such allowlist exists, stop and report.
  - **Access-rule roles field.** The roles field in the same dialog (`asset-access.tsx:149-155`)
    becomes checkboxes from `identityService.listRoles`.

## 2. Anchors (verified at `dcaae8a5`)

- `rust/crates/lakehouse-auth/src/permissions.rs:47` `PermissionSet`, `:54` `parse`, `:90` `has`.
- `rust/crates/lakehouse-api/src/policy.rs:127` `Policy`, `:147` `POLICY_TABLE`.
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs:49-74` `ToolSpec`, `:1963` `TOOLS`.
- `rust/crates/lakehouse-api/tests/ai_build_tool_list.rs:143`, which shows the fixture pattern.
- `rust/crates/lakehouse-api/src/routes/identity.rs`: role create and update, and the
  service identity routes.
- `src/features/admin/roles-page.tsx:185,209-215`, `service-identities-page.tsx:205-218`.
- `src/services/contracts/identity.ts:13-19,151`, `src/services/clients/identity.ts:107`.

## 3. Tasks

S1 (one commit each, in order):

- **T1 (D1, D2, D5, D6).** The catalogue module, `is_known`, the drift test, and the fixture
  with its equality test.
  - *Accept:*
    - `cargo test -p lakehouse-auth permission_catalog`
    - `cargo test -p lakehouse-api permission_catalog`
    - clippy for both crates with `-D warnings`
    - The drift test fails if one entry is removed. Show this in the handoff by quoting the
      failure from a throwaway local edit, not a commit.
- **T2 (D3, D4).**
  - The `GET /api/identity/permissions` route.
  - Validation on role and service identity create/update.
  - `sqlx::test` cases:
    - an unknown token gives 400 naming it;
    - `area:*` and `*:*` are accepted;
    - an existing unknown token survives an update;
    - the route is refused without `identity:read`.
  - *Accept:*
    - `cargo test -p lakehouse-api identity`
    - `cargo test -p lakehouse-api --test route_auth`
    - `cargo test -p lakehouse-api --lib route_policy_tests`
- **T3 (D7).** The contract, client, picker and both forms.
  - Bun tests:
    - checkboxes are grouped by area;
    - an unknown token shows as "Not recognised" and can be removed;
    - the saved payload is the comma string the API takes today;
    - there is no `query:run` anywhere in `src/`.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/features/admin`.

S2 (after `WEB-7` is merged):

- **T4.** SSL modes. *Accept:* `bun test src/features/connectors`.
- **T5.** Pipeline zones and columns. *Accept:* `bun test src/features/pipelines`.
- **T6.** Data quality, freshness and alert table and column pickers. *Accept:* `bun test
  src/features/governance src/features/alerts`.
- **T7.** The time zone picker and the access-rule roles checkboxes. *Accept:* `bun test
  src/features/dashboards src/features/catalog`.
- **T8.** The row filter grammar route and its hint. If T8 adds a Rust route, use the S1-style
  Rust checks (route_auth both ways).
  - *Accept:* `bun test src/features/catalog`, plus `cargo test -p lakehouse-api --test
    route_auth` if the route was added.

## 4. Out of scope

- Changing which values the server accepts. The one exception is D3's refusal of unknown
  permissions.
- The Invite user form, which belongs to `ADM-11`.
- `ADM-5` module switches.

## 5. Handoff (developer appends below, one entry per slice)

Each entry gives the commits, the exact commands with their counts, anything *not verified*
with the reason, and the output of `git status --short`, `git log --oneline origin/main..HEAD`
and `git ls-remote --heads origin <branch>`.

## 6. Review (planner)
