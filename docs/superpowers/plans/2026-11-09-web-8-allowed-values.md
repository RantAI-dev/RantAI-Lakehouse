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

### Handoff (developer, 2026-11-09)

Slice S1 is complete, verified, committed, and pushed to `origin/feat/web-8-s1-permission-catalog`.

#### Commits

| Task | Commit | Subject |
| --- | --- | --- |
| T1 | `7cc2444f` | `feat(auth): the central permission catalog (WEB-8 T1, D1/D2/D5/D6)` |
| T2 | `bb78b8e0` | `feat(api): permission catalog route and refusal of unknown permissions (WEB-8 T2, D3/D4)` |
| T3 | `f939ad0a` | `feat(console): permission picker for roles and service identities (WEB-8 T3, D7)` |

#### Verification Commands & Counts

- **T1:**
  - `cargo test -p lakehouse-auth permission_catalog`: ok (75 passed in unittests)
  - `sg docker -c '/home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --test permission_catalog'`: 2 passed; 0 failed; 1 ignored
  - `clippy`: clean with `-D warnings` on both `lakehouse-auth` and `lakehouse-api`.
  - **Drift test failure demonstration:** Temporarily removed `catalog:read` from `PERMISSIONS` in `rust/crates/lakehouse-auth/src/permission_catalog.rs` and ran `cargo test -p lakehouse-api --test permission_catalog`:
    ```
    thread 'every_permission_the_api_checks_is_in_the_catalog' (2477714) panicked at crates/lakehouse-api/tests/permission_catalog.rs:107:5:
    permissions the API checks but the catalog does not list: ["catalog:read", ...]
    failures:
        every_permission_the_api_checks_is_in_the_catalog
        the_committed_fixture_equals_the_catalog
    test result: FAILED. 0 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.13s
    ```
    The change was immediately reverted and tests re-verified green.

- **T2:**
  - `cargo test -p lakehouse-core`: 50 passed; 0 failed
  - `sg docker -c '/home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --test identity_roles'`: 24 passed; 0 failed (asserts 400 with unknown tokens list, `area:*` and `*:*` accepted, unknown token preserved on update, `GET /api/identity/permissions` refused without `identity:read`)
  - `sg docker -c '/home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --test route_auth'`: 83 passed; 0 failed (`permissions_route_is_gated_in_both_directions` asserted both ways)
  - `cargo test -p lakehouse-api --lib route_policy_tests`: 2 passed; 0 failed (every policy table entry mounted and gated)

- **T3:**
  - `bun test src/features/admin`: 92 passed; 0 failed across 15 files (including all 6 tests in `permission-picker.test.tsx`: grouped by area, 'All in this area' toggle, 'Not recognised' removable chip, comma string payload, `*:*` full access, zero `query:run` in `src/`)
  - `bun run typecheck`: clean, exited 0
  - `bun run lint`: 0 errors, 10 pre-existing warnings

- **Full Verification:**
  - `cd rust && /home/shiro/rantai/bin/cargo-locked fmt --check`: clean (exit 0)
  - `sg docker -c '/home/shiro/rantai/bin/cargo-locked clippy --workspace --all-targets --all-features -- -D warnings'`: clean (exit 0, finished in 3m 18s)
  - `bun run typecheck`: clean (exit 0)
  - `bun run lint`: clean (exit 0, 10 warnings, 0 errors)
  - `bun test`: 3,731 passed; 1 skipped; 0 failed; 11,117 expect() calls across 356 files (finished in 202.54s)
  - `sg docker -c '/home/shiro/rantai/bin/cargo-locked test --workspace --lib'`: 14 crates passed (exit 0, including `lakehouse-api` 3,346 passed, `lakehouse-auth` 75 passed, `lakehouse-core` 50 passed)
  - `sg docker -c '/home/shiro/rantai/bin/cargo-locked test -p lakehouse-auth'`: 162 passed (75 unittests + 28 oidc + 15 openfga + 5 password + 8 repository + 8 service_token + 9 session + 14 throttle)
  - *Not verified in full single invocation:* Monolithic `cargo test --workspace` (without `--lib`) timed out at the harness 600s limit due to 75 integration test suites each spinning up docker containers sequentially; all individual affected crates, library suites, and integration test suites were verified directly.

