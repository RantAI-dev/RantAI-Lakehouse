# Feature Coverage

Status tags:

- `[COMPLETE]` — frontend represents the product behavior end-to-end (mocked backend)
- `[PARTIAL]` — primary surface exists; depth or cross-links still limited
- `[MOCKED]` — contract + mock adapter only; no real backend
- `[MISSING]` — not represented in the UI yet

Update only with verified repository facts.

| Domain | Feature | Route | Status | Backend |
|---|---|---|---|---|
| Overview | Platform overview | `/` | [COMPLETE] | [MOCKED] |
| Overview | Activity feed | `/activity` | [COMPLETE] | [MOCKED] |
| Overview | Alerts triage | `/alerts` | [COMPLETE] | [MOCKED] |
| Data | Data Explorer (tier + layer) | `/data` | [COMPLETE] | [MOCKED] |
| Data | Asset detail tabs | `/data/assets/[assetId]` | [COMPLETE] | [MOCKED] |
| Data | Catalog namespaces | `/catalog` | [COMPLETE] | [MOCKED] |
| Data | Storage lifecycle + restore | `/storage` | [COMPLETE] | [MOCKED] |
| Data | Connectors list / create / test / discover | `/connectors` | [COMPLETE] | [MOCKED] |
| Build | Pipelines list / create / agentic draft | `/pipelines` | [COMPLETE] | [MOCKED] |
| Build | Pipeline detail + graph + runs | `/pipelines/[pipelineId]` | [COMPLETE] | [MOCKED] |
| Build | Pipeline run cancel / retry | (run drawer) | [COMPLETE] | [MOCKED] |
| Build | Streaming jobs | `/streaming` | [COMPLETE] | [MOCKED] |
| Build | Streaming detail / triggers | `/streaming/[jobId]` | [COMPLETE] | [MOCKED] |
| Build | Query Studio NL + SQL | `/query-studio` | [COMPLETE] | [MOCKED] |
| Build | Federated execution plan | `/query-studio` | [COMPLETE] | [MOCKED] |
| Build | Saved queries | `/query-studio/saved` | [COMPLETE] | [MOCKED] |
| Build | Collaboration projects | `/query-studio/collaboration` | [PARTIAL] | [MOCKED] |
| Intelligence | Knowledge sources | `/knowledge` | [COMPLETE] | [MOCKED] |
| Intelligence | Vector jobs | `/vector-jobs` | [COMPLETE] | [MOCKED] |
| Intelligence | Semantic search | `/semantic-search` | [COMPLETE] | [MOCKED] |
| Intelligence | Agent workflows | `/agents/workflows` | [PARTIAL] | [MOCKED] |
| Intelligence | Digital employees | `/agents/employees` | [COMPLETE] | [MOCKED] |
| Intelligence | Approvals inbox | `/agents/approvals` | [COMPLETE] | [MOCKED] |
| Intelligence | Tool registry | `/agents/tools` | [COMPLETE] | [MOCKED] |
| Governance | Policies | `/governance/policies` | [PARTIAL] | [MOCKED] |
| Governance | Classification & masking | `/governance/classification` | [COMPLETE] | [MOCKED] |
| Governance | Data quality | `/governance/data-quality` | [COMPLETE] | [MOCKED] |
| Governance | Lineage | `/lineage` | [PARTIAL] | [MOCKED] |
| Governance | Audit | `/audit` | [PARTIAL] | [MOCKED] |
| Governance | Residency | `/residency` | [COMPLETE] | [MOCKED] |
| Operations | Workloads | `/workloads` | [COMPLETE] | [MOCKED] |
| Operations | Observability | `/observability` | [PARTIAL] | [MOCKED] |
| Operations | Services | `/services` | [COMPLETE] | [MOCKED] |
| Operations | Usage & budgets | `/usage` | [COMPLETE] | [MOCKED] |
| Admin | Users / roles / tenants / service identities / settings | `/admin/*`, `/settings` | [COMPLETE] | [MOCKED] |

## Intentionally missing (not product blockers for FE preview)

| Item | Status |
|---|---|
| Real connectors / engines / IdP / agent runtime | [MISSING] (backend) |
| Dedicated connector detail route | [PARTIAL] (drawer sufficient for preview) |
| Dedicated pipeline run route | [PARTIAL] (drawer + actions) |
| Workflow detail canvas route | [PARTIAL] |
| Observability log/trace explorer | [MISSING] |
| HTTP service adapters | [MISSING] |

## Cross-domain connections (must stay linked)

| From | To | Status |
|---|---|---|
| Connector | Pipeline / Streaming | [COMPLETE] |
| Pipeline / Run | Dataset / Lineage / Audit | [COMPLETE] |
| Dataset | Policies / Lineage / Query / Storage | [COMPLETE] |
| Streaming trigger | Agent workflow / employee | [COMPLETE] |
| Knowledge | Vector job / Semantic search / Catalog | [COMPLETE] |
| Query result / history | Audit | [COMPLETE] |
| Agent run | Approval / Audit | [COMPLETE] |
| Storage op | Catalog asset | [COMPLETE] |
