# ADM-13: each role sees only the pages it may use

Backlog `ADM-13` (Feature, Administration & Security, P1). Spec `docs/core/specs/adm-13.md`,
feature page `docs/core/features/pages-per-role.md`. Base: `main` at `dcaae8a5` (anchors
verified there). **Waits for `WEB-8` S1**, the permission list: the new permissions are added
to it.

Branch `feat/adm-13-pages-per-role`, off the latest `origin/main` after `WEB-8` S1 merges. One
PR. Planner and reviewer: Claude Opus. Developer: another agent.

## 1. Decisions

Owner, 2026-11-09: D1 to D3. Planner: the rest.

- **D1. An Assistant only user asks and reads, and changes nothing.**
- **D2. New permissions.** Add these to `permission_catalog` (`WEB-8` S1):

  | Permission | Area | Opens |
  | --- | --- | --- |
  | `assistant:use` | Assistant | The assistant, as today. Tools still need their own permissions |
  | `assistant:ask` | Assistant | The assistant in Ask mode only. Its read tools may read tables, metrics and the catalog that the person may read (masks, row filters and `GOV-7` still apply) |
  | `ops:read` | Operations | System status (`/api/ops/{kind}`, `/api/ops/logs`'s floor) |
  | `alert:read` | Operations | The Alerts list (`GET /api/alerts` and its other GETs) |
  | `governance:read` | Governance | Data Quality and Audit Log lists (`GET /api/governance/{kind}`) |

- **D3. Migration `NNNN_pages_per_role.sql`.** Use the next free number on `origin/main`
  (`0132` at `dcaae8a5`), and check for a clash before committing. It needs a why-header. It
  does three things:
  1. Append `,assistant:use,ops:read,alert:read,governance:read` to every role row that holds
     none of them and is neither `*:*` nor named `Dashboard Viewer`. The `UPDATE` must be
     targeted (`AGENTS.md`): the `WHERE` clause guards on the current `permissions` value so
     a re-run adds nothing twice. Custom roles are included, so nobody else loses a page.
  2. Insert the role **Assistant only** with `assistant:ask,dashboard:read` and a plain
     description. Use the next seeded UUID after `…000000000007` (the last at `dcaae8a5`),
     with `ON CONFLICT (name) DO NOTHING`.
  3. Leave **Dashboard Viewer** at `dashboard:read`. It loses the assistant (owner, D2 on the
     feature page).

  **Analyst** keeps its permissions plus D3.1. No other seeded role changes.
- **D4. Any-of policy.**
  - Add `Policy::RequiresAnyPermission(&'static [&'static str])` (`policy.rs:127`).
  - `auth_gate` (`policy.rs:~1262`) refuses with 403 unless the principal `has` at least one.
  - `tests/route_auth.rs` gets both directions:
    - `zero_permission_principal_…` (`:146`) treats the new variant as gated;
    - a new test walks every `RequiresAnyPermission` row and asserts that each listed
      permission alone passes and that a principal with an unrelated permission is refused.
  - The route-completeness test (`routes/mod.rs:2126`) still passes.
- **D5. Server rows that change** (`policy.rs`):
  - Every `/api/ai/*` row (`:821-834`: chat, tool, sessions, build-status, terms) becomes
    `RequiresAnyPermission(&["assistant:use", "assistant:ask"])`. Update the comment above
    them.
  - `GET /api/ops/{kind}` and `GET /api/ops/logs` (`:313-314`) become
    `RequiresPermission("ops:read")`. The handler's stricter `ops:logs` stays.
  - `GET /api/alerts` (`:412`) and the other alert GETs become `RequiresAnyPermission(&["alert:read",
    "alert:write"])`. Chart alert routes from `BI-19` C3, where anybody who can view a chart
    may save an alert on it, are **not** changed.
  - `GET /api/governance/{kind}` (`:391`) becomes `RequiresPermission("governance:read")`.
  - **Before changing any of these rows**, grep `src/` for every caller of that route. If a
    page open to Dashboard Viewer or Assistant only calls it, such as a dashboard freshness
    badge calling `/api/ops/*`, **stop and report** instead of changing the row. The handoff
    lists each changed row with its callers.
- **D6. Ask-only assistant** (`routes/ai/`):
  - Introduce `AssistantAccess::{Full, AskOnly}`, decided once per request from the principal:
    `Full` if `has("assistant:use")`, else `AskOnly` if `has("assistant:ask")`. The route
    policy already refused anyone with neither.
  - `AskOnly` forces Ask mode: the chat (`ai/mod.rs:206`) and `/api/ai/tool` (`:1338`) pass
    `is_build = false` to `gate::decide_with` (`gate.rs:732`), whatever the request asks.
  - `tool_schemas_for` (`registry.rs:2901`) hides every tool that is not `Risk::Read` from an
    `AskOnly` caller.
  - `gate::permission_granted` (`gate.rs:711`): for a `Risk::Read` tool whose permission is
    `query:read` or `catalog:read`, holding `assistant:ask` also grants it. At `dcaae8a5`
    these tools are `run_sql`, `list_datasets`, `lakehouse_overview`, `describe_dataset` and
    the other Read tools on those two permissions; the handoff lists them all. Nothing else is
    widened, and `lineage:read` tools are not included.
  - The same substitution applies inside handlers that a read tool reaches and that check
    those permissions themselves. One is `data_map_access` (`ai/mod.rs:362-368`): `assistant:ask`
    counts as `catalog:read` + `query:read` there. Grep the tool dispatch paths for `.has("query:read")`
    and `.has("catalog:read")`, change each one through **one** helper (for example
    `assistant_reads(principal, perm)`), and list them in the handoff. Duplicated guards are a
    finding (rule 4).
  - Headless digital-employee runs (`agents.rs:1158`) are unchanged.
- **D7. One access map in the console.**
  - `src/components/app-shell/nav-config.ts`: `NavItem` (`:26`) gains `requires: readonly
    string[]`, any-of. An empty array means any signed-in user.
  - Add a `module?: string` placeholder only if `ADM-5` has merged by then; otherwise leave it
    out.
  - New `ROUTE_ACCESS: readonly { pattern: string; requires: readonly string[] }[]` in the same
    file. It covers **every** non-public, non-redirect `src/app/**/page.tsx`, including
    secondary routes such as `/data/assets/[assetId]`, `/lineage`, `/gold-exports`,
    `/agents/runs/[runId]` and `/account/change-password` (any signed-in user).
  - Derive each entry from the GET route the page loads (`policy.rs`). The research table in
    this plan's section 2 is the starting point. Dashboards uses `dashboard:read`, which is
    `dashboard_access.rs:64` `ENTRY_PERMISSION`. Admin pages use `identity:read`, and Settings
    uses `settings:write` or `identity:read`.
  - `canOpen(pathname, hasPermission): boolean`. An **unlisted path returns false** (fail
    closed).
  - A Bun test globs `src/app/**/page.tsx` and fails if a non-public, non-redirect page has no
    `ROUTE_ACCESS` entry.
  - A second test reads `rust/crates/lakehouse-api/tests/fixtures/permissions.json` (`WEB-8`
    D6) and fails if any `requires` string is not in it.
- **D8. Sidebar, palette, guard, dock.**
  - `app-sidebar.tsx:59` and `command-palette.tsx:88` filter `NAV_GROUPS` with `canOpen` and
    drop empty groups.
  - Palette quick actions get `requires`:
    - Ask AI → assistant;
    - Add a chart → `dashboard:write`;
    - Export YAML → `dashboard:read` (it is a GET of `/api/dashboard/export`);
    - Browse all dashboards → `dashboard:read`;
    - Change theme → none.
  - New `src/components/app-shell/route-guard.tsx`, mounted in `app-frame.tsx` around the page
    (`:45-56`, after authentication).
    - When `canOpen` is false it renders `PermissionState` (`page-states.tsx:112`), so the
      page component never mounts and makes no request.
    - Exception: `/` for a user without an assistant permission redirects to the first
      `canOpen` nav item. A Dashboard Viewer lands on `/dashboards`. A user with nothing gets
      `PermissionState`.
  - `CopilotProvider`/dock (`app-frame.tsx:38`) renders only with `assistant:use` or
    `assistant:ask`.
  - All of it uses `useAuth().hasPermission` (`auth-provider.tsx:169`); there is no second
    matcher.
- **D9. Admin console.** The role picker from `WEB-8` S1 shows the new permissions with their
  descriptions automatically. No other UI is needed.

## 2. Anchors (verified at `dcaae8a5`)

- `rust/crates/lakehouse-api/src/policy.rs:127` `Policy`, `:147` `POLICY_TABLE`, `:313-314`
  ops, `:391` governance `{kind}`, `:412` alerts, `:821-834` ai, `:~1262` `auth_gate`, `:1284`
  the 403 response.
- `rust/crates/lakehouse-api/tests/route_auth.rs:146`, `:193`.
- `rust/crates/lakehouse-api/src/routes/ai/gate.rs:711` `permission_granted`, `:732`
  `decide_with`.
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs:1963` `TOOLS`, `:2901`
  `tool_schemas_for`.
- `rust/crates/lakehouse-api/src/routes/ai/mod.rs:206` chat, `:362-368` `data_map_access`,
  `:964`, `:1338`, `:1366`.
- `rust/migrations/0002_seed_identity.sql:38-46` seeded roles.
- `src/components/app-shell/nav-config.ts:26,39,77-168`; `app-sidebar.tsx:59`;
  `src/components/command-palette.tsx:88,~286-303`; `app-frame.tsx:38,45-56`;
  `src/features/auth/auth-provider.tsx:146-151,169`;
  `src/components/patterns/page-states.tsx:112`.

## 3. Tasks (one commit each, in order; Rust first, then the console)

- **T1 (D2, D3).** The catalogue entries, the migration, and a `sqlx::test` for the migration:
  - an Analyst role gets the four permissions;
  - Dashboard Viewer does not;
  - a custom role gets them;
  - a `*:*` role is untouched;
  - Assistant only exists;
  - running the `UPDATE` twice adds nothing twice.
  - Regenerate `permissions.json`.
  - *Accept:* `cargo test -p lakehouse-auth permission_catalog`, `cargo test -p lakehouse-api
    permission_catalog`, and the migration test.
- **T2 (D4, D5).** The any-of policy, the changed rows, and the `route_auth` tests in both
  directions.
  - *Accept:* `cargo test -p lakehouse-api --test route_auth`, `cargo test -p lakehouse-api
    --lib route_policy_tests`, and clippy for `lakehouse-api`.
- **T3 (D6).** Ask-only.
  - Tests:
    - an `assistant:ask`-only principal gets only Read tools in the schema list;
    - a write tool call is refused with the Ask-mode message even when the request says build
      mode;
    - `run_sql` is granted to `assistant:ask` without `query:read`;
    - `run_sql` is still refused to a principal with neither;
    - `data_map_access` gives `Full` to `assistant:ask`;
    - a principal with only `assistant:use` and no `query:read` is still refused `run_sql`, as
      today.
  - `tool_schemas.json` must not change. If it does, explain why in the commit.
  - *Accept:* `cargo test -p lakehouse-api ai::`, `cargo test -p lakehouse-api --test
    ai_build_tool_list`.
- **T4 (D7).** `requires`, `ROUTE_ACCESS`, `canOpen`, and the two drift tests.
  - *Accept:* `bun test src/components/app-shell`.
- **T5 (D8).** The sidebar, palette, guard, landing redirect and dock.
  - Bun tests with `stubApi` as in `roles-page.test.tsx:35-53`:
    - a Dashboard Viewer sees only Dashboards;
    - `/query-studio` shows "You don't have access" and makes no `/api/query` request;
    - `/` redirects a Dashboard Viewer to `/dashboards`;
    - an Assistant only user sees Home, History and Dashboards and the dock;
    - `*:*` sees everything;
    - the palette hides "Add a chart" without `dashboard:write`.
  - *Accept:* `bun run typecheck && bun run lint && bun test src/components
    src/features/auth`.
- **T6.** Docs.
  - `docs/OPERATIONS.md`: the two new roles, what the upgrade adds to existing roles, and how
    to take the assistant away from a role.
  - The feature page status: leave it to the planner.

## 4. Out of scope

- `ADM-5` module switches.
- `GOV-7` table access.
- Any change to dashboard sharing levels.
- A tenant administrator.
- Changing chart alert rights (`BI-19` C3).

## 5. Handoff (developer appends below)

Give the commits, the exact commands with their counts, the D5 route-caller list, the D6 list
of read tools and in-handler checks, anything *not verified* and why, and the output of
`git status --short`, `git log --oneline origin/main..HEAD` and `git ls-remote --heads origin
<branch>`.

## 6. Review (planner)