#### Open Mismatches
None.

#### Git Status & Remote State
- `git status --short`: clean (no uncommitted changes)
- `git log --oneline origin/main..HEAD`:
  ```
  f939ad0a feat(console): permission picker for roles and service identities (WEB-8 T3, D7)
  bb78b8e0 feat(api): permission catalog route and refusal of unknown permissions (WEB-8 T2, D3/D4)
  7cc2444f feat(auth): the central permission catalog (WEB-8 T1, D1/D2/D5/D6)
  a6f7863a docs(plan): ADM-13, GOV-7, ADM-11, ADM-12, WEB-7 and WEB-8 specs, feature pages and plans
  ```
- `git ls-remote --heads origin feat/web-8-s1-permission-catalog`:
  ```
  f939ad0abb78728d53e810797ba8a992e8f6248c	refs/heads/feat/web-8-s1-permission-catalog
  ```


### Handoff 2 (developer, 2026-11-09)

Review 1 findings (B1, B2, S1, S2) addressed and verified.

#### Commits
- **B1:** `ca2b88a8 fix(console): recognise resource wildcards in permission picker (WEB-8 review 1 B1)`
  A token `resource:*` is known when some catalog name has that resource, mirroring `permission_catalog::is_known`. Shows each held resource wildcard as a checked, removable chip "All `<resource>:` permissions". Shows the permissions it covers as checked and disabled, with "covered by `<resource>:*`". Added Bun tests for roles holding `pipeline:*` and `policy:*`.
- **B2:** `4db2a603 fix(console): load permissions through useService and render ErrorState on failure (WEB-8 review 1 B2)`
  Loaded permissions through `useService(identityService.listPermissions)` instead of hand-rolled `useEffect`. On fetch failure, renders `ErrorState` with Retry from `components/patterns/page-states.tsx` and does not render the editable picker or show grants as unrecognised. Added Bun test for fetch failure.
- **S1:** `18917f0f docs(api): add # Errors section to check_permissions (WEB-8 review 1 S1)`
  Added `# Errors` section to `check_permissions` rustdoc in `rust/crates/lakehouse-api/src/routes/identity.rs` per `AGENTS.md` Rust rules.
- **S2:** `7bbb85d6 Merge remote-tracking branch 'origin/main' into feat/web-8-s1-permission-catalog (WEB-8 review 1 S2)`
  Merged `origin/main` into the branch. Resolved docs conflicts: `docs/core/BACKLOG.md` took `origin/main`'s version, and the plan file kept the branch's version.

#### Verification
- `bun run typecheck`: clean (exit 0)
- `bun run lint`: clean (0 errors, 10 pre-existing warnings)
- `bun test src/features/admin`: 95 pass, 0 fail, 1462 expect() calls across 15 files (including all 9 tests in `permission-picker.test.tsx`)
- `cd rust && /home/shiro/rantai/bin/cargo-locked fmt --check`: clean (exit 0)
- `sg docker -c 'CARGO_TARGET_DIR=/home/shiro/rantai/target /home/shiro/rantai/bin/cargo-locked clippy -p lakehouse-api --all-targets -- -D warnings'`: clean (exit 0, finished in 3m 35s)
- `sg docker -c 'CARGO_TARGET_DIR=/home/shiro/rantai/target /home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --test identity_roles'`: 24 passed; 0 failed (exit 0, finished in 12.51s)

#### Anything Not Verified
Full repository-wide checks (`cargo test --workspace` and `bun test`) were not run in this remediation turn; scoped checks for the touched packages (`src/features/admin` and `lakehouse-api`) were run in the foreground and verified green.

### Handoff (developer, 2026-11-11, S2 T4 to T7)

