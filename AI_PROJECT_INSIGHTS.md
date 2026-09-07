# AI Project Insights

Verified context for the **Rantai Lake** console after the lifecycle alignment pass, and after the P0-P6 Lakehouse Foundation build (Rust backend cutover + a real Bronze Iceberg/Lakekeeper/RustFS/CDC layer). Update only with facts that are true in the repository.

**This section historically said "frontend preview only" and "no real
backend anywhere in this repository." That has not been true since the
Phase 2 Rust backend cutover, and is even less true after P6. See "What is
real" below before assuming anything here is mocked.**

---

## 1. Project Overview

**Rantai Lake** is an enterprise lakehouse console — a Next.js/React
frontend backed by a real Rust (axum) API over Postgres, ClickHouse, and
(optionally) Dagster/Debezium/Trino — for:

- ingest via connectors;
- processing via data pipelines and knowledge/vector jobs;
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
| **Real Backend** | Backed by `lakehouse-api` (Rust/axum) over Postgres, ClickHouse, or Dagster — a genuine HTTP round trip, not a mock |
| **Mocked Backend Contract** | Typed service + mock adapter returns realistic shapes/delays; no real backend |
| **Not Yet Implemented** | No real engine, IdP, agent runtime, or HTTP API for this specific capability |

### What is real (verify against `src/services/index.ts` before trusting this list)

- **Query engine:** ClickHouse (`serving.*`/`silver.*` marts, Query Studio SQL execution).
- **OLTP store:** Postgres, via `lakehouse-store` (identity, governance rules, pipelines, connectors, knowledge sources/vector jobs, agents, saved queries, storage policies).
- **Pipeline runner / orchestration:** Dagster (opt-in `dagster` compose profile) — batch ingest (dlt), Bronze maintenance (`expire_snapshots`), replication-slot metrics.
- **Identity provider:** local password + session + service-token auth are real (`lakehouse-auth`); OIDC is a real resource-server verifier, not a mock.
- **A lakehouse layer:** Bronze data as Apache Iceberg on RustFS/SeaweedFS object storage, registered in Lakekeeper's Iceberg REST catalog, written by Dagster/dlt (batch) and Debezium Server (CDC), read by ClickHouse's `DataLakeCatalog`. Verified end to end — see `docs/plans/G1-RESULT.md` through `docs/plans/P5-RESULT.md`. Console surfaces: Catalog (Bronze tables appear as `iceberg-table` assets), Storage (Warm-tier byte estimate), Governance → "Bronze Maintenance" and "Ingestion (CDC)".
- **10 of 12 `src/services/index.ts` domains are real**, not mocked: overview, assets/catalog, pipelines, query studio, agents, governance, ops, identity, connectors, storage.

### What is NOT real (still genuinely mocked or missing)

- **No streaming engine.** No Kafka/Redpanda/Pulsar/Flink anywhere in this repository. The console's `streaming` domain (`src/services/mock/streaming.ts`, `/streaming` routes) fabricated `kafka.*` topics, lag, throughput, and checkpoint data and rendered it as if real — that was a locked-decision violation, so it was removed outright rather than kept mocked. CDC via Debezium Server exists and is real, but a change-data-capture pipe into Bronze Iceberg is not a streaming engine and is never described as one.
- **No vector database / embedding-backed search.** `knowledgeService.search` stays mocked; knowledge *sources* and *vector jobs* are real (Postgres), the search-query path is not.
- **No agent/tool execution runtime.** Digital-employee/agent definitions, runs, and approvals are real (Postgres), but nothing actually executes an agent or a tool.
- **No live connector health checks.** `testConnection` bumps a timestamp and returns hardcoded latency; it dials nothing (`lakehouse-store/src/connectors.rs`).
- **ClickHouse cannot write Iceberg through the catalog on 26.3** (`CREATE TABLE`/`INSERT` fail or segfault — see `docs/plans/G1-RESULT.md`), and **in-engine Bronze compaction is limited to `expire_snapshots`** (`remove_orphan_files`/`OPTIMIZE` don't work — `docs/plans/G3-RESULT.md`, ADR 0009's Trino-as-cron escape hatch).
- **Lakekeeper authorization is `allow-all`, not enforced** (R1 open — `docs/plans/P5-REPORT.md`).

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
- **Build:** `/pipelines`, `/query-studio`
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
  → services/clients/* (real, most domains) or services/mock/* (knowledge.search)
  → apiFetch → /api/* → next.config.ts rewrite → lakehouse-api (Rust)  — or mockCall(delay, abort) for mocked domains
  → typed result
```

10 of 11 domains already go through `services/clients/*` to the real Rust
API, not `mock adapter`/`mockCall` — see §1's "What is real". When wiring a
still-mocked domain: add `services/clients/*` implementing the same
contract and point `services/index.ts` at it. Pages should not need
redesign.

Canonical service registry today: overview, asset, pipeline, query, knowledge, agent, governance, ops, identity, connector, storage.

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

**Superseded by "What is real" / "What is NOT real" in §1** — the two
sub-lists below predate the Rust backend cutover and mislabeled most of the
product as mocked. Kept only for the items still genuinely gaps:

### Genuinely still mocked or missing

- Semantic search (no vector store — see §1)
- Agent/tool execution runtime (definitions/runs are real; nothing executes)
- Live connector health checks (`testConnection` dials nothing)
- Workspace settings (`getWorkspaceSettings` returns a fixed response; no setter)

### Not Yet Implemented (deferrals)

- Vector store / agent execution runtime / live connector dialing (see §1)
- Dedicated connector detail route (drawer today)
- Dedicated pipeline run route (drawer today)
- Agent workflow detail canvas route
- Observability metric/log/trace explorer
- Collaboration project detail route
- Playwright e2e suite
- Navbar command palette / global search (shell only)

---

## 8. Scripts

Runtime: **Bun** (`>= 1.3.0`). Jangan pakai `npm`/`npx` — lockfile-nya `bun.lock`.

```bash
bun install
bun run dev
bun run build
bun run lint
bun run test
bun run typecheck
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
