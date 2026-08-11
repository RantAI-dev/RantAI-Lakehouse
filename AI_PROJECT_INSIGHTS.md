# AI Project Insights

Verified context for the **Rantai Lake** console after the lifecycle alignment pass. Update only with facts that are true in the repository.

---

## 1. Project Overview

**Rantai Lake** is an enterprise lakehouse console (**frontend preview only**) for:

- ingest via connectors;
- processing via data pipelines, streaming jobs, and knowledge/vector jobs;
- storage across **Hot / Warm / Cold / AI** physical tiers (logical Raw→Semantic is a separate catalog dimension);
- catalog, governance, residency, lineage, audit;
- Query Studio (NL + SQL) with execution transparency and simple federated plans;
- semantic search;
- agent workflows, digital employees, approvals, tool registry;
- workloads, observability, services, usage & budgets;
- users, roles, tenants, service identities.

### Honesty labels (do not conflate)

| Label | Meaning |
|---|---|
| **Implemented Frontend** | Routes, UX, navigation, and product copy exist |
| **Mocked Backend Contract** | Typed service + mock adapter returns realistic shapes/delays |
| **Not Yet Implemented** | No real engine, IdP, agent runtime, or HTTP API |

**There is no real data engine, query engine, pipeline runner, vector database, streaming engine, governance enforcer, identity provider, agent runtime, or observability backend in this repository.**

Visual identity: **Rantai Design System** (`design-system/`) — navy/blue OKLCH, Geist, dark default. Do not change design-system tokens when adding features.

---

## 2. Tech Stack

| Category | Choice |
|---|---|
| Framework | Next.js 16 App Router, React 19 |
| Language | TypeScript 5 strict |
| Styling | Tailwind CSS v4 + design-system tokens |
| UI | shadcn/`@base-ui/react` in `src/components/ui/` |
| Icons | lucide-react |
| Theme | next-themes (design-system ThemeProvider) |
| SQL editor | `@uiw/react-codemirror` + SQL lang |
| State | Local `useState` / `useService` / `useServiceAction` — no global store |
| Tests | Node test runner (`node --test`) for pure helpers |

---

## 3. Folder Structure

```
src/
  app/                    # Thin route files + route groups
    (overview|data|build|intelligence|governance|operations|admin)/
  components/
    app-shell/            # Sidebar, navbar, nav-config (IA)
    patterns/             # PageHeader, DataTable, StatusBadge family, DetailDrawer, …
    ui/                   # shadcn primitives
  features/               # Feature modules (pages compose these)
  services/
    contracts/            # Typed interfaces (swap target for real APIs)
    mock/                 # Mock adapters + in-session stores
    transport.ts          # mockCall / mockProgress
    errors.ts
    index.ts              # Service registry
  hooks/use-service.ts
  lib/status.ts           # Shared taxonomies (includes blocked, partial)
  lib/format.ts
docs/
  UX_FLOWS.md
  FEATURE_COVERAGE.md
  RANTAI_LAKE_REPOSITORY_VALIDATION.md
```

Pages must stay thin. Do not put large fixtures inside `app/**/page.tsx`.

---

## 4. Information Architecture

Sidebar groups (`src/components/app-shell/nav-config.ts`):

- **Overview:** `/`, `/activity`, `/alerts`
- **Data:** `/data`, `/catalog`, `/storage`, `/connectors`
- **Build:** `/pipelines`, `/streaming`, `/query-studio`
- **Intelligence:** `/knowledge`, `/vector-jobs`, `/semantic-search`, `/agents/workflows`, `/agents/employees`, `/agents/approvals`, `/agents/tools`
- **Governance:** policies, classification, data-quality, lineage, audit, residency
- **Operations:** workloads, observability, services, usage
- **Administration:** users, roles, tenants, service-identities, settings

**Secondary routes:** `/query-studio/saved`, `/query-studio/collaboration`.

Product lifecycle the UI must keep coherent:

```text
Connect → Process → Store → Govern → Discover → Query → Automate → Audit → Monitor
```

See `docs/FEATURE_COVERAGE.md` and `docs/RANTAI_LAKE_REPOSITORY_VALIDATION.md`.

---

## 5. Data Flow

```
Page (features/*)
  → useService(fetcher)           # list/detail loads
  → useServiceAction(action)      # mutations (run, ack, cancel, search, decide)
  → services/index.ts (registry)
  → mock adapter
  → mockCall(delay, abort)
  → typed result
```

When wiring a real backend: add `services/clients/*` implementing the same contract and point `services/index.ts` at it. Pages should not need redesign.

Canonical service registry today: overview, asset, pipeline, streaming, query, knowledge, agent, governance, ops, identity, connector, storage.

Folded names (by design): Catalog→AssetService, Lineage/Audit→GovernanceService, Observability/Usage→OpsService.

---

## 6. Shared Patterns

Prefer these over page-local inventions:

- `PageHeader` / `EntityHeader`
- `FilterToolbar` / `SearchField` / `FilterSelect`
- `DataTable`
- Status badge family (`StatusBadge`, `TierBadge`, `HealthBadge`, `ApprovalBadge`, …)
- `FreshnessIndicator`, `MetricCard` / `MetricGrid`
- `EmptyState` / `ErrorState` / `PermissionState` / `LoadingSkeleton`
- `DetailDrawer`, `FlowCanvas`, `RunTimeline`, `CodeBlock`, `SectionCard`, `MetadataList`
- `ConfirmActionDialog` (optional children for comment fields)
- `FormStepLayout` / `CreateSheet` for create flows

Status strings must come from `src/lib/status.ts`.

Copy rule: product-neutral engine labels (Hot analytical store, Federated compute, …). Infra names only in advanced/diagnostic copy.

List pages: **PageHeader → FilterToolbar → loading/error → DataTable** (+ DetailDrawer when no detail route).

Physical storage tier ≠ logical data layer — UI must keep both distinct.

---

## 7. Known Gaps

### Implemented Frontend + Mocked Backend Contract

- Approvals inbox with approve/reject
- Pipeline run cancel/retry + checkpoint/audit links
- Query execution plan panel + audit handoff
- Cross-links across connectors, pipelines, assets, streaming, knowledge, agents
- Storage restore/rehydrate mock

### Not Yet Implemented (deferrals)

- Real HTTP adapters / engines / IdP / agent runtime
- Dedicated connector detail route (drawer today)
- Dedicated pipeline run route (drawer today)
- Agent workflow detail canvas route
- Observability metric/log/trace explorer
- Collaboration project detail route
- Playwright e2e suite
- Navbar command palette / global search (shell only)

---

## 8. Scripts

```bash
npm run dev
npm run build
npm run lint
npm test
npx tsc --noEmit
```

---

## 9. Rules for Future Changes

1. Do not copy monolithic legacy pages into features.
2. Add service contract + mock before UI.
3. Keep pages thin; logic lives in `features/` and `services/`.
4. Reuse patterns; do not invent new table/status shells.
5. Never claim mock behavior as production backend capability.
6. Update `docs/FEATURE_COVERAGE.md`, `docs/RANTAI_LAKE_REPOSITORY_VALIDATION.md`, and this file only with proven facts.
7. Preserve design-system tokens and dark product default.
8. Prefer `useService` / `useServiceAction`; do not invent page-local fetch state.
9. Keep related entities navigable via shared IDs (pipeline ↔ asset ↔ audit ↔ agent).
10. End-to-end create/ops UX flows: `docs/UX_FLOWS.md`.

---

_Last reviewed: lifecycle alignment — cross-links, approvals, query plans, storage restore, validation docs._