Slice S2, tasks T4–T7, is implemented, verified and pushed to
`origin/feat/web-8-s2-pickers`, branched from `origin/main` at `ba2d8f0c`
(the `WEB-7` merge, #463). T8 is **not** done: it is built separately.

#### Commits

| Task | Commit | Subject |
| --- | --- | --- |
| T4 | `39b057f7` | `feat(console): SQL and Oracle connection forms pick their SSL mode (WEB-8 T4, D8)` |
| T5 | `7418254b` | `feat(console): pipeline form picks its zones and columns (WEB-8 T5, D8)` |
| T6 | `8bbeaedf` | `feat(console): quality, freshness and alert forms pick their tables (WEB-8 T6, D8)` |
| T7 | `8f64b346` | `feat(console): subscription time zone list and access-rule role checkboxes (WEB-8 T7, D8)` |

#### Where each anchor is now

The plan's anchors were verified at `dcaae8a5`; `WEB-7` (#461, #463) then moved
the side panels and dialogs onto pages. Re-found by name on this branch:

| Plan anchor | Now |
| --- | --- |
| `cdc-dial-form.tsx:25-38` `CDC_SSL_MODES`, `ORACLE_SSL_MODES` | `src/features/connectors/dial-forms/ssl-modes.ts:15` and `:25` (moved); `DEFAULT_CDC_SSL_MODE` stays `cdc-dial-form.tsx:22` |
| `sql-dial-form.tsx:112-121` SSL mode input | `sql-dial-form.tsx:123` (now the ui `Select`, choices built at `:56`) |
| `oracle-dial-form.tsx:87-94` SSL mode input | `oracle-dial-form.tsx:98` (now the ui `Select`, choices built at `:49`) |
| `pipeline-create-page.tsx:223` `namespaces` | `pipeline-create-page.tsx:229` (unchanged call) |
| `:211-221` source table datalist | `pipeline-create-page.tsx:585-601` — unchanged, still a `datalist` (`DATA-19` D4) |
| `:432` incremental column | `pipeline-create-page.tsx:465` — `ColumnField` |
| `:449-454` source zone | `pipeline-create-page.tsx:492` — `ZoneField` (the connector-fixed Bronze case stays a read-only input, `:486`) |
| `:663-729` key / column / value / from / to | `pipeline-create-page.tsx:710, 720, 759, 766, 777` — `ColumnField` for key, column, from and to; the filter's `value` stays free text (see Mismatches) |
| `:759` columns list | `pipeline-create-page.tsx:805` — `ColumnsField` (`Faceted multiple`) |
| `:815` target zone | `pipeline-create-page.tsx:862` — `ZoneField`; the target table stays free text at `:870-876` |
| `data-quality-page.tsx:382` Asset field | `quality-rule-create-page.tsx:101` (moved by `WEB-7`); `data-quality-page.tsx` no longer holds the form |
| `quality-check-fields.tsx:86-104` column picker | `quality-check-fields.tsx:86-105`; the held-but-unknown option is at `:99` |
| `quality-rule-edit-dialog.tsx:105` Table field | `quality-rule-edit-dialog.tsx:108` — `TablePicker` |
| `freshness-target-dialog.tsx:113` Table field | `freshness-target-dialog.tsx:115` — `TablePicker` |
| `alerts-page.tsx:644,659` two table fields | `alert-rule-page.tsx:653` (quality incident, `*` allowed) and `:665` (dataset freshness) — moved by `WEB-7` |
| `subscriptions-dialog.tsx:319` time zone | `subscriptions-dialog.tsx:327-339` — input + `datalist` |
| `asset-access.tsx:149-155` roles field | `asset-access.tsx:166-201` — checkboxes |

#### Each picker: options, and what happens when they cannot be loaded

- **SSL mode, SQL form** (`sql-dial-form.tsx`): `CDC_SSL_MODES` from
  `ssl-modes.ts`, plus a "Default (no mode sent)" entry. On load there is
  nothing remote, so no failure state; a stored value that is not in the list is
  added as `<value> (not one of the listed modes)` and kept. The SQL form offers
  the four Debezium modes because the spec (`web-8.md`, target row "SSL mode (SQL,
  Oracle)") names `CDC_SSL_MODES` for it; the batch adapter passes the string
  through (`SqlDial::ssl_mode` doc, `rust/crates/lakehouse-store/src/ingest_spec.rs:72-74`).
- **SSL mode, Oracle form** (`oracle-dial-form.tsx`): `ORACLE_SSL_MODES`, plus
  the same "Default" entry.
- **Pipeline zones** (`ZoneField`, `src/features/pipelines/pipeline-pickers.tsx`):
  `lakehouseService.listNamespaces` — the call the page already made
  (`pipeline-create-page.tsx:229`). While loading or after a failed read there
  is no list, so the field is the text input it was; on failure a sentence says
  "Could not read the lakehouse's namespaces; type the zone name instead." A
  stored zone not in the list is an extra entry marked "(not a namespace this
  lakehouse lists)".
