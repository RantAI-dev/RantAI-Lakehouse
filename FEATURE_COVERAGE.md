# Feature Coverage

Matrix of product surfaces vs implementation status. Update only with verified repository facts.

| Domain | Feature | Route | Previous reference | Target | Status | Mock | Real API | Notes |
|---|---|---|---|---|---|---|---|---|
| Overview | Overview dashboard | `/` | Dashboard (medallion) | Full KPI overview | implemented | yes | no | Platform + ops KPI rows; incidents → `/alerts` |
| Overview | Activity | `/activity` | — | Activity feed | implemented | yes | no | Filters + audit deep-link via `?event=` |
| Overview | Alerts | `/alerts` | — | Alerts list + triage | implemented | yes | no | Drawer; acknowledge/resolve via service |
| Data | Data Explorer | `/data` | `/` table browser | Tier + layer browse | implemented | yes | no | URL filters: search/tier/layer/type/class |
| Data | Asset detail | `/data/assets/[assetId]` | `/tables/[id]` | Full asset tabs | implemented | yes | no | Schema/sample/quality/lineage/policies/dependents/history |
| Data | Catalog | `/catalog` | `/data-catalog` | Namespaces | implemented | yes | no | Search + namespace drawer → browse assets |
| Data | Storage Lifecycle | `/storage` | — | Tier policies/ops | implemented | yes | no | Status badges; tier-lane visualization |
| Data | Connectors | `/connectors` | `/connectors` | List + detail drawer | implemented | yes | no | Wizard deferred; `getConnector` in drawer |
| Build | Pipelines | `/pipelines` | `/pipelines` | List + filters | implemented | yes | no | Builder wizard deferred |
| Build | Pipeline detail | `/pipelines/[pipelineId]` | `/pipelines/[id]` | Graph + runs | implemented | yes | no | Tabs: Overview/Graph/Runs + run drawer |
| Build | Streaming Jobs | `/streaming` | — | List + detail | implemented | yes | no | Tabs: Overview/Definition/Sources/Triggers/Checkpoints |
| Build | Query Studio | `/query-studio` | `/query-studio` | NL + SQL + transparency | implemented | yes | no | Internal tabs; `useServiceAction`; `?saved=` handoff |
| Build | Saved Queries | `/query-studio/saved` | partial | Saved list | implemented | yes | no | Secondary nav under Query Studio (not sidebar) |
| Build | Collaboration | `/query-studio/collaboration` | collaboration | Project list | implemented | yes | no | Secondary nav; project detail route deferred |
| Intelligence | Knowledge | `/knowledge` | `/intelligence-knowledge` | Sources | implemented | yes | no | Owner + index status; detail drawer |
| Intelligence | Vector Jobs | `/vector-jobs` | `/embeddings` | Vector jobs | implemented | yes | no | Moved from Build → Intelligence |
| Intelligence | Semantic Search | `/semantic-search` | `/semantic-search` | Hybrid search | implemented | yes | no | `useServiceAction`; strategy tabs |
| Intelligence | Agent Workflows | `/agents/workflows` | agentic dialog | Workflow list | implemented | yes | no | Visual builder deferred |
| Intelligence | Digital Employees | `/agents/employees` | — | List + detail | implemented | yes | no | RunTimeline; server-side approvals filter |
| Intelligence | Tool Registry | `/agents/tools` | — | Tool inventory | implemented | yes | no | Health + approval + deprecated |
| Governance | Policies | `/governance/policies` | `/data-governance` | Policy list | implemented | yes | no | Authoring wizard deferred; detail drawer |
| Governance | Classification & Masking | `/governance/classification` | governance tabs | Classification | implemented | yes | no | Review-status pills |
| Governance | Data Quality | `/governance/data-quality` | governance tabs | Quality rules | implemented | yes | no | CheckBadge + last run |
| Governance | Lineage | `/lineage` | `/lineage` | Graph + table | implemented | yes | no | FlowCanvas + edges table + column mappings |
| Governance | Audit | `/audit` | `/audit-logs` | Actor-chain audit | implemented | yes | no | Detail drawer; `?event=` correlation |
| Governance | Residency | `/residency` | tenant field | Residency rules | implemented | yes | no | Site pills + violation emphasis |
| Operations | Workloads | `/workloads` | — | Active/queued | implemented | yes | no | Cancel queued/running via service |
| Operations | Observability | `/observability` | — | SLO board | implemented | yes | no | Explorer deferred; CheckBadge for SLOs |
| Operations | Services | `/services` | — | Service health | implemented | yes | no | Detail drawer |
| Operations | Usage & Budgets | `/usage` | — | Usage summary | implemented | yes | no | Budget utilization pills |
| Admin | Users | `/admin/users` | `/user-management` | User list | implemented | yes | no | Detail drawer |
| Admin | Teams & Roles | `/admin/roles` | — | Roles | implemented | yes | no | |
| Admin | Tenants | `/admin/tenants` | `/tenant-management` | Tenants | implemented | yes | no | Detail drawer |
| Admin | Service Identities | `/admin/service-identities` | — | Service clients | implemented | yes | no | Rotation pills |
| Admin | Settings | `/settings` | — | Workspace settings | implemented | yes | no | Via `identityService.getWorkspaceSettings` |

**IA notes (sidebar):** Overview includes Alerts. Build lists Pipelines, Streaming Jobs, Query Studio only. Saved Queries and Collaboration are secondary tabs inside Query Studio. Vector Jobs lives under Intelligence (knowledge → index → search → agents).

**Taken out / de-duplicated from top-level nav:** Saved Queries and Collaboration as standalone sidebar items (still reachable via Query Studio tabs). Similarity Explorer remains a legacy redirect to Semantic Search.

**Legacy redirects:** `/data-catalog`, `/data-governance`, `/audit-logs`, `/user-management`, `/tenant-management`, `/intelligence-knowledge`, `/embeddings`, `/similarity-explorer`, `/tables/[id]`.
