# RantAI Lake — Repository Validation Report

Status: verified against the repository after the lifecycle alignment pass (frontend-only, mocked backend).

---

## Executive Summary

The console already covered most of the target RantAI Lake lifecycle as separate product surfaces. The remaining product gap was **connection**: related entities existed, but users could not reliably navigate Connector → Pipeline → Run → Dataset → Catalog → Lineage → Audit, or Agent → Approval → Audit.

This pass:

- aligned mock entity IDs across domains;
- wired cross-navigation CTAs;
- added Approvals inbox with decide actions;
- added federated/simple execution-plan UI in Query Studio;
- added pipeline run cancel/retry + checkpoint/audit links;
- clarified Storage Hot/Warm/Cold/AI vs logical Raw→Gold;
- added mock restore/rehydrate;
- kept all backend behavior behind typed mock adapters.

The frontend now represents one coherent platform story while remaining honest that **every engine capability is mocked**.

---

## Current Repository State

| Area | State |
|---|---|
| Stack | Next.js App Router, React, TypeScript strict, Tailwind, design-system |
| Data access | `src/services/contracts` + `src/services/mock` via `src/services/index.ts` |
| Real backends | **None** (no query engine, pipeline runner, vector DB, IdP, agent runtime, observability backend) |
| Pages | Thin `src/app/**/page.tsx` → feature modules in `src/features/**` |
| IA | Sidebar groups in `src/components/app-shell/nav-config.ts` |

Target lifecycle represented in navigation:

```text
Connect → Process → Store → Govern → Discover → Query → Automate → Audit → Monitor
```

---

## Target Product Flow

```text
Data Source → Connector → Processing → Storage → Catalog & Governance
  → Query / Search / Application / Agent → Audit & Lineage → Monitoring
```

Processing paths:

```text
Data Pipeline | Streaming Pipeline | Knowledge / Vector Pipeline
```

Consumption paths:

```text
Query Studio | Semantic Search | Dashboard / Application | AI Agent
```

Agent path:

```text
Trigger → Resolve Context → Query / Retrieve → Analyze → Risk / Policy
  → Approval? → Execute → Audit + Lineage
```

---

## Feature Coverage Matrix

| Feature | Route | Status | Target Behavior | Gap / Notes | Priority |
|---|---|---|---|---|---|
| Connectors | `/connectors`, `/connectors/create` | Partial → improved | List, configure, test, discover, activate | Drawer detail (no dedicated route); test/discover mocked | P1 route |
| Pipelines | `/pipelines`, create, detail | Partial → improved | Graph, runs, cancel/retry, asset links | Dedicated run route deferred | P1 |
| Streaming | `/streaming`, create, detail | Partial → improved | Lag/throughput, checkpoints, triggers → agents | Live animation intentionally avoided | — |
| Knowledge / Vector | `/knowledge`, `/vector-jobs` | Partial → improved | Source → index → search links | Re-index/re-embed actions still light | P1 |
| Semantic Search | `/semantic-search` | Partial → improved | Hybrid strategies + citations + catalog links | Filters limited | P1 |
| Catalog / Explorer | `/catalog`, `/data`, asset detail | Partial → improved | Namespace → asset → linked tabs | Sample always mocked | — |
| Storage lifecycle | `/storage` | Partial → improved | Hot/Warm/Cold/AI + restore; layer ≠ tier | Full lifecycle history light | P1 |
| Query Studio | `/query-studio` (+ saved/collab) | Partial → improved | NL + SQL, estimate, plan, audit | Streaming result rows simulated as batch | — |
| Governance | policies, classification, quality, residency | Partial | Policy create + impact preview | Impact simulation still static | P1 |
| Lineage | `/lineage` | Partial | Graph + table | Focus query param soft | P1 |
| Audit | `/audit` | Partial | Actor chain + `?event=` | Cross-domain event seeding incomplete | P1 |
| Agent workflows | `/agents/workflows` | Partial | Create + list; canvas on create | Detail route + canvas on detail missing | P1 |
| Digital employees | `/agents/employees/[id]` | Partial → improved | Runs + approvals linked | — | — |
| Approvals | `/agents/approvals` | **Added** | Inbox, evidence, approve/reject, audit | — | — |
| Workloads / Observability / Services / Usage | ops routes | Partial | Health, cancel, budgets | Trace/log explorer deferred | P1 |
| Admin | users, roles, tenants, identities, settings | Partial | CRUD sheets | No real IdP | — |