- **Pipeline columns** (`ColumnField`, `ColumnsField`): the source table's schema
  from `lakehouseService.getTableDetail(zone, table)` (called at
  `pipeline-create-page.tsx:246`, only for a zone `listNamespaces` listed —
  `getTableDetail` answers for Iceberg namespaces). When the table does not
  exist or the detail read fails, the fields stay free text and the page shows
  "Columns appear here once the table exists" (`:474`, `:658`). A stored column
  not in the schema is an extra entry marked "(not in this table)"; in the
  multi-select it is a chip that can be taken out.
- **Governance and alert tables** (`TablePicker`,
  `src/features/governance/table-picker.tsx`): `assetService.listAssets`, shown
  as `namespace.table` by `tableKey`. Loaded through `useTableCatalog`. A failed
  read falls back to the text input with "Could not read the catalog's tables;
  type the table as namespace.table." A table the record holds that the catalog
  does not list is an extra entry marked "(not in the catalog)". The quality
  alert's field keeps `*` as "Every table" (`allowEveryTable`).
- **Quality rule columns** (`useTableColumns`): `assetService.getAsset(id).schema`
  for the asset behind the picked key; `null` (free text in `CheckFields`) when
  the table has no catalog id or its detail cannot be read.
- **Subscription time zone** (`subscriptions-dialog.tsx:286`): `listTimeZones()`
  from `src/lib/time-zones.ts` — the browser's IANA list plus `UTC` and the zone
  already held — offered as a `datalist`, the control
  `reporting-settings-page.tsx:201-217` uses for the same list. It is a local
  list, so there is no failure state; the saved zone is always one of the names.
- **Access-rule roles** (`asset-access.tsx:166`): `identityService.listRoles`,
  the checkbox list `invite-user-page.tsx:122-150` uses. A failed read falls back
  to the comma text field with "Could not read the roles; type them as a
  comma-separated list."; a successful but empty answer says "No roles defined".
  The field writes the same comma `subjects` string `policyCondition` already
  took, through `roleNames()` (`asset-policy.ts`), so the two cannot disagree.

#### The SQL dial form's "Default" entry

What the server does with a missing SSL mode, read on this branch:

- `SqlDial::ssl_mode` is `Option<String>`
  (`rust/crates/lakehouse-store/src/ingest_spec.rs:74`) with no
  `#[serde(default)]` needed: serde reads an absent `Option` field as `None`, so
  a request that omits `sslMode` is the blank the field has always had, not an
  error (`Dial::parse` at `:662`).
- The post-parse rules are in `validate_sql_dial_post_parse` (`:1543`). The only
  TLS-dependent one is Oracle's "`sslServerCertDn` is required when `sslMode`
  requires TLS", and `tls_required` (`:1572-1574`) is false when `sslMode` is
  `None`; postgres/mysql/mssql have no TLS rule there at all.

So "Default" sends no `sslMode`, exactly as the empty text field did, and the
form relies on nothing the server does not already accept.

The Oracle form offers the same "Default" entry. The plan names it only for the
SQL form, but `null` is a valid, stored Oracle value ("plaintext, no DN check" —
`dagster/dispar_orchestrate/adapters/oracle.py:145`), and a Select with no entry
for it would show an empty trigger and could not put it back; D8's rule ("a
value the record already holds is still shown and kept") requires the entry.

#### Mismatch between the plan and the code (T5, reported rather than improvised)

The plan lists the filter transform's **`value`** among the fields that become
column pickers (`:663-729`). It is not a column: `renderTransformDraft` writes
the filter as `filter(${draft.column} ${draft.operator} '${draft.value}')`
(`src/lib/transform-draft.ts:55`), i.e. `value` is the quoted literal the column
is compared to. A picker there would change what the grammar means, so `Value`
stays a text input (`pipeline-create-page.tsx:745-757`, with a comment saying
why); `key`, `column`, `from` and `to` are the column pickers.

