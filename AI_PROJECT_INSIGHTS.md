# AI Project Insights

Ringkasan konteks repository **Rantai Lake** console setelah rebuild dan hardening UX/IA sesuai *New Repository Implementation Plan*. Update file ini hanya dengan fakta yang sudah terverifikasi di kode.

---

## 1. Project Overview

**Rantai Lake** adalah enterprise lakehouse console (UI preview) untuk:

- data across **Hot / Warm / Cold / AI** storage tiers (logical layers Raw→Semantic tetap sebagai filter sekunder);
- pipelines, streaming jobs, vector/knowledge jobs;
- Query Studio (NL + SQL) dengan execution transparency;
- catalog, lineage, governance, residency, audit;
- agent workflows, digital employees, tool registry, approvals;
- workloads, observability, services, usage & budgets;
- users, roles, tenants, service identities.

**Status integrasi:** semua data melalui **typed service contracts** + **mock adapters** di `src/services/`. Tidak ada backend nyata. Jangan mengklaim kemampuan engine sebagai “live”.

Visual identity: **Rantai Design System** (`design-system/`) — navy/blue OKLCH, Geist, dark default. Jangan ubah design system tokens/visual language saat menambah fitur.

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
    overview|catalog|pipelines|streaming|queries|knowledge|
    agents|governance|ops|admin|connectors|storage/
  services/
    contracts/            # Typed interfaces
    mock/                 # Mock adapters + fixtures
    transport.ts          # Shared mockCall / mockProgress
    errors.ts
    index.ts              # Service registry (swap mocks later)
  hooks/use-service.ts    # useService + useServiceAction
  lib/status.ts           # Shared taxonomies + label maps
  lib/format.ts           # Shared formatters
```

Pages must stay thin. Do not put large fixtures inside `app/**/page.tsx`.

---

## 4. Information Architecture

Sidebar groups (`src/components/app-shell/nav-config.ts`):

- **Overview:** `/`, `/activity`, `/alerts`
- **Data:** `/data`, `/catalog`, `/storage`, `/connectors`
- **Build:** `/pipelines`, `/streaming`, `/query-studio`
- **Intelligence:** `/knowledge`, `/vector-jobs`, `/semantic-search`, `/agents/*`
- **Governance:** policies, classification, data-quality, lineage, audit, residency
- **Operations:** workloads, observability, services, usage
- **Administration:** users, roles, tenants, service-identities, settings

**Secondary routes (not in sidebar):** `/query-studio/saved`, `/query-studio/collaboration` — reached via `QueryStudioTabs` under Query Studio. Titles resolve via `pageTitleFor`; active sidebar item stays Query Studio (`activeNavHref`).

See `FEATURE_COVERAGE.md` for the full matrix.

---

## 5. Data Flow

```
Page (features/*)
  → useService(fetcher)           # list/detail loads
  → useServiceAction(action)      # user-triggered mutations (run, ack, cancel, search)
  → services/index.ts (registry)
  → mock adapter
  → mockCall(delay, abort)
  → typed result
```

When wiring a real backend: add `services/clients/*` implementing the same contract and point `services/index.ts` at it. Pages should not change.

**Service method naming (canonical):** `listX` / `getX` (e.g. `listPipelines`, `getPipeline`, `listJobs`, `getJob`, `listConnectors`, `getConnector`). Health fields use `health: Health` (not `status`) on connectors and platform services.

---

## 6. Shared Patterns

Prefer these over page-local inventions:

- `PageHeader` / `EntityHeader`
- `FilterToolbar` / `SearchField` / `FilterSelect`
- `DataTable`
- `StatusBadge` / `TierBadge` / `HealthBadge` / `AutonomyBadge` / `SeverityBadge`
- `CheckBadge` / `ApprovalBadge` / `OutcomeBadge` / `AlertStatusBadge` / `WorkloadStatusBadge`
- `Pill` (generic) for domain-local statuses
- `FreshnessIndicator`
- `MetricCard` / `MetricGrid`
- `EmptyState` / `ErrorState` / `PermissionState` / `LoadingSkeleton` / `MetricSkeleton`
- `DetailDrawer`, `FlowCanvas`, `RunTimeline`, `CodeBlock`, `SectionCard`, `MetadataList`

Status strings must come from `src/lib/status.ts` (includes `EntityStatus`, `Health`, `CheckStatus`, `ApprovalStatus`, `ActorKind`, `AuditOutcome`, `AlertStatus`, `WorkloadStatus`, …).

Copy rule: use **product-neutral** labels for engines/gateway (Hot analytical store, Federated compute, …). Infra names belong in advanced tooltips only.

List pages follow: **PageHeader → FilterToolbar → loading/error → DataTable** (+ DetailDrawer when no detail route).

---

## 7. Known Gaps (intentional deferrals)

- Full pipeline / connector create wizards
- Visual agent workflow builder canvas
- Observability metric/log/trace explorer
- Collaboration project detail route
- Playwright e2e suite (unit tests cover format helpers)
- HTTP adapters for real APIs
- Navbar command palette / global search (UI shell only)

---

## 8. Scripts

```bash
npm run dev
npm run build
npm run lint
npm test          # node --test on src/lib/*.test.ts
npx tsc --noEmit
```

Verified baseline (post IA/UX hardening): `tsc` clean, `eslint` 0 errors (1 pre-existing warning in `ui/sidebar.tsx`), `npm test` 3 pass, `next build` success (44 routes).

---

## 9. Rules for Future Changes

1. Do not copy monolithic legacy pages into features.
2. Add service contract + mock before UI.
3. Keep pages thin; logic lives in `features/` and `services/`.
4. Reuse patterns; do not invent new table/status shells.
5. Never claim mock behavior as production backend capability.
6. Update `FEATURE_COVERAGE.md` and this file only with proven facts.
7. Preserve design-system tokens and dark product default.
8. Prefer `useService` for loads and `useServiceAction` for mutations; do not invent page-local fetch state.

---

_Last reviewed: IA consolidation + cross-module UX hardening (filters, drawers, shared badges, contract cleanup) against RantAI-Lake-New-Repository-Implementation-Plan._
