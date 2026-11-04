# WEB-3: the console shows what works, in one place each

Backlog `WEB-3` (new, Task, P1, Platform). Planner: Claude Opus, 2026-11-04.
Owner's decision, 2026-11-04: "be aggressive … if you think to remove or revamp it tell me",
then "go ahead and do it. update the cases ok." Anchors verified at `main` `d19bdf73`.
This file plans three slices; **slice 1 is the only one ready to build.** Slices 2 and 3 get
their own plan checks before they start.

## 0. What is wrong, with evidence

An audit of all 39 sidebar entries (2026-11-04) found no mock data, but about half the pages
are thin, duplicated, or show numbers the product does not have:

- **Digital Employees, Agent Runs**: `INT-1` is parked; `routes/agents.rs:13-18` says there
  is no execution runtime. Live pages still link to them (`approvals-page.tsx:118,130,369`,
  `asset-lineage.tsx:166`, Health's agent card).
- **Observability**: `ops.rs:127` always returns `activeIncidents: null`; the two SLOs are
  fixed in code (`ops.rs:128-141`).
- **Workloads**: promises "class, engine category, cost, and fairness signals"; `ops.rs`
  returns null for class and cost. The one real action (cancel a running query) is Query
  Studio's Stop (`BI-5`).
- **Activity**: the same audit trail as Audit Log (`overview.rs:369` vs `governance.rs:361`).
- **Create Policy wizard** (`/governance/policies/create`): saves free-text
  subjects/resources/conditions; `policy_engine.rs:2-21` enforces only structured JSON
  conditions, so a policy made there is listed as enforced and masks nothing. The policy
  dialog on a table's Access tab (`catalog/asset-access.tsx`) writes enforced conditions.
- **SSO**: one line and a "Test sign-in" button (`/api/auth/providers`).
- **Lineage page**: the same API as the table's Lineage tab, with a second graph renderer.
  It is a deep-link target from charts, pipelines and runs (`chart-lineage-panel.tsx:92`,
  `pipeline-detail-page.tsx:587`, `run-inspector.tsx:324`, `asset-lineage.tsx:296`), so it
  leaves the sidebar in slice 1 and its links move in slice 3.

## 1. Decisions

- **D1. Removed means gone.** The route file, the feature files only it uses, their tests,
  the client methods and contract types nothing else uses, and every link to it. No
  "hidden" or `preview` copy is kept (rule: no dead code; `AGENTS.md` "no new mocks").
- **D2. Backend routes stay in slice 1.** `/api/ops/observability`, `/api/ops/workloads`,
  `/api/agents/employees*`, `/api/agents/runs` (list), `/api/overview/activity` stay
  registered and classified; the assistant's tools (`list_workloads`, `kill_query`,
  `get_audit_history`) still use some of them. Retiring unused routes is a separate task
  (`WEB-4`, proposed).
- **D3. A run's detail page stays.** An approval from an assistant chat creates a real
  `agent_run` (`routes/ai/gate.rs:252`), and `/agents/runs/[runId]` shows its trace. It
  stays, reachable from an approval; the runs LIST, the employees list and the employee
  detail pages go.
- **D4. Approvals moves to `/governance/approvals`.** `/agents/approvals` becomes a
  redirect-only page, as the repo's other legacy paths are (`src/app/audit-logs/page.tsx`).
- **D5. Policies keeps its list, loses "Create Policy".** The header and empty-state buttons
  become one line: "Policies are added on a table's Access tab, where they are enforced."
  with a link to `/catalog`. The page description drops "agent, residency, retention, and
  budget" (none is enforced; `governance.rs` says residency is enforced nowhere). Existing
  wizard-made policies stay listed (data is not deleted).
- **D6. SSO's content moves to Settings.** A "Sign-in" section at the end of
  `/admin/settings` shows what `/admin/sso` shows today (provider on/off, the
  `OIDC_ROLE_MAP` note, "Test sign-in"). Reuse the SSO component; do not rewrite it.
  `/admin/sso` becomes redirect-only to `/admin/settings`.
- **D7. Links that pointed at removed pages:**
  - Health's query cards (`overview-page.tsx:83,89`) link to `/query-studio/history`.
  - Health's "Recent activity" "View all" (`overview-page.tsx:206`) → `/audit`.
  - Home's comment about `/activity` (`home-page.tsx:138`) → name `/audit`.
  - Approvals: the agent name is plain text; the run id keeps its link to
    `/agents/runs/[runId]` (D3); the empty state's "View agent runs" button goes.
  - `asset-lineage.tsx:166` `dependentHref`: an `agent` dependent gets no link (plain text).
  - Health's agent-runs card (if present in `overview-page.tsx`) goes.
- **D8. The sidebar after slice 1** (`nav-config.ts`): the Intelligence group goes; under
  Monitoring, Activity, Workloads and Observability go; under Governance, Lineage goes from
  the list (route kept) and Approvals' href becomes `/governance/approvals`; under
  Administration, SSO goes. If no item is `preview: true` any more, remove the `preview`
  field, `SHOW_PREVIEW` and the "Soon" rendering from `nav-config.ts` and
  `app-sidebar.tsx`, with their tests.
- **D9. The assistant's page context** (`copilot/page-context.ts`) drops entries for removed
  routes and adds `/governance/approvals`. No tool or `tool_schemas.json` change.

## 2. Anchors (at `d19bdf73`)