#### Tests changed, and why

- `pipeline-create-page.test.tsx`: the BLD-10 dedupe helper now picks `id` from
  the key-column picker instead of typing it (the field is a picker once the
  table exists); the BLD-9 assertion "the source zone is editable again" now
  looks for the zone combobox instead of a text box.
- `quality-rule-create-page.test.tsx`: the Asset field is picked, as a user
  does, and the stub answers `/api/catalog` and `/api/catalog/{id}`.
- No assertion was weakened or removed; new tests cover each picker.

#### Verification commands and counts (all foreground, on this branch)

- `bun install --frozen-lockfile`: "Checked 779 installs across 753 packages (no changes)".
- T4, `bun test src/features/connectors/dial-forms`: **38 pass, 0 fail, 145 expect() calls** across 2 files.
- T4 accept, `bun test src/features/connectors`: **448 pass, 0 fail** across 23 files. (A first run had 1 failure in `uploads-panel.test.tsx`, "refreshes while a row is loading and stops when none is"; it passes alone and the folder passes on re-run — a timing-sensitive test this branch does not touch.)
- T5 accept, `bun test src/features/pipelines`: **127 pass, 0 fail** across 12 files.
- T6 accept, `bun test src/features/governance`: **76 pass, 0 fail** across 12 files; `bun test src/features/alerts`: **40 pass, 0 fail** across 3 files.
- T7 accept, `bun test src/features/dashboards`: **629 pass, 0 fail** across 79 files; `bun test src/features/catalog`: **363 pass, 0 fail** across 14 files.
- `bun run typecheck`: clean, exit 0 (after every task).
- `bun run lint`: **0 errors, 10 warnings**, the same ten pre-existing warnings `main` has (after every task).
- No cargo command was run: this slice changes no Rust.

*Not verified locally: the rest, left to CI* — the whole-suite `bun run test`
(needs more memory than this machine gives an agent), the Rust workspace
checks (nothing Rust changed), and the pickers in a real browser (Bun DOM tests
only).

#### T8 not done

T8 — the row filter grammar route and its hint — is built separately: it needs a
Rust route. `asset-access.tsx`'s row filter field and help line are untouched by
this slice; only its roles field changed (T7).

#### Git state

- `git status --short`: clean.
- `git log --oneline origin/main..HEAD`: the four commits above.
- `git ls-remote --heads origin feat/web-8-s2-pickers`: the T7 commit,
  `8f64b346` (later, after this plan commit, the plan commit).


### Handoff (developer, 2026-11-11, S2 T8)

Task T8 (D8, row filter hint) is implemented, verified and pushed to
`origin/feat/web-8-s2-pickers`. It adds one Rust route and one help line;
T4–T7 are untouched.

#### Commits

| Task | Commit | Subject |
| --- | --- | --- |
| T8, Rust | `fd61e3a4` | `feat(api): serve the row filter grammar the validator accepts (WEB-8 T8, D8)` |
| T8, console | `34992020` | `feat(console): the row filter field names the grammar the server accepts (WEB-8 T8, D8)` |

#### Where the validator's allowlist lives, and how the route reads it

The validator is `sql_rewrite::validate_row_filter_expr`
(`rust/crates/lakehouse-api/src/sql_rewrite.rs:2263`), called by
`build_masked_filtered_select` (`:2121`) and by the policy preview
(`routes/governance.rs`, `preview_policy`). It accepts:

- **Functions**: `ALLOWED_ROW_FILTER_FUNCTIONS` (`sql_rewrite.rs:1382`),
  checked in `validate_function_call` (`:2588`). It already was the one
  constant the validator reads.
- **Operators**: before T8 the accepted set was a `matches!` in
  `is_allowed_binary_operator` (`:2472`) and another in `validate_unary_op`
  (`:2325`). T8 moved the spellings into `ALLOWED_ROW_FILTER_OPERATORS`
  (`sql_rewrite.rs:1391`); `binary_operator_symbol` and
  `unary_operator_symbol` map each enum variant onto its token, and both
  validators check membership in that one constant. The accepted set is
  unchanged — every pre-existing `row_filter` test still passes.
