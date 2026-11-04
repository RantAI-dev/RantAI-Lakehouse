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

## 6. Slice 3 (outline; plan check before it starts)

- Column sensitivity and masks set on the table's columns; the Classification & Masking page
  goes; a "sensitive columns" filter in the Catalog. The classification `masking_rule` text
  field is replaced by a real column-mask policy (it is never applied today).
- Policies becomes a read-only "Access rules" overview linking to each table.
- Data Quality gains the check schedules and freshness targets that live only on the table
  page.
- Tag keys and Maps become Settings sections; Sessions a tab in Users; "Teams & Roles"
  becomes "Roles", with edit and delete.
- The Lineage page's deep links move to the table's Lineage tab; the page goes.

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

## 9. Review (planner appends below)
