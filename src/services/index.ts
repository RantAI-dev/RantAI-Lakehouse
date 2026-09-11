/**
 * Service registry — pages import from here, never from mock modules directly.
 * Swap `mock*` for HTTP/Flight adapters when backends are ready.
 */
import { mockAssetService } from "./mock/assets"
import { clickhouseQueryService } from "./clients/queries"
import { clickhouseAssetService } from "./clients/assets"
import { dagsterPipelineService } from "./clients/pipelines"
import { clickhouseOpsService } from "./clients/ops"
import { clickhouseOverviewService } from "./clients/overview"
import { clickhouseGovernanceService } from "./clients/governance"
import { postgresIdentityService } from "./clients/identity"
import { postgresConnectorService } from "./clients/connectors"
import { postgresAgentService } from "./clients/agents"
import { clickhouseAlertRuleService } from "./clients/alerts"
import { icebergLakehouseService } from "./clients/lakehouse"

// Overview is now fully real — summary/activity from ClickHouse+Dagster,
// alerts (list/ack/resolve) from Postgres (Task 2.6). mock/overview.ts
// has been deleted.
export const overviewService = clickhouseOverviewService
export const assetService = clickhouseAssetService
void mockAssetService
// Pipelines is now fully real — list/get/runs/trigger from Dagster,
// create/generate from Postgres + LLM, cancel/retry/pause/resume are
// real Dagster mutations. mock/pipelines.ts has been deleted.
export const pipelineService = dagsterPipelineService
// Query Studio is now fully real — SQL execution, saved/history, and
// generateSql all go through the Rust backend (ClickHouse + Postgres + LLM).
// mock/queries.ts has been deleted.
export const queryService = clickhouseQueryService
// Agents is now fully real — employees/tools/workflows/runs/approvals
// live in Postgres (Task 2.9). There is no agent/tool execution runtime
// (the contract never asked for one). mock/agents.ts has been deleted.
export const agentService = postgresAgentService
// Governance is now fully real — reads from ClickHouse/Dagster,
// policies + create*Rule from Postgres. mock/governance.ts has been deleted.
export const governanceService = clickhouseGovernanceService
// Ops is now fully real — observability/usage/workloads/services from
// ClickHouse+Dagster, cancelWorkload is a real KILL QUERY. mock/ops.ts
// has been deleted.
export const opsService = clickhouseOpsService
// Identity is now real — users/roles/tenants/service identities live in
// Postgres. Every contract method is served, so mock/identity.ts has been
// deleted.
export const identityService = postgresIdentityService
// Connectors is now real — connector definitions (CRUD + testConnection) live
// in Postgres (Task 2.7). Credentials are NEVER stored or returned: only the
// `secretRef` (a reference, e.g. "env:FOO"); see
// `rust/crates/lakehouse-store/src/connectors.rs` for the design rationale.
// mock/connectors.ts has been deleted.
export const connectorService = postgresConnectorService
// Alert rules (WS1 task 1.15) — CRUD + run over `console.alert_rule` in
// ClickHouse, ported by `lakehouse_alerts`. No mock ever existed for this
// service; the feature previously fetched `/api/alerts` directly with no
// `res.ok` check.
export const alertRuleService = clickhouseAlertRuleService
// Lakehouse (WS2 §4) — the read-only Iceberg warehouse/namespace/table
// surface over `/api/lakehouse/*`. No mock ever existed for this service.
export const lakehouseService = icebergLakehouseService