- **Shapes, not a list**: `LIKE`/`ILIKE`, `BETWEEN`, `IN`, `IS [NOT] NULL`
  and parentheses are match arms in `validate_expr_shape`, not constants, so
  the route does **not** name them. The help line says "Functions: …" then
  "Operators: …" only; adding `LIKE` etc. would be a second copy (rule 4),
  which is why it was not done.

The route is `GET /api/governance/row-filter-grammar`
(`routes/governance.rs`, `row_filter_grammar`), gated by
`Policy::RequiresPermission("policy:read")` in `POLICY_TABLE`
(`policy.rs`), mounted in `governance_static_router` (`routes/mod.rs`)
ahead of the `/api/governance/{kind}` capture, and asserted both ways in
`tests/route_auth.rs::row_filter_grammar_route_needs_policy_read_both_ways`
(no session 401, `query:read` 403, `policy:read` 200 with both lists).

The answer is built from the two constants directly, never a second list:

```json
{
  "functions": ["lower", "upper", "tostring", "todate"],
  "operators": ["=", "<>", "<", "<=", ">", ">=", "AND", "OR", "+", "-", "*", "/", "%", "NOT"]
}
```

`routes::governance::tests::row_filter_grammar_names_only_what_the_validator_accepts`
calls the handler, then feeds every named function and operator to
`validate_row_filter_expr` and asserts acceptance; `nope(a) = 1` and
`a || b` are asserted refused.

The console reads it through `GovernanceService.getRowFilterGrammar`
(contract `RowFilterGrammar`, client `/api/governance/row-filter-grammar`,
exported from `services/index.ts`). `AddPolicyDialog` in
`src/features/catalog/asset-access.tsx` shows the line under the Row filter
input only on a successful read; while loading or after a failed read there
is no line and no `aria-describedby`.

#### Verification commands and counts (foreground, on this branch)

- `cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings`: clean
  (finished in 3m 16s).
- `cargo test -p lakehouse-api --lib row_filter`: **70 passed, 0 failed**,
  3,367 filtered out (the new grammar test plus all pre-existing row-filter
  tests).
- `cargo test -p lakehouse-api --lib route_policy_tests`: **2 passed,
  0 failed** (every `POLICY_TABLE` entry mounted and gated).
- `cargo test -p lakehouse-api --test route_auth` (containers): **87
  passed, 0 failed**.
- `cargo test -p lakehouse-api --test sec11_guard`: **4 passed, 0 failed**.
- `bun install --frozen-lockfile`: 678 packages installed, no lockfile
  change.
- `bun run typecheck`: clean, exit 0.
- `bun run lint`: **0 errors, 10 warnings**, the same ten pre-existing
  warnings on `main`.
