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

## 9. Review (planner appends below)
