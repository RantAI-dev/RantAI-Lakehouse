# Feature Coverage

**Updated in P6.** This file predates the Rust backend cutover (P0–P6) and
previously described every domain as a mock adapter. That was already false
before P6 (10 of 12 `src/services/index.ts` domains were already real) and
is more false now that the lakehouse layer (Bronze Iceberg/Lakekeeper/
RustFS, CDC via Debezium, Dagster-run maintenance) is surfaced through the
Catalog, Storage, and Governance domains. The table below reflects the
actual `src/services/index.ts` wiring, verified against
`src/services/clients/*` and the Rust routes they call.

Status tags:

- `[COMPLETE]` — frontend represents the product behavior end-to-end
- `[PARTIAL]` — primary surface exists; depth or cross-links still limited
- `[REAL]` — backed by the real Rust API (ClickHouse/Postgres/Dagster), not a mock
- `[MOCKED]` — contract + mock adapter only; no real backend
- `[MISSING]` — not represented in the UI yet

Update only with verified repository facts.

| Domain | Feature | Route | Status | Backend |
|---|---|---|---|---|
| Overview | Platform overview | `/` | [COMPLETE] | [REAL] |
| Overview | Activity feed | `/activity` | [COMPLETE] | [REAL] |
| Overview | Alerts triage | `/alerts` | [COMPLETE] | [REAL] |
| Data | Data Explorer (tier + layer) | `/data` | [COMPLETE] | [REAL] |
| Data | Asset detail tabs | `/data/assets/[assetId]` | [COMPLETE] | [REAL] |
| Data | Catalog namespaces (incl. Bronze Iceberg tables) | `/catalog` | [COMPLETE] | [REAL] |
| Data | Storage lifecycle + restore (Hot/Warm real; Cold/AI always 0 — see README limitations) | `/storage` | [COMPLETE] | [REAL] |
| Data | Connectors list / create / test / discover | `/connectors` | [COMPLETE] | [REAL] (CRUD is real Postgres; `testConnection` returns hardcoded latency — see `lakehouse-store/src/connectors.rs`, not a live socket check) |
| Build | Pipelines list / create / agentic draft | `/pipelines` | [COMPLETE] | [REAL] |
| Build | Pipeline detail + graph + runs | `/pipelines/[pipelineId]` | [COMPLETE] | [REAL] |
| Build | Pipeline run cancel / retry | (run drawer) | [COMPLETE] | [REAL] |
| Build | Query Studio NL + SQL | `/query-studio` | [COMPLETE] | [REAL] |
| Build | Federated execution plan | `/query-studio` | [COMPLETE] | [PARTIAL] — query execution itself is real (ClickHouse); "federated" is UI copy, not multi-engine execution (Trino exists only as a background Bronze compactor, per ADR 0009, and is never in the query-serving path) |
| Build | Saved queries | `/query-studio/saved` | [COMPLETE] | [REAL] |
| Build | Collaboration projects | `/query-studio/collaboration` | [PARTIAL] | [REAL] |
| Intelligence | Knowledge sources | `/knowledge` | [COMPLETE] | [REAL] |
| Intelligence | Vector jobs | `/vector-jobs` | [COMPLETE] | [REAL] |
| Intelligence | Semantic search | `/semantic-search` | [COMPLETE] | [MOCKED] — deliberate; no vector store/embedding index exists (`rust/crates/lakehouse-store/src/knowledge.rs`) |
| Intelligence | Agent workflows | `/agents/workflows` | [PARTIAL] | [REAL] (definitions/runs/approvals persisted in Postgres; there is no agent/tool *execution* runtime) |
| Intelligence | Digital employees | `/agents/employees` | [COMPLETE] | [REAL] |
| Intelligence | Approvals inbox | `/agents/approvals` | [COMPLETE] | [REAL] |
| Intelligence | Tool registry | `/agents/tools` | [COMPLETE] | [REAL] |
| Governance | Policies | `/governance/policies` | [PARTIAL] | [REAL] |
| Governance | Classification & masking | `/governance/classification` | [COMPLETE] | [REAL] |
| Governance | Data quality | `/governance/data-quality` | [COMPLETE] | [REAL] |
| Governance | Lineage | `/lineage` | [PARTIAL] | [REAL] |
| Governance | Audit | `/audit` | [PARTIAL] | [REAL] (Dagster run history) |
| Governance | Residency | `/residency` | [COMPLETE] | [REAL] |
| Governance | Bronze maintenance (`expire_snapshots` dry-run/applied; P6 addition) | `/governance/data-quality` (Maintenance tab) | [PARTIAL] | [REAL] — `GET /api/governance/maintenance`, reading `lake.bronze_meta.maintenance_run` |
| Governance | CDC replication slot health (P6 addition) | `/governance/data-quality` (Ingestion tab) | [PARTIAL] | [REAL] — `GET /api/governance/replication`, reading `lake.bronze_meta.replication_slot` |
| Operations | Workloads | `/workloads` | [COMPLETE] | [REAL] |
| Operations | Observability | `/observability` | [PARTIAL] | [REAL] |
| Operations | Services | `/services` | [COMPLETE] | [REAL] |
| Operations | Usage & budgets | `/usage` | [COMPLETE] | [REAL] |
| Admin | Users / roles / tenants / service identities | `/admin/*` | [COMPLETE] | [REAL] |
| Admin | Workspace settings | `/settings` | [COMPLETE] | [MOCKED] — `getWorkspaceSettings` returns a fixed response; there is no setter and nothing is persisted |

## Intentionally missing / still mocked

| Item | Status |
|---|---|
| Real streaming engine (Kafka/Flink/etc.) | [MISSING] (backend) — CDC via Debezium exists but is not a streaming engine. The mocked streaming UI surface was removed (not kept as a mock) rather than fabricate lag/throughput numbers for an engine that doesn't exist. |
| Vector store / embedding-backed semantic search | [MISSING] (backend) |
| Live connector health checks (`testConnection` dials nothing) | [MISSING] (backend) |
| Dedicated connector detail route | [PARTIAL] (drawer sufficient for preview) |
| Dedicated pipeline run route | [PARTIAL] (drawer + actions) |
| Workflow detail canvas route | [PARTIAL] |
| Agent/tool execution runtime | [MISSING] |
| Observability log/trace explorer | [MISSING] |
| Lakekeeper authorization (R1) — runs `allow-all`, not enforced | [MISSING] — see `docs/plans/P5-REPORT.md` |
| Cold/AI storage tiers (always report 0 bytes) | [MISSING] — see README "Status / Known limitations" |

## Cross-domain connections (must stay linked)

| From | To | Status |
|---|---|---|
| Connector | Pipeline | [COMPLETE] |
| Pipeline / Run | Dataset / Lineage / Audit | [COMPLETE] |
| Dataset | Policies / Lineage / Query / Storage | [COMPLETE] |
| Knowledge | Vector job / Semantic search / Catalog | [COMPLETE] |
| Query result / history | Audit | [COMPLETE] |
| Agent run | Approval / Audit | [COMPLETE] |
| Storage op | Catalog asset | [COMPLETE] |