| What | Where |
| --- | --- |
| Sidebar | `src/components/app-shell/nav-config.ts:156-240`, `app-sidebar.tsx` |
| Routes to delete | `src/app/(intelligence)/agents/employees/**`, `(intelligence)/agents/runs/page.tsx`, `(operations)/observability`, `(operations)/workloads`, `(overview)/activity`, `(governance)/governance/policies/create`, `(admin)/admin/sso` (becomes redirect, D6) |
| Route to move | `(intelligence)/agents/approvals/page.tsx` → `(governance)/governance/approvals/page.tsx` (D4) |
| Feature files | `src/features/agents/employee*`, `runs-page.tsx`, `run-columns.tsx` (check `run-detail-page.tsx`'s imports before deleting anything it uses), `src/features/ops/workloads-page.tsx`, the observability page, `src/features/overview/activity-page.tsx`, `src/features/governance/policy-create-page.tsx` |
| Links | `approvals-page.tsx:110-135, 362-372`; `overview-page.tsx:78-92, 200-210`; `home-page.tsx:134-140`; `asset-lineage.tsx:160-170`; `policies-page.tsx:88-115` |
| Clients | `src/services/clients/ops.ts`, `src/services/clients/agents*.ts`, `src/services/index.ts:35,56` |
| Docs | `docs/core/reference/user-guide.md:412` (Create Policy), `:531-533` (Activity, Workloads, Observability), `:554` (Intelligence); feature pages that name a removed page (`git grep -nE "Observability|Workloads|/activity|Digital Employees|Agent Runs|Create Policy|/admin/sso" docs/core`) |

## 3. Slice 1 tasks (one commit each, in order; push after each)

**T1. Delete the pages and their code (D1, D3).** Routes, feature files, tests, unused client
methods and contract types. `git grep` each removed export afterwards: zero hits outside
the deleted files.
- *Accept:* `bun run typecheck && bun run lint`.

**T2. The sidebar and the moves (D4, D6, D8, D9).** `nav-config.ts`, the approvals move with
its redirect, SSO into Settings with its redirect, page context.
- *Accept:* `bun run typecheck`; `bun run test src/components/app-shell src/features/admin src/features/agents src/features/copilot`.

**T3. The links (D5, D7).**
- *Accept:* `git grep -nE "/agents/employees|\"/agents/runs\"|/observability|/workloads|\"/activity\"|/governance/policies/create|/admin/sso" -- src` shows only redirect pages and the run-detail route; `bun run test`.

**T4. Docs and changelog.** User guide sections, feature pages, `CHANGELOG.md`
`## [Unreleased]` → `### Removed`: the pages and why (one line each, the reasons from
section 0), and `### Changed`: Approvals' new address, SSO in Settings, Policies' button.
- *Accept:* the repo's docs lints in `ci.yml` "Repo lints" pass.

## 4. Verification before handoff

```bash
bun run typecheck && bun run lint && bun run test
docker compose --profile '*' config --quiet
git status
```
No Rust, Python or compose change is expected; if one happens, stop and report.

## 5. Slice 2 plan check (planner, 2026-11-04, on top of slice 1 `c3d37eeb`)

**Builder: an Opus developer agent** (product owner, 2026-11-04: revamped, merged or new UI is
built by Opus; `AGENTS.md` "Exception for console design work"). Same developer rules: no
change of scope or decisions; a mismatch with the code is reported, not improvised. The
branch is `feat/web-3-slice-2-catalog-and-system-status`, made from slice 1's branch; merge
`origin/main` into it once slice 1 (#372) merges.

### 5.1 What is wrong

- **Catalog and Data Explorer are one list twice.** `src/features/catalog/catalog-page.tsx`
  (216 lines) and `data-explorer-page.tsx` (385) both page through
  `assetService.listAssetsPage`; Catalog only adds a namespace drawer that previews five
  assets (`catalog-page.tsx:27`). A user has two places to find a table and the same row
  actions in neither.
- **"Is the system healthy?" is five read-only pages.** Health (`overview/overview-page.tsx`,
  244), Services (`ops/services-page.tsx`, 148), Capacity (`lakehouse/capacity-page.tsx`, 87),
  Ingestion (CDC) (`governance/ingestion-page.tsx`, 91) and Table Maintenance
  (`governance/maintenance-page.tsx`, 107, with `lakehouse/iceberg-tables-section.tsx`). Health
  already repeats parts of Services and Capacity as cards.

### 5.2 Decisions

- **S2-D1. One Catalog at `/catalog`.** Search, filters and the asset list in one page:
  - the search and filters Data Explorer has (name, columns, tags, description, owner;
    source/namespace, layer, certification), plus the namespace grouping Catalog has, as a
    filter or a side list, not a drawer of five;
  - every row action Data Explorer has today, and the row opens the asset page
    (`/data/assets/[assetId]`, unchanged);
  - the "supported: false" empty state Data Explorer shows is kept, honestly worded;
  - `/data` becomes redirect-only to `/catalog` (keep query parameters that still mean
    something, e.g. a search term; drop the rest).
  - Sidebar "Data": Catalog, Query Studio, Sources.
- **S2-D2. One System status page at `/health`** (page title and nav label "System status"):
  sections, in this order, each from the API it uses today, no new endpoint:
  1. a one-line overall state (every service up / which is not) and the open alerts count,
     linking to Alerts;
  2. **Services**: the probe table from `services-page.tsx` (a value the API does not return —
     version, latency — shows "Not measured", never a blank or a made-up number);
  3. **Storage**: capacity (the daily snapshot and ClickHouse disk) from `capacity-page.tsx`;
  4. **Live change capture**: the replication-slot health from `ingestion-page.tsx`, shown
     only when at least one CDC connector exists, otherwise one muted line;
  5. **Table maintenance**: the latest maintenance runs from `maintenance-page.tsx`, each
     linking to its table; the per-table policy stays on the table's page
     (`lakehouse/table-detail-page.tsx`, unchanged);
  6. the freshness strip and the query cards Health has today.
  - `/services`, `/lakehouse/capacity`, `/governance/ingestion` and `/governance/maintenance`
    become redirect-only to `/health#<section>`.
  - The Iceberg table list (maintenance page's "Tables" tab) goes: each table is reached from
    its asset page; if an Iceberg table has no asset page, say so in the handoff instead of
    keeping the list.
  - Sidebar "Monitoring" becomes: System status, Alerts, Audit Log.
- **S2-D3. CDC health also on its connector.** A CDC connector's page in Sources
  (`src/features/connectors/*` detail) shows its own slot's health from the same API, filtered
  to that connector. One component, used by both places (rule 4).
- **S2-D4. Design.** Use the existing `design-system/` components and `src/components/patterns`
  (PageHeader, SectionCard, MetricCard, data table, EmptyState, StatusBadge). No new colour or
  spacing tokens. A section that cannot load shows its own error with Retry; the rest of the
  page still renders. Layout must work at 1280 px and on a phone width (the sidebar already
  collapses).
- **S2-D5. The assistant.** `copilot/page-context.ts`: `/catalog` and `/health` describe the
  new pages; the removed paths go. No tool change, no `tool_schemas.json` change (if one seems
  needed, stop and report).
- **S2-D6. Nothing is deleted from the backend** (D2 of slice 1 holds).

### 5.3 Tasks (one commit each; push after each)

- **S2-T1. Catalog (S2-D1).** *Accept:* `bun run typecheck && bun run lint && bun run test src/features/catalog src/components/app-shell`.
- **S2-T2. System status (S2-D2).** *Accept:* the same, for `src/features/overview src/features/ops src/features/lakehouse src/features/governance`.
- **S2-T3. CDC on the connector (S2-D3).** *Accept:* the same, for `src/features/connectors`.
- **S2-T4. Page context, docs, changelog (S2-D5).** User guide sections for the merged pages;
  CHANGELOG `## [Unreleased]` → `### Changed`.
- **S2-T5. Proof in a browser.** Run the console against a dev API (`docs/core/reference/user-guide.md`
  or the repo's dev instructions) and take a screenshot of `/catalog` and `/health` at
  1280 px and at 390 px; put them under `docs/plans/web-3-screens/` and list them in the
  handoff. If no dev stack can be started, write *not verified* with the reason.

### 5.4 Verification before handoff

```bash
bun run typecheck && bun run lint && bun run test
git status
```

## 6. Slice 3 plan check (planner, 2026-11-04; anchors at slice 2's branch `2eae89d1`)

Split in two, because the screens need API the product does not have yet:

- **3a: API** (DeepSeek developer, bluefin). Branch `feat/web-3-slice-3a-api` from `main`.
- **3b: the screens** (Opus developer, desktop; owner 2026-11-04). Branch
  `feat/web-3-slice-3b-screens` from slice 2's branch, with `origin/main` and 3a's branch merged in
  once they land.

3b starts with the parts that need no new API (S3-T1 to S3-T4) while 3a is built.

### 6.1 Evidence (survey 2026-11-04)

- **Columns.** The Schema tab (`catalog/asset-schema.tsx:132-185`, `asset-columns.tsx:544-575`)
  shows a `masked` pill and a classification badge per column, with no controls.
  - `masked` means masked *for the viewer* (`routes/catalog.rs:2360-2366`), not that a mask policy
    exists. The policies in force are in `AssetDetail.policySummary`.
  - A classification is set and removed on the Access tab (`asset-access.tsx:108-135`, `:197-290`).
  - A mask is a policy made by the Access tab's Add policy dialog (`:302-370`), which writes the
    structured conditions `PolicyCondition::parse` enforces.
  - The classification `maskingRule` text is never applied anywhere.
  - Classification keys a table by `lineageKey`; a policy keys it by `policyTable`.
- **`GET /api/governance/classification`** (`routes/governance.rs:455-481`) emits one row per
  registry dataset with `classification: "internal"`, hardcoded. The `tier` it selects is unused, and
  `asset` is the dataset's title, not a key. That is an invented value (principle 2).
- **Policies.** A policy carries no asset id. The table is only in `conditions.table`.
  - `silver.*` and `serving.*`: the table is the asset id (`catalog.rs:1726`).
  - `bronze.<table>`: the asset id is a registry slug that nothing on the policy names.
  - There is no `PUT /api/governance/policies/{id}`.
- **Quality.**
  - Schedules: `GET /api/governance/quality/schedules` lists every table; `PUT .../{asset}` sets one.
    There is no reset.
  - Freshness targets: `GET` / `PUT /api/governance/sla` and `DELETE /api/governance/sla/{table}`.
  - Today both are shown only on the asset page (`quality-schedule.tsx:46`, `asset-activity.tsx:282-430`).
- **Roles.** Only `GET` / `POST /api/identity/roles`.
  - The store has `delete_role` (FK violation while a user holds the role) but no `update_role`.
  - The page is create-only and calls itself "Teams & Roles".
- **Lineage links.**
  - The asset page reads `?tab=`, so `/data/assets/<id>?tab=lineage` works.
  - The asset's own link (`asset-lineage.tsx:298`) can use it.
  - A pipeline has an optional `targetAssetId`.
  - A run has none.
  - A chart's lineage spans several tables.
  So the Lineage page **stays** for charts, runs and pipelines without a target; only the links
  that know their asset move to the tab. (This revises section 0's "the page goes".)

### 6.2 Slice 3a: API (DeepSeek)

- **S3a-D1. Policies say which table page they belong to.** The policy views (`PolicyView`, from
  `SEC-61` #373) gain `assetId: Option<String>`, resolved server-side from `conditions.table`:
  - `silver.*` / `serving.*`: the table itself;
  - `bronze.<table>`: the registry slug, from the same registry reader the catalog uses (rule 4);
  - unparseable conditions, or a table with no catalog entry: `None`.
  Contract `assetId?: string`. Build on `SEC-61`: merge #373's branch, or main once #373 merges.
- **S3a-D2. Roles can be edited and deleted.**
  - `PUT /api/identity/roles/{id}` (name, description, permissions) and
    `DELETE /api/identity/roles/{id}`, each `POLICY_TABLE`-classified with the permission
    `POST /api/identity/roles` uses.
  - Both asserted both ways in `tests/route_auth.rs`.
  - Both audited, as role creation is.
  - A store `update_role`.
  - Delete while users hold the role → 409 with the fixed sentence "This role is still given to
    N people. Take it away from them first."
  - A seeded system role (check how the seed marks one) → 409 "A built-in role cannot be deleted."
  - Never pass an FK violation's text through (principle 4).
- **S3a-D3. A schedule can go back to the default.** `DELETE /api/governance/quality/schedules/{asset}`
  → 204, after which `GET` shows the default for that asset. Same permission as `PUT`.
- **S3a-D4. No invented classification.** `classification()` emits a registry-derived row only
  when the dataset's `tier` is one of the product's classification levels (read the levels from
  the code: the classification badge and the `create_rule` validation), with that level as its
  classification, and `asset` as the same key the stored rules use. Otherwise it emits no row.
  Report the `tier` values seen in the seed. If the levels and the tiers do not match at all,
  stop and report: the derived rows are then simply removed (planner default).
- **S3a-D5. Tests and docs.**
  - Route tests for each new route, both ways, in `route_auth.rs`.
  - A unit test for each `assetId` case.
  - A test that the derived classification row carries the tier, not "internal".
  - CHANGELOG `### Added` / `### Fixed`.
  - Mutation check: hardcode `"internal"` again and the D4 test fails.
- *Verification:* the `AGENTS.md` Rust block for `lakehouse-api` and `lakehouse-store`, scoped
  per commit; the workspace run once before handoff (`CARGO_BUILD_JOBS=4` on bluefin).

### 6.3 Slice 3b: the screens (Opus)

- **S3-D1. Sensitivity and masks live on the column** (Schema tab). Opening a column shows a
  "Sensitivity and masking" section:
  - **Sensitivity:** the column's classification, with Set / Change / Remove. The create and
    delete classification calls already exist; "change" is delete then create, in that order,
    with the new value shown only after both succeed.
  - **Masked for:** the roles of every enforced column-mask policy in `policySummary` that names
    this column, each linking to its policy.
    - **Add a mask** writes a Column mask policy for this column and the chosen roles, through the
      same `createPolicy` call and the same `policyTable` / `policyCondition` builders the Access
      tab uses today. Move those helpers to one shared module (rule 4); do not copy them.
    - **Remove** deletes a policy that masks only this column. A policy that also masks other
      columns is not deleted from here: the row says "Part of <policy name>, which also masks
      <columns>" with a link to the Access tab.
  - Needs `governance:write` / `policy:write`, as today. A reader sees the facts without the buttons.
- **S3-D2. The Access tab** keeps grants, request access and the table's policies (with
  SEC-61's not-enforced notice). Its Classify dialog and classification card go: one place per
  fact.
- **S3-D3. The Classification & Masking page goes.**
  - `/governance/classification` becomes redirect-only to `/catalog`.
  - The Catalog gains a "Has sensitive columns" filter **only if** the catalog list carries the
    asset's classification (`AssetSummary`). If not, the filter is left out and the handoff says
    so; no new endpoint.
- **S3-D4. Policies becomes "Access rules"** at `/governance/policies` (the address is kept).
  - A read-only list: name, table (linking to `/data/assets/<assetId>?tab=access` when `assetId`
    is known, plain text otherwise), roles, what it does (masks <columns> / filters rows), and
    enforced or not (SEC-61).
  - No Enforce, Stop or Delete here: those stay on the table's Access tab. The drawer is read-only
    and links to the table.
  - Needs S3a-D1.
- **S3-D5. Data Quality gains two tabs: Schedules and Freshness targets.**
  - **Schedules:** every table's schedule from the list endpoint, with Edit (the existing
    `QualitySchedulePanel` controls, reused) and Reset (S3a-D3).
  - **Freshness targets:** every target from `GET /api/governance/sla`, with Add, Edit and Remove
    (the existing dialog, reused).
  - A table key that is not an asset id shows as plain text (the Bronze slug gap). Do not
    fabricate a link.
  - The asset page keeps its own controls.
- **S3-D6. Settings has sections:** Reporting, Remembered results, Query limits, Sign-in, Tag keys,
  Maps.
  - A section list or tabs, and the section is in the URL (`?section=`).
  - Reuse the Tag keys page body and `MapsAdmin` as they are.
  - `/governance/tag-keys` and `/admin/maps` become redirect-only to their section.
- **S3-D7. Users has tabs:** People | Sessions (the Sessions page body).
  `/admin/sessions` → `/admin/users?tab=sessions`.
- **S3-D8. "Teams & Roles" becomes "Roles"**, with Edit and Delete (S3a-D2). A 409 shows the
  API's sentence.
- **S3-D9. Lineage links:**
  - the asset page's link → `?tab=lineage`;
  - a pipeline with a `targetAssetId` → that asset's tab;
  - everything else keeps `/lineage?focus=`.
- **S3-D10. The sidebar after slice 3:**
  - Governance: Access rules, Data Quality, Approvals.
  - Administration: Users, Roles, Tenants, Service Identities, Settings.
  - Page context (`copilot/page-context.ts`) follows. No tool or `tool_schemas.json` change.
- **S3-D11. QA.** The planner updates the Lark QA cases that name the moved pages when 3b merges.

**Tasks** (one commit each, push after each; 3a-dependent ones last):

- **S3-T1** Settings sections (D6).
- **S3-T2** Users tabs (D7).
- **S3-T3** Data Quality tabs: schedules Edit and freshness targets (D5, without Reset).
- **S3-T4** Column sensitivity and masks, the Access tab change, and the page removal (D1–D3).
- **S3-T5** (after 3a) Access rules (D4), schedule Reset, Roles edit/delete (D8).
- **S3-T6** Lineage links, sidebar, page context, docs, CHANGELOG (D9, D10).

*Verification:* `bun run typecheck && bun run lint && bun run test`. On the desktop, run Rust
tests scoped only (its loopback hangs on closed ports), and leave the full run to CI.
Screenshots: only if `libnss3` / `libnspr4` are installed by then; otherwise *not verified*.

## 7. Out of scope

- Any backend route removal (D2, `WEB-4`).
- The assistant's tools.

## 8. Handoff (developer appends below)

### Handoff (slice 1) — developer, 2026-11-04

Branch `feat/web-3-slice-1-remove-thin-pages`, one commit per task, pushed after each.

| Task | Commit | What |
| --- | --- | --- |
| T1 | `3ecdd09d` | Pages, feature files, client methods and contract types removed (D1, D3) |
| T2 | `8fc58d79` | Sidebar, approvals move, SSO into Settings, page context (D4, D6, D8, D9) |
| T3 | `f80235bc` | Every remaining link moved (D5, D7) |
| T4 | `983618ec` | User guide, feature page, support model, CHANGELOG (T4) |
| — | the commit that adds this entry | This handoff |

No Rust, Python or compose change; no `cargo` command was run. Slice 2 and 3 were not started.

**T1 accept.** `bun run typecheck` clean. `bun run lint`: 0 errors, 10 warnings — all in files this slice did not touch (`use-copilot.ts`, `use-data-table.ts`, `dashboard-specs.ts`, `uploads.test.ts`, `http.ts`, chart builder). `git grep` for each removed export (`EmployeesPage`, `EmployeeDetailPage`, `RunsPage`, `ObservabilityPage`, `WorkloadsPage`, `ActivityPage`, `PolicyCreatePage`, `getEmployeeColumns`, `getRunColumns`, `getWorkloadColumns`, `getActivityColumns`, `WorkloadStatusBadge`, `ObservabilitySummary`, `WorkloadItem`, `CreateEmployeeInput`, `RunEmployeeInput`, `WorkloadStatus`, `WORKLOAD_STATUS_LABEL`): zero hits anywhere in `src`. The removed `AgentService`/`OpsService` method names have zero hits too; `listRuns` still exists on `PipelineService` (unrelated).

**T2 accept.** `bun run typecheck` clean. `bun run test src/components/app-shell src/features/admin src/features/agents src/features/copilot` — Bun appended those paths to the script's own `bun test src` and ran the full suite: 3205 pass, 1 skip, 0 fail, 3206 tests, 327 files. The four paths were therefore not proven in isolation. There was no test for `preview`, `SHOW_PREVIEW`, `comingSoon` or the "Soon" rendering, so none was deleted.

**T3 accept.** The line

```
git grep -nE "/agents/employees|\"/agents/runs\"|/observability|/workloads|\"/activity\"|/governance/policies/create|/admin/sso" -- src
```

prints:

```
src/app/(admin)/admin/settings/page.tsx:6:import { SsoPage } from "@/features/admin/sso-page"
src/lib/status.ts:232: * /api/agents/employees/{id}/run`, copilot-operations-handover plan T3.2).
src/services/clients/agents.ts:67:      `/api/agents/employees/${encodeURIComponent(id)}`,
src/services/contracts/agents.ts:24:   * (`GET /api/agents/employees/{id}`), `null` on the LIST route (a list
src/services/contracts/agents.ts:40:   * /api/agents/employees/{id}/run`) sends to the copilot. `null`/absent
```

The first is a false positive: the pattern matches the SSO component's own import path (`@/features/admin/sso-page`), which D6 says to reuse rather than rename. The other four are `/api/agents/employees/...` route paths (D2 keeps the backend routes). There is no UI link to a removed page left, and no redirect page contains its old path. `bun run test`: 3205 pass, 1 skip, 0 fail.

**T4 accept (ci.yml "Repo lints").** `./rust/tests/parity/check-no-secrets.sh` — corpus clean. `check_bare_iceberg_count.py`, `check_intra_package_imports.py`, `check_compose_init_readiness.py`, `check_connector_gate_coverage.py` (19 supported types, 19 covered, 0 waived), `check_image_digests.py` (46 pinned, 8 exempt) — all OK. `unittest discover` over `ops/lint` 33 tests OK, `ops/gates` 29 OK, `ops/bench` 18 OK.

**Section 4 block, final commit, foreground.**

- `bun run typecheck` — clean.
- `bun run lint` — 0 errors, 10 pre-existing warnings (same list as T1).
- `bun run test` — 3205 pass, 1 skip, 0 fail, 8720 expect() calls, 3206 tests, 327 files, 194.43 s.
- `docker compose --profile '*' config --quiet` — exit 0, with throwaway `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY` **and `CONNECTOR_SECRETS_KEY`** (the last one is not in the SEC-18 note in `AGENTS.md`, but compose refuses without it). No `.env` was written.
- `git status` — empty.

**Not verified.**

- No browser run. The two redirects (`/agents/approvals`, `/admin/sso`), the new Settings "Sign-in" section and the moved approvals page were checked by typecheck and unit tests only, not in a running console — that is the owner's acceptance checklist.
- The assistant's page context for `/governance/approvals` has no test; `page-context.ts` has none today, and none was added (D9 asks for no tool change either).
- The narrow T2 test paths were not proven in isolation (see above).

**Test deletions.** No removed page had a test. No kept page's test asserted a removed link, so no test was updated. `sso-page.test.tsx` still covers the reused SSO component unchanged.

### Handoff (slice 2) — developer (Opus), 2026-11-04

Branch `feat/web-3-slice-2-catalog-and-system-status` on top of slice 1 (`2ef32ef4`), one
commit per task, pushed after each. `origin/main` was not merged in (#372 not merged yet).

| Task | Commit | What |
| --- | --- | --- |
| S2-T1 | `495efab1` | One Catalog at `/catalog`; `/data` redirect; sidebar Data; links (S2-D1) |
| S2-T2 | `ebcac6e4` | System status at `/health`; four redirects; Iceberg list gone; sidebar Monitoring (S2-D2) |
| S2-T3 | `a794e4d6` | A CDC connector's page shows its own slot (S2-D3) |
| S2-T4 | `bb7abba1` | Page context, user guide, CHANGELOG (S2-D5) |
| S2-T5 | — | *Not verified* (no browser on this machine; see below). No commit: there were no screenshots to add |
| — | the commit that adds this entry | This handoff |

No Rust, Python or compose change; no `cargo` command was run. Nothing deleted from the backend (S2-D6).

**Per-task accept lines.** `bun run test <paths>` appends the paths to the script's own `bun test src`
and runs everything (slice 1's handoff says the same), so the scoped runs below used `bun test <paths>`.

- S2-T1: `bun run typecheck` clean; `bun run lint` 0 errors, 10 warnings (the same pre-existing list as slice 1);
  `bun test src/features/catalog src/lib/table-filter-link.test.ts src/lib/catalog-redirect.test.ts src/components` 400 pass, 0 fail, 24 files.
- S2-T2: typecheck clean; lint 0 errors, 10 warnings; `bun test src/features/overview src/features/governance src/features/lakehouse src/lib src/components/app-shell` 1194 pass, 0 fail, 132 files (`src/features/ops` no longer exists).
- S2-T3: typecheck clean; lint 0 errors, 10 warnings; `bun test src/features/connectors` 410 pass, 0 fail, 21 files (the three new slot cases: `-t "replication slot"` 3 pass).
- S2-T4: `bun test src/features/copilot` 112 pass, 0 fail; `./rust/tests/parity/check-no-secrets.sh` corpus clean; typecheck clean, lint 0 errors.

**Section 5.4, final code commit (`bb7abba1`), foreground.**

- `bun run typecheck` — exit 0, no output.
- `bun run lint` — exit 0; 0 errors, 10 warnings, all in files this slice did not touch.
- `bun run test` — first run: 3227 pass, 1 skip, **1 fail**, 3229 tests, 328 files, 193.79 s. The failure was
  `CommandPalette catalog search > shows an asset the server matched by a term that is in neither its id nor its
  name` (the file's first test, a `findByText` with the default 1 s wait). It passed 3 of 3 runs of
  `bun test src/components/command-palette.test.tsx` (17 pass each), and the second full run was green:
  **3228 pass, 1 skip, 0 fail, 8740 expect() calls, 3229 tests, 328 files, 180.88 s.** The slice touched that
  file only to move the "See all results" link to `/catalog?search=`. I read it as a timing flake under full-suite
  load, not proven; the reviewer's own run will say.
- `git status` — empty.

**S2-T5, screenshots: *not verified*.** The dev stack did start: a throwaway compose project
(`docker compose -p web3s2 up -d --no-build lakehouse-api`, with `ops/init-env.sh`'s generated `.env`, the
local `rantai-lakehouse-api:local` image, `oidc-mock` built, API on a spare host port) and
`next dev` against it. What could not run is a browser: T3's preview and Playwright's Chromium both stop on
missing system libraries (`libnspr4.so`, `libnss3.so`), and installing them needs sudo. So there are no files
under `docs/plans/web-3-screens/`, and no look at 1280 px or 390 px was taken. What was checked without a
browser, against the running dev server: `/catalog` and `/health` compile and answer 200; the redirects
answer `NEXT_REDIRECT` to `/catalog?search=orders` (from `/data?search=orders&page=3`: search kept, `page`
dropped), `/health#services`, `/health#storage`, `/health#change-capture`, `/health#maintenance`, and
`/catalog` (from `/lakehouse/tables`). Not checked: that the browser scrolls to the `#section` after a server
redirect, and anything signed in. The stack, its volumes, the built image and the `.env` were removed
afterwards. `next dev` rewrote `AGENTS.md` (its own "agentRules" block); that change was reverted, not committed.

**For the reviewer: choices made inside the decisions.**

1. *Catalog namespace grouping (S2-D1) is a filter row, not a side list.* A side column costs ~240 px, and the
   explorer's column-hiding thresholds are derived from the window width with the sidebar open
   (`catalog-page.tsx`, DATA-11/DATA-24 comments); a side list would have invalidated all of them. The row sits
   with the Layer row (same quick-filter pattern, aligned labels, sideways scroll on a phone). Picking a namespace
   sets the `namespace` filter to its **id** (DATA-23) and shows a strip with its description, owner, residency,
   source engine and asset count, so nothing the old drawer showed is lost. The namespaces load on their own and
   fail alone with Retry.
2. *`/data` redirect keeps* `search`, `filters`, `sort`, `joinOperator`, `groupBy` and the legacy
   `q/layer/tier/type` (the Catalog still translates those on arrival); it drops `page`, `perPage`, `view` and
   column layout (`src/lib/catalog-redirect.ts`).
3. *Old Catalog's saved table layout.* The asset table keeps the default `persistKey` (the pathname), so
   `/catalog`'s remembered state from the old namespace table may be read once; `useTableMemory` drops filters on
   columns that no longer exist, and an unknown column id in a saved order is ignored. A fresh key would have
   broken the sidebar's "remember my filters" link (`rememberTableHref` keys on the pathname).
4. *System status is exactly the six sections of S2-D2.* Health's other cards (catalog assets, pipelines running,
   policy violations, pending approvals, storage by tier) and its "Recent activity" list are **not** on the page.
   If the owner wants any back, that is a decision change. With them gone, `listActivity` and `listWarehouses`
   had no consumer and were removed from the contracts and clients (routes untouched).
5. *Overall state counts only probed services.* `ops.rs` marks unprobed ones `unknown`/`checked: false`; the line
   says "All N probed services are up", lists the unprobed ones as "not counted", and says "not known" when
   nothing was probed (`src/lib/system-status.ts`, tested).
6. *Each slot once, each table once.* `/api/governance/replication` and `/api/governance/maintenance` return
   history (every 15-minute check; every run). The sections show each slot's newest reading and each table's
   newest run; the old pages listed every row.
7. *"Shown only when at least one CDC connector exists" (S2-D2.4): mismatch, resolved without a new endpoint.*
   The section's API cannot tell whether a CDC connector exists; the connector list has no adapter field, and
   `/api/connectors/ingestible` needs `ingest:read` (the orchestrator's permission) and is not the API the
   section used. So: no slot reading → the one muted line. And on a deployment where no CDC connector ever
   ran, the slot table does not exist and the route answers 503, the same as ClickHouse being down
   (`routes/governance.rs`). The section then shows its error with Retry plus a sentence saying both causes.
   Please decide whether that is acceptable or whether the 503 case needs a backend change (out of this
   slice's scope).
8. *Iceberg tables without an asset page.* The removed list (`iceberg-tables-section.tsx`) showed every table in
   the warehouse, "including ones the catalog registry never recorded (Gold export tables, for example)". Those
   tables now have no list that reaches them; their page `/lakehouse/tables/{ns}/{table}` still works by
   address, and a maintained Bronze table is linked from System status's maintenance runs. `/lakehouse/tables`
   now redirects to `/catalog` (it redirected to the removed maintenance page). The table page's back button
   was "Table Maintenance" → `/governance/maintenance`; it is now "Maintenance runs" → `/health#maintenance`
   (the only change to `table-detail-page.tsx`).
9. *CDC on the connector (S2-D3)* is in the connector's **Overview** tab, shown when its ingest spec's adapter or
   ingest mode is `cdc` (the spec the page already loads, so a non-CDC connector makes no extra request).
10. *Labels.* "Open in Data Explorer" → "Open in Catalog" (upload result), "Inspect in Data Explorer" →
    "Find in Catalog" (classification, data quality), "Lands in Data Explorer as" → "Lands in the Catalog as".
    Several pages in `docs/core/features/` still say "Data Explorer" in decisions and in acceptance rows dated
    before this slice; they were left as written, because rewriting a passed acceptance row would change a
    record. Only the user guide and CHANGELOG were in S2-T4.
11. *System status layout.* A summary card (state pill, sentence, open-alerts button to `/alerts`), a row of
    section jump links (the redirect anchors), then one `SectionCard` per section using the simple
    `patterns/data-table`, not the advanced toolbar table: these are short status lists, not something to
    filter. A page-level Refresh reloads every section. No new colour or spacing token.

**Tests added, moved or removed.** Added: `src/lib/system-status.test.ts` (9), `src/lib/catalog-redirect.test.ts`
(5), two `setFilter` cases in `table-filter-link.test.ts`, `src/features/overview/system-status-page.test.tsx` (7),
three slot cases in `connector-detail-page.test.tsx`, four namespace-row cases in `catalog-page.test.tsx`. Moved:
`data-explorer-page.test.tsx` → `catalog-page.test.tsx` (DATA-24 cases kept; path and layout key now `/catalog`).
Removed with what they tested: the old `catalog-page.test.tsx` (namespace drawer) and
`catalog-namespace-columns.test.tsx`. Updated because the link or label they asserted moved:
`command-palette.test.tsx`, `upload-file-page.test.tsx`, `table-filter-link.test.ts`, `lakehouse-view.test.ts`
(the removed `lakehouseWarehousesUrl` case). None of the five removed monitoring pages had a test.

**Not verified.** The screenshots and every visual check at 1280 px and 390 px (above); the browser following a
`#section` after a server redirect; the assistant's new page context with a model (no test exists for
`page-context.ts`, and none was added).

### Fix for review R1 (slice 2) — developer, 2026-11-04

**Commit.** `d448bf38` fix(web-3): no CDC yet reads as no slots, not an outage (WEB-3 review R1). Two files:
`rust/crates/lakehouse-api/src/routes/governance.rs` and `src/features/governance/replication-slot-health.tsx`.

**What changed.**
- `replication()` answers `{"replicationSlots": []}` with 200 when the slot query fails with `ClickHouse` error
  code 60 (`UNKNOWN_TABLE`), read with the existing `ChError::server_code()` (`lakehouse-clickhouse`), which parses
  the engine's `Code: N.` prefix. No error text is matched. Every other error, including one with no code, still
  returns the 503. The code is a private const `CH_UNKNOWN_TABLE` in `governance.rs`, with no change to
  `lakehouse-clickhouse`.
- The doc comment above `REPLICATION_SLOTS_SQL` now describes this and cites `WEB-3 review R1`. A comment at the
  match arm cites it too.
- Three route tests in `governance.rs` go through `get` with a router and a `wiremock` `ClickHouse`:
  `replication_route_answers_an_empty_list_when_the_slot_table_does_not_exist` (404 `Code: 60 … (UNKNOWN_TABLE)`
  → 200, `{"replicationSlots": []}`); `replication_route_is_503_when_clickhouse_is_down` (`CH_URL` at
  `127.0.0.1:1` → 503); `replication_route_is_503_for_any_other_clickhouse_error` (`Code: 497 … (ACCESS_DENIED)`,
  and a bare `UNKNOWN_TABLE` body with no code → 503 each).
- Console: `ReplicationSlotHealth`'s error line is now "The slot readings could not be read from ClickHouse." It
  no longer names the missing table, and the doc comment says the missing table gives the empty line. The same
  component renders on a connector's page, so the sentence does not point at a Services section.

**Mutation check.** I changed the code-60 arm to `return Err(err.into())`, which brings back the 503 for the
missing table, and ran `cargo test -p lakehouse-api --lib replication_route`: 2 passed, 1 failed. The failing test
was `replication_route_answers_an_empty_list_when_the_slot_table_does_not_exist`. The file was then restored and
the diff checked.

**Commands run (foreground unless noted; `CARGO_TARGET_DIR=/home/uaa/rantai/cargo-target`, `CARGO_BUILD_JOBS=4`).**
- `cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings`: clean.
- `cargo test -p lakehouse-api --lib replication`: 4 passed, 0 failed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean, exit 0.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 10 warnings, none in the changed file.
  `bun test src/features/overview`: 18 passed, 0 failed.
- `cargo test -p lakehouse-api` (background, run alone after a first run was stopped at the 30-minute limit
  while still compiling next to `bun run test`). `--lib`: 2983 passed, 5 failed, 18 ignored. Cargo stops after a
  failing binary, so I then ran `cargo test -p lakehouse-api --no-fail-fast --test '*'` (background): 105
  binaries, 1215 passed, 28 failed, 7 ignored.
- `bun run test` (background, ran at the same time as the cargo run above): 3229 tests, 3 failed, 2 errors, 1
  skipped. The 3 failures are timeouts: `CommandPalette catalog search …` and two `UploadsPanel folders …` tests,
  one of which took 1683 s. Both errors are "unhandled error between tests". Alone, `bun test
  src/components/command-palette.test.tsx src/features/connectors/uploads-panel.test.tsx` gives 44 passed,
  0 failed.

**The 33 Rust failures are from this machine, not from R1.**
- The 5 `--lib` failures are `connector_probe` ×3, `pdf::tests::a_gotenberg_that_is_down_is_unreachable` and
  `routes::connectors::tests::deprovision_reads_slot_and_publication_from_dial_for_cdc_adapter`. Each expects a
  closed local port to refuse the connection, and each gets a 5 s timeout instead. Run alone, all 5 fail in 5.01 s.
  On this WSL machine `exec 3<>/dev/tcp/127.0.0.1/1` hangs instead of being refused.
- The 28 integration failures are in `agents_approval` (12), `ai_tool` (12), `agents_run`, `embed_tokens`,
  `pdf_export` and `report_runner` (1 each). 23 of them are a 408 where the test expects 200. None of these tests
  touches the governance route.
- With `governance.rs` checked out at `094ac662` (before the fix), `cargo test -p lakehouse-api --test
  agents_approval` gives the same result as with the fix: 9 passed, 11 failed. The file was then restored.
- These suites need a host where a closed port refuses the connection; CI should be. *Not verified* green on this
  machine.

**Not verified.** The page against a real install with no CDC connector (no stack was brought up). The empty line
was not checked in a browser.

### Handoff (slice 3b) — developer (Opus), 2026-11-04

Branch `feat/web-3-slice-3b-screens` (from `b2b55d9a`), one commit per task, pushed after each. No
rebase, no force-push. **S3-T5 is not built: slice 3a is not done.** At the last check (after S3-T6)
`origin/feat/web-3-slice-3a-api` had S3a-D1 (`632df1df`) and S3a-D2 (`1f279d10`) but no schedule
`DELETE` (S3a-D3) and no "Handoff (slice 3a)" entry, so it was not merged and nothing of its API was
used. `origin/main` has neither #373 nor #374, so `origin/main` was not merged either.

| Task | Commit | What |
| --- | --- | --- |
| S3-T1 | `9216e132` | Settings sections in `?section=`; Tag keys and Maps inside; three redirects (S3-D6) |
| S3-T2 | `56eac7da` | Users: People and Sessions tabs in `?tab=`; `/admin/sessions` redirect (S3-D7) |
| S3-T3 | `34d79561` | Data Quality: Schedules (Edit) and Freshness targets (Add, Edit, Remove) tabs (S3-D5 without Reset) |
| S3-T4 | `bf637397` | Column "Sensitivity and masking"; Access tab loses classification; Classification page gone (S3-D1 to S3-D3) |
| S3-T5 | — | **Waits for 3a** (Access rules S3-D4, schedule Reset, Roles edit/delete S3-D8, and the two sidebar renames) |
| S3-T6 | `6d7fe3d2` | Pipeline lineage link, sidebar removals, page context, user guide, CHANGELOG (S3-D9, S3-D10 in part) |
| — | the commit that adds this entry | This handoff |

No Rust, Python or compose change; no `cargo` command was run.

**Per-task checks** (each: `bun run typecheck` exit 0; `bun run lint` 0 errors, 10 warnings, the same
pre-existing list as slices 1 and 2). Scoped runs used `bun test <paths>` (`bun run test <paths>` runs everything):

- S3-T1: `bun test src/features/admin src/features/governance/tag-keys-page.test.tsx src/lib/url-tab.test.ts` 67 pass, 0 fail.
- S3-T2: `bun test src/features/admin/users-page.test.tsx src/features/admin/sessions-page.test.tsx` 17 pass, 0 fail.
- S3-T3: `bun test src/features/catalog src/features/governance src/lib/table-key-href.test.ts` 385 pass, 0 fail.
- S3-T4: `bun test src/features/catalog src/features/governance src/lib/column-masks.test.ts src/services` 547 pass, 0 fail.
- S3-T6: `bun test src/features/pipelines/pipeline-detail-page.test.tsx src/features/copilot src/components/app-shell`
  130 pass, 0 fail; `./rust/tests/parity/check-no-secrets.sh` corpus clean.

**Before handoff, on `6d7fe3d2`, foreground.**

- `bun run typecheck` — exit 0.
- `bun run lint` — exit 0; 0 errors, 10 warnings, none in a file this slice touched.
- `bun run test` — first run: 3253 pass, 1 skip, **1 fail**, 3255 tests, 331 files, 206.18 s. The failure was
  `UploadsPanel > refreshes while a row is loading and stops when none is` (a poll-count assertion, expected 3), in
  `src/features/connectors/uploads-panel.test.tsx`, which this slice does not touch (no file under
  `src/features/connectors` changed). It passed 3 of 3 runs alone (27 pass each), and the second full run was green:
  **3254 pass, 1 skip, 0 fail, 8804 expect() calls, 3255 tests, 331 files, 196.39 s.** Read as a timing flake under
  load, not proven.
- `git status` — empty.

**Not verified.** Screenshots and every look at 1280 px and 390 px: `libnss3` / `libnspr4` are still not installed,
so no browser starts. The redirects were not exercised against a running server this time (the five redirect
pages are one `redirect()` each). The assistant's page context with a model (no test covers `page-context.ts`).

**Mismatches with the plan, reported, not improvised.**

1. *S3-D2 leaves the table's own classification without a control.* The Access tab's Classify dialog was the only
   place that set or removed a classification rule for **the whole table** (no column). S3-D1 gives columns their
   controls; nothing replaces the table-level one, so after S3-T4 a table-level rule can neither be set nor removed
   in the console (the header badge still shows the level and where it comes from). Built as decided; the planner
   should decide whether the header or the Overview gets a Set / Remove for the table, or whether a table is only
   ever as sensitive as its columns.
2. *S3-D9's first bullet is a self-link.* The asset page's only lineage link (`asset-lineage.tsx:298`, "Open lineage
   graph") is on the Lineage tab itself, so pointing it at `?tab=lineage` would link the page to itself. It was left
   pointing at `/lineage?focus=` (the full graph page, which S3-D9 keeps). Remove the button, or keep it?
3. *S3-D5 "every table's schedule".* `GET /api/governance/quality/schedules` lists only tables with a saved schedule
   (`routes/quality_schedules.rs:43`). The tab lists those and says in one line that any other table runs every
   N minutes (the endpoint's `defaultIntervalMinutes`). There is no Add on the Schedules tab (not decided); a new
   schedule is set from the table's Quality tab, and the empty state says so.
4. *S3-D3 filter left out.* The catalog list row carries the asset's own `classification` (asset level, which a
   table-level rule can raise on its own), not whether any column is classified, so a "Has sensitive columns" filter
   would be approximate (rule 14). Not added; no endpoint added.
5. *S3-D10 in part.* The sidebar removals are in S3-T6; the renames "Policies" → "Access rules" and "Teams & Roles"
   → "Roles" wait for S3-T5, so the sidebar never names a page by a title the page does not have.

**Choices made inside the decisions, for the reviewer.**

1. *One URL-tab helper.* Three pages needed the asset page's `?tab=` posture, so it is `useUrlTab`
   (`src/hooks/use-url-tab.ts`, pure parts and tests in `src/lib/url-tab.ts`): first tab is the bare address, an
   unknown value opens the first tab. The asset, connector and Sources pages keep their own copies (not refactored).
2. *Settings* uses the asset page's line tabs (six sections, sideways scroll on a phone), under one "Settings"
   header. The sections are the old components unchanged except their titles: Reporting's header said "Settings"
   and now says "Reporting"; Sign-in's said "Single Sign-On" and now says "Sign-in". Tag keys and Maps keep their own
   headers (they carry the New tag key action and the upload form). Only the open section loads.
   `/admin/sso` now redirects to `?section=sign-in` (was `/admin/settings`).
3. *Users and Data Quality* use the plain tab list, as Sources does. Invite User shows only on People; Add Quality
   Rule only on the rules tab, now named "Incidents and rules". The page descriptions were rewritten to say what the
   tabs hold. The stale user-guide line "Roles cannot yet be changed here once a person is invited" was dropped
   (`ADM-4` D1 shipped a role editor).
4. *Reuse, not copies (rule 4).* The asset page's freshness dialog moved to
   `governance/freshness-target-dialog.tsx`, keyed by the SLA's table key, with a Table field when adding from the
   tab; the schedule control moved into `ScheduleControls` in `quality-schedule.tsx`; `policyTable` /
   `policyCondition` moved to `catalog/asset-policy.ts` (their test file moved with them). A table key links only
   for `silver.` / `serving.` keys (`src/lib/table-key-href.ts`), which `lineage-graph.tsx`'s `nodeHref` now uses
   too; a Bronze key is plain text.
5. *The column section* sits under the opened column's facts on the Schema tab (`column-sensitivity.tsx`); the
   Sample tab's inspector opens the same `ColumnDetail` without it. When the section is shown, the facts no longer
   repeat Masked / Classification. The `masked` pill has the title "Masked for you", and the fact reads
   "Masked for you" (it is masked for the viewer, section 6.1).
   - Change is one action, delete then create. A failed create after a successful delete says the old level is
     already gone and reloads, so the page shows what is stored.
   - **Add a mask is enforced at once** (`activate: true`), because the section lists enforced masks only and a
     draft would be added and not appear. The policy name defaults to `mask_<table>_<column>_for_<roles>` (names are
     `UNIQUE`, `0003_governance.sql:48`) and can be edited.
   - A policy with a row filter counts as "does more than this column" even when it masks only this column, so it
     is not deleted from here ("Part of X, which also filters rows"): deleting it would also lift the row filter.
   - Drafts that mask the column are not listed (S3-D1 says enforced).
   - Without `policy:read` the API sends no roles: the section says "You. Who else it is masked for is shown with
     policy:read." (or "Not for you. …") instead of "No one".
6. *The asset page keeps its data on screen while it reloads* (`useService`'s existing `keepDataOnReload`), so the
   opened column stays open after a change instead of the page flashing to a skeleton. This applies to every
   reload on that page (certification, Access tab policies, and so on).
7. *Removed with the Classification page:* `classification-page.tsx`, `classification-columns.tsx`, their test, and
   `listClassifications` from the contract and client (its only consumer). The `GET /api/governance/classification`
   route is untouched (S3a-D4 is 3a's).
8. *Tests.* Added: `url-tab` (4), `table-key-href` (2), `column-masks` (4), `settings-page` (3), two Users tab cases,
   eight Data Quality tab cases, ten column-section cases in `asset-detail-tabs.test.tsx`, two pipeline lineage-link
   cases. Removed with what they tested: the six Access-tab classification cases (replaced by the column cases) and
   `classification-page.test.tsx`. Renamed: `asset-access.test.ts` → `asset-policy.test.ts`. One assertion changed
   its label: "Masked" → "Masked for you" (`asset-columns.test.tsx`).

**What S3-T5 needs when 3a lands:** merge `origin/feat/web-3-slice-3a-api` once its handoff is in; then Access rules
(S3-D4, read-only list with `assetId` links, drawer read-only), Reset on the Schedules tab (S3a-D3), Roles edit and
delete with the API's 409 sentence (S3-D8), the sidebar renames, and the user-guide rows for Policies and Roles.

## 9. Review (planner appends below)

### Review slice 2, round 1 (planner, 2026-11-04, at `e566187b`)

Read the diff (57 files, +2429/−2716) against S2-D1 to S2-D6 and the handoff. The design
choices 1–4 and 6–8 the handoff lists are accepted as made: the namespace filter row,
System status with exactly the six sections, counting only probed services, the latest
reading per slot and per table, the CDC panel in the connector's Overview, and leaving the
dated feature-page records alone. The command-palette test that failed once and then passed
3/3 alone and in the second full run is noted; CI is the judge.

- **R1 (BLOCKER). A healthy install without change capture shows an error on System
  status.** Handoff point 5: `GET /api/governance/replication` answers 503 when no CDC
  connector has ever run, because `lake.bronze_meta.replication_slot` does not exist yet
  (`routes/governance.rs`, the doc comment above `REPLICATION_SLOTS_SQL`: "surfaced as the
  standard 503"). With slice 2, every fresh install — the customer's first look at the
  product — opens System status with an error and Retry in one section. The connector list
  carries no adapter, so the console cannot tell on its own (it would need one ingest-spec
  read per connector). Fix it at the source:
  - `replication()` answers `{"replicationSlots": []}` (HTTP 200) when the table does not
    exist, decided by the engine's own "unknown table" signal — ClickHouse's error **code**
    (`UNKNOWN_TABLE`, 60) or an `EXISTS TABLE` probe, whichever `lakehouse_clickhouse` exposes;
    never by matching error text (principle 4), and never for any other failure: a
    ClickHouse that is down, a permission error or a malformed answer stays a 503.
  - Update the doc comment above `REPLICATION_SLOTS_SQL`, which says the opposite today, and
    cite `WEB-3 review R1` there.
  - Tests: the route with no table → 200 and an empty list; the route with ClickHouse down →
    503 (keep or add). A mutation check: put back the 503 for the missing table and the first
    test fails.
  - The console then shows its muted line for an empty list (already built) and the error
    only for a real outage; adjust the section's error sentence so it no longer names the
    missing-table cause.
  - This is a small Rust change inside the slice; scoped checks:
    `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets -- -D warnings`,
    `cargo test -p lakehouse-api <filter>`. Use one shared `CARGO_TARGET_DIR`; never
    `cargo clean`.
- **Follow-up, not this slice (handoff point 6):** Iceberg tables with no catalog entry
  (e.g. some Gold export tables) are no longer reachable from any list. The planner files it
  as a backlog item.

### Review slice 3b, round 1 (planner, 2026-11-04, at `03093545`)

S3-T1 to S3-T4 and S3-T6 read against S3-D1 to S3-D11. No BLOCKER. The handoff's design choices
are accepted: a mask added on a column is enforced at once; a policy that also filters rows is
not removed from the column; "roles hidden" without `policy:read`; `keepDataOnReload` on the
asset page; the shared `useUrlTab`, freshness dialog, schedule control and policy builders;
Bronze keys as plain text. The one full-suite failure (an `uploads-panel` poll count, passing
3/3 alone and in the second run) is noted; CI judges.

Decisions on the five mismatches the handoff reports:

1. **Table-level sensitivity (S3-D2 removed its only control).** Planner decision: it moves to
   the asset's About card (`catalog/asset-about.tsx`) as "Sensitivity (whole table)", with
   Set / Change / Remove, using the same classification calls with no column. The rule is
   unchanged: a table is never less strict than its strictest column, so the header badge may
   show a stricter level than the table-level rule, and the card says why in one muted line.
   Built in the next run, as **S3-T5a**, before S3-T5.
2. **The asset Lineage tab's "Open lineage graph" button** keeps pointing at
   `/lineage?focus=`: it opens the full page, so it is not a self-link. Accepted as built.
3. **Schedules lists only tables with a saved schedule**, with the default line. Accepted. Adding
   a schedule for another table from this tab is a follow-up (a table picker), not this slice.
4. **No "Has sensitive columns" filter.** Accepted: the list does not carry it, and an
   approximate filter would mislead.
5. **The sidebar renames ship with their pages (S3-T5).** Accepted.

Next run: S3-T5a now; S3-T5 when slice 3a's handoff entry is on its branch.

### Handoff (slice 3b, round 2) — developer, 2026-11-04

Branch `feat/web-3-slice-3b-screens`, on top of review round 1 (`dccddafc`).

| Commit | Task | What |
|---|---|---|
| `8d2ce73c` | S3-T5a | "Sensitivity (whole table)" on the About card (review round 1, decision 1) |
| — | the commit that adds this entry | This handoff |

**Built (S3-T5a), as decision 1 says.**
- The About card (`catalog/asset-about.tsx`) has a "Sensitivity (whole table)" row with
  Set / Change / Remove. It uses the same calls as a column: `createClassificationRule` with
  the table's lineage key and **no `column`**, and `deleteClassificationRule` for the table
  rule in force (the rule in `classificationRules` without a `column` key). Change removes
  that rule, then adds the new one; column rules are never touched. Needs `governance:write`;
  without it the level shows with no buttons.
- The level shown is the table rule's own, not the header's. When a column makes the header
  badge stricter, one muted line says so: "The header shows Restricted because column amount
  is Restricted: a table is never less restrictive than its most restrictive column."
  (`stricterBecauseOfColumns`). No line when the table rule is already the strictest.
- No second copy of the controls (rule 4): the column's `SensitivityControls` in
  `column-sensitivity.tsx` now takes an optional `column`; without it, it writes and words
  everything for the whole table (label, toast, remove dialog, half-done Change message).
  `LEVELS` is exported there as `SENSITIVITY_LEVELS`.
- Six tests in `asset-detail-tabs.test.tsx` ("About card: sensitivity of the whole table"):
  Set with no column, Change (delete then post, column rule left alone), Remove with what
  applies instead, the muted line, no line when the table rule is strictest, no buttons
  without `governance:write`.

**Still waits: S3-T5** (Access rules D4, schedule Reset D5, Roles edit/delete D8, sidebar
renames to "Access rules" and "Roles"). At this run `origin/feat/web-3-slice-3a-api` is at
`bf0c1786` with S3a-D1 to S3a-D4 committed, but its plan has no "Handoff (slice 3a)" entry
(`grep -n -i handoff` on that file finds slice 1 and slice 2 only). Per the brief, the branch
was not merged and S3-T5 was not started; no API shape was assumed.

**Commands run (foreground, on `8d2ce73c` plus this docs commit):**
- Per task: `bun run typecheck` (0 errors), `bun run lint` (0 errors, 10 warnings, all in
  files this slice does not touch), `bun test src/features/catalog/asset-detail-tabs.test.tsx`
  (143 pass, 0 fail), `bun test src/features/catalog` (350 pass, 0 fail, 14 files).
- Before handoff: `bun run typecheck && bun run lint && bun run test`: typecheck clean, lint
  0 errors / 10 warnings, test **3260 pass, 0 fail, 3261 tests across 331 files**.
- No Rust, Python or compose change, so no cargo, pytest or compose command was run.

**Not verified.** The About card row was not looked at in a running console (no dev stack
started); it is checked by the tests above only.
