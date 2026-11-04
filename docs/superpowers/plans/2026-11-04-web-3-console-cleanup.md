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

## 5. Slice 2 (outline; plan check before it starts)

- **Catalog and Data Explorer become one Catalog page** (both page through
  `assetService.listAssetsPage`).
- **System status**: Health, Services, Capacity, Ingestion (CDC) and the Table Maintenance
  list on one page; CDC health also on its connector in Sources; maintenance status on the
  table's Overview.

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

## 9. Review (planner appends below)