Statuses used: **Complete** (for FE representation), **Partial**, **Missing**, **Needs Refactor**, **Needs Relocation**. No major domain is Missing after this pass; remaining work is deepening, not inventing surfaces.

---

## Missing Features

None of the P0 product domains are entirely missing as routes. Deferred / still light:

- Dedicated connector detail route
- Dedicated pipeline run detail route
- Agent workflow detail with stored FlowCanvas
- Observability metric/log/trace explorer
- Collaboration project detail
- Real HTTP adapters

---

## Partial Features

- Governance policy impact / conflict simulation
- Lineage column-level depth
- Streaming → materialized view asset always linked by index order
- Audit event corpus not exhaustively correlated for every mock action
- System states: Empty / Failed / Permission Denied exist; Blocked seeded in query history; Partial on some pipeline runs; Cancelled via workloads/run cancel

---

## Refactored Features

- Connector dependents now `{ id, name, kind }` with navigable links
- Pipeline/run contracts gained relationship + cancel/retry fields
- Query estimate/result gained `plan` + `auditEventId`
- Knowledge/vector/search share `sourceId` / `assetId`
- Streaming triggers use `targetHref`
- Storage restore via `StorageService.restoreAsset`
- Agent `decideApproval` + Approvals inbox
- EntityStatus gained `blocked` and `partial`

---

## Route Changes

| Change | Detail |
|---|---|
| Added | `/agents/approvals` |
| Nav | Intelligence → Approvals |
| Unchanged | Legacy redirects (`/data-catalog`, `/embeddings`, …) |

---

## Shared Component Changes

- `ConfirmActionDialog` accepts optional `children` (comment field for approvals)
- Status badge tones for `blocked` / `partial`
- No visual identity / design-system token changes

---

## Service Contract Changes

Extended (still mock-backed):

- `ConnectorService.testConnection`, richer `ConnectorDetail`
- `PipelineService.cancelRun`, `retryRun`, asset/connector IDs on models
- `QueryService` plan stages + audit ids on results/history
- `KnowledgeService` relationship IDs on sources/jobs/hits
- `StreamingService` `targetHref`, `sinkAssetIds`
- `StorageService.restoreAsset`
- `AgentService.decideApproval`, richer `ApprovalItem`

Not split into separate named services (folded by design): Catalog→`AssetService`, Lineage/Audit→`GovernanceService`, Observability/Usage→`OpsService`. Pages should not care; adapters stay swappable.

---

## Known Gaps

1. Mock creates are in-session only (lost on full reload).
2. Audit `?event=` deep-links work when IDs match seeded events; not every generated id has a matching audit row.
3. No Playwright e2e suite for the critical flows.
4. Navbar command palette remains shell-only.

---

## Future Backend Integration Points

Replace mock adapters in `src/services/index.ts` with HTTP/client implementations of the same contracts:

```text
Page → Typed Service → Real API Adapter
```

Do **not** redesign pages when wiring backends. Introduce `services/clients/*` and keep mock adapters for local/demo.

---

## Critical Flow Verification (manual checklist)

```text
Connector → Pipeline → Run → Dataset → Catalog → Lineage
Streaming Source → Job → Materialized View → Trigger → Agent
Document → Knowledge Source → Vector Job → Semantic Search
Query Studio → Execute → Result → Metrics → History → Audit
Agent → Query / Retrieve → Approval → Action → Audit
Dataset → Governance → Policy → Impact → Audit
```

All of the above are navigable in the UI with mocked outcomes after this pass.