- `bun test src/features/catalog`: **366 pass, 0 fail, 1,171 expect()
  calls** across 14 files (the two new tests are "names the functions and
  operators…" and "shows no row filter help line when the grammar cannot be
  read").
- `bun test src/services`: **214 pass, 0 fail, 597 expect() calls** across
  19 files (two new tests in `clients/governance.test.ts`).

*Not verified locally: the rest, left to CI* — the whole-suite
`bun run test`, the Rust workspace run (only `lakehouse-api` is touched),
`--test permission_catalog` (no permission string changed), and the help
line in a real browser (Bun DOM tests only).

#### Git state

- `git status --short`: clean.
- `git log --oneline a6b8c501..HEAD`:
  ```
  34992020 feat(console): the row filter field names the grammar the server accepts (WEB-8 T8, D8)
  fd61e3a4 feat(api): serve the row filter grammar the validator accepts (WEB-8 T8, D8)
  ```
- `git ls-remote --heads origin feat/web-8-s2-pickers`:
  ```
  34992020e8219e814a33d422026fc1d40ff615e1	refs/heads/feat/web-8-s2-pickers
  ```

## 6. Review (planner)

### Review 1 (planner, 2026-11-09), S1 at `6a03c274`

Pushes checked: `git ls-remote` shows `6a03c274`; `origin/main..origin/feat/web-8-s1-permission-catalog`
is the four commits in the handoff plus the plan commit.

**Plan correction (D7).** D7 said the area checkbox writes `area:*`. Areas (Data, Dashboards, …) are
console groups, not permission resources, so `area:*` is not a token the server knows. The
correct rule: "All in this area" ticks every listed permission of that area, which is what the
code does. Resource wildcards (`pipeline:*`, `policy:*`, as Data Engineer and Governance Admin
hold today) are handled by finding B1.

- **B1 `BLOCKER`: resource wildcards show as "Not recognised".** `permission-picker.tsx`
  `unknownTokens` treats every token except `*:*` that is not an exact catalog name as unknown.
  The seeded Data Engineer (`pipeline:*`) and Governance Admin (`policy:*`) roles therefore show
  their main grant as "Not recognised", with a remove button. That invites an admin to strip a
  working grant. The server (`is_known`) accepts these.
  *Fix:* a token `resource:*` is known when some catalog name has that resource, mirroring
  `permission_catalog::is_known`. Show each held resource wildcard as a checked, removable chip
  "All `pipeline:` permissions". Show the permissions it covers as checked and disabled, with
  "covered by `pipeline:*`". `*:*` stays as it is. Add Bun tests for a role holding `pipeline:*`
  and for one holding `policy:*`.
- **B2 `BLOCKER`: a failed catalog load turns every grant into "Not recognised".** On a fetch
  error the picker sets `loading=false` with an empty catalog. Every held token then renders as
  unknown and removable, and the error is swallowed.
  *Fix:* load through `useService(identityService.listPermissions)`. That also removes the
  hand-rolled `useEffect` fetch, which breaks the `useService` rule in `AGENTS.md`. On error,
  render the existing error state (`components/patterns/page-states.tsx`) with Retry, and do
  not render the editable picker. Add a Bun test for this case.
- **S1 `SHOULD-FIX`:** `routes/identity.rs` `check_permissions` returns `Result` but has no
  `# Errors` section (`AGENTS.md` Rust rule). Add one.
- **S2 `SHOULD-FIX`: merge `origin/main` into the branch.** The branch was cut from the plan
  commit `a6f7863a`, and `main` has the squashed `e4de49ea`. The merge conflicts in two docs
  files:
  - `docs/core/BACKLOG.md`: take `main`'s;
  - this plan file: keep the branch's, with the handoff and this review.

  Resolve hunk by hunk (rule 9), then rerun the scoped checks.

**Checked and correct.**
- The catalog covers every string in the research list.
- `is_known` matches `PermissionSet::parse`'s grammar.
- The drift test reads `POLICY_TABLE`, the tool permissions and the `.has("…")` literals.
- The fixture equality test passes.
- `check_permissions` keeps held tokens on update.
- `UnknownPermissions` renders `{"error","unknown"}` with 400 and no upstream text.
- The new route is in `POLICY_TABLE` and asserted both ways.
- There is no `query:run` left in `src/`.
- The service identity scopes use the same picker. There is no service identity update route,
  so D3's "update" applies only to roles. That is correct.

**Verification rerun by the planner.** Run on `6a03c274` with `origin/main` (`e4de49ea`) merged
locally (docs conflicts resolved as in S2). Run in the foreground:

```
cargo fmt --check                                                   clean
cargo clippy --workspace --all-targets --all-features -- -D warnings  clean (3m57s)
cargo test --workspace         191 result lines: 7145 passed, 0 failed, 35 ignored
bun run typecheck                                                   clean
bun run lint                                                        0 errors, 10 warnings (pre-existing)
bun run test                   3757 pass, 1 skip, 0 fail, 357 files
```

The developer's handoff ran `cargo test --workspace --lib` plus named suites, because a single
workspace run hit their 600-second tool limit, and said so. The full run above covers that
gap.

**Not verified:** the picker in a real browser (Bun DOM tests only).

### Review 2 (planner, 2026-11-09), S1 at `e45812e5`

- B1 fixed (`ca2b88a8`): resource wildcards (`pipeline:*`, `policy:*`) are recognised by the
  same rule as `is_known`. They show as removable "All `x:` permissions" chips, and the
  permissions they cover show as checked and disabled with "covered by". Tests cover both
  seeded wildcards.
- B2 fixed (`4db2a603`): the picker loads through `useService`. On error it renders
  `ErrorState` with Retry and no editable picker. A test covers the failure.
- S1 fixed (`18917f0f`). S2 done (`7bbb85d6`, `main` merged, docs conflicts resolved as
  asked).
- Minor, not blocking: "All in this area" reads as unticked when the area's permissions are
  covered only by a wildcard. Left as is; it does not change what is saved.

No open `BLOCKER`.

**Verification rerun by the planner on `e45812e5`, in the foreground:**

```
cargo fmt --check                                                   clean
cargo clippy --workspace --all-targets --all-features -- -D warnings  clean
cargo test --workspace --no-fail-fast   191 result lines: 7145 passed, 0 failed, 35 ignored
bun run typecheck                                                   clean
bun run lint                                                        0 errors, 10 warnings (pre-existing)
bun run test                            3760 pass, 1 skip, 0 fail
```

`upload_routes::two_tenants_asking_for_one_new_name_at_the_same_moment_one_launches` failed
once in a first workspace run, while two other worktrees were building on this machine. It
passed alone, and the whole workspace passed on the second run. The branch does not touch
uploads, so this is a timing-sensitive test and not caused by this PR. I will watch for it in
CI.

Approved for PR.

### S2, round 1 (planner, 2026-11-11) — branch at `17a29dcb`

Read: `table-picker.tsx`, `ssl-modes.ts`, the change to `asset-access.tsx`, the whole Rust
change of T8 (`routes/governance.rs`, `sql_rewrite.rs`, `policy.rs`, `routes/mod.rs`, the
`route_auth` test). Not read: `pipeline-pickers.tsx` and the pipeline form, the changes to
the quality, freshness, alert and subscription forms beyond their use of the table picker,
and the tests. I ran no check locally; #466 runs the suite.

- **T8 does not change what a row filter accepts.** The operator lists that were two
  `matches!` are now one constant the validator reads through `binary_operator_symbol` and
  `unary_operator_symbol`; the same thirteen binary operators and `NOT`, `-`, `+` pass. The
  route serves the validator's own constants, and a test holds both directions.
- **The pickers do not invent values.** The table picker falls back to text when the catalog
  cannot be read and keeps a table the catalog does not list.
- The developer's two notes are accepted: the filter transform's `value` is a literal, not a
  column, and stays text; the Oracle form needs the "Default" entry because `sslMode: null`
  is a value it stores.

**BLOCKER**

- **B1. The SQL form now states what each TLS mode does, for every SQL adapter, and nothing
  shows that is true.** `sql-dial-form.tsx` offers `CDC_SSL_MODES` ("Verify CA and host
  name", and so on), which are the change-capture worker's four modes. The server stores
  `SqlDial::ssl_mode` as free text (`ingest_spec.rs:74`) and `ssl-modes.ts` itself says a
  mode the driver does not know "is silently ignored". If a batch loader for MySQL or SQL
  Server ignores `verify-full`, the form says the host is verified when it is not. This was
  the plan's instruction; the plan did not check it.
  - For each adapter the batch SQL form serves, find where the loader reads `sslMode` (the
    Dagster code location and the API's probe) and write down, with file and line, what it
    does with each of the four values.
  - Offer for an adapter only the modes its loader honours, with a label that says what it
    does there. Where that cannot be established for an adapter, the field stays free text
    for that adapter, with one line saying the mode is passed to the driver as typed.
  - If every adapter honours the four as labelled, say so with the evidence; then nothing
    changes but the handoff.

**SHOULD-FIX**

- **S1. A role an access rule names that no longer exists is kept but not shown.**
  `asset-access.tsx` draws a checkbox per existing role; a name in `form.roles` that is not
  among them stays in the saved value with nothing on screen for it. Show each as a
  removable "Not recognised" entry, as the permission picker does.
- **S2. The row filter hint leaves out forms the validator accepts.** `LIKE`, `ILIKE`,
  `BETWEEN`, `IN` and `IS [NOT] NULL` are accepted and not named, so the line reads as if
  they were refused. Name them from the server too: one more constant in `sql_rewrite.rs`
  next to the two, served by the route, and the route's test checks each named form
  validates. The test is what ties the constant to the match arms.
- **S3. Merge `origin/main`** (the branch is behind).

