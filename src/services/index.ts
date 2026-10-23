/**
 * Service registry — pages import from here, never from mock modules directly.
 * Swap `mock*` for HTTP/Flight adapters when backends are ready.
 */
import { clickhouseQueryService } from "./clients/queries"
import { clickhouseAssetService } from "./clients/assets"
import { dagsterPipelineService } from "./clients/pipelines"
import { clickhouseOpsService } from "./clients/ops"
import { clickhouseOverviewService } from "./clients/overview"
import { clickhouseGovernanceService } from "./clients/governance"
import { postgresIdentityService } from "./clients/identity"
import { postgresConnectorService } from "./clients/connectors"
import { clickhouseDashboardService } from "./clients/dashboards"
import { calcFieldService as calcFieldClientService } from "./clients/calc-fields"
import { metricService as metricClientService } from "./clients/metrics"
import { hierarchyService as hierarchyClientService } from "./clients/hierarchies"
import { paramService as paramClientService } from "./clients/params"
import { postgresAgentService } from "./clients/agents"
import { clickhouseAlertRuleService } from "./clients/alerts"
import { icebergLakehouseService } from "./clients/lakehouse"
import { goldService as goldClientService } from "./clients/gold"
import { notificationsService as notificationsClientService } from "./clients/notifications"
import { uploadService as uploadClientService } from "./clients/uploads"
import { homeService as homeClientService } from "./clients/home"
import { reportService as reportClientService } from "./clients/reports"
import { settingsService as settingsClientService } from "./clients/settings"
import { chatTermService as chatTermClientService } from "./clients/chat-terms"
import { semanticService as semanticClientService } from "./clients/semantic"
import { tagKeysService as tagKeysClientService } from "./clients/tag-keys"
import * as authClient from "./clients/auth"

// Overview is now fully real — summary/activity from ClickHouse+Dagster,
// alerts (list/ack/resolve) from Postgres (Task 2.6). mock/overview.ts
// has been deleted.
export const overviewService = clickhouseOverviewService
export const assetService = clickhouseAssetService
// Pipelines is now fully real — list/get/runs/trigger from Dagster,
// create/generate from Postgres + LLM, cancel/retry/pause/resume are
// real Dagster mutations. mock/pipelines.ts has been deleted.
export const pipelineService = dagsterPipelineService
// Query Studio is now fully real — SQL execution, saved/history, and
// askQuestion/generateSql (the chat's engine, `/api/ai/chat`) all go through
// the Rust backend (ClickHouse + Postgres + LLM).
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
// Dashboards: boards live in ClickHouse (`console.bi_board`), SQL sources
// and folders over `/api/dashboard/sources` and `/api/dashboard/folders`.
// Only list-level board operations go through here; the dashboard canvas
// itself still calls `/api/dashboard/*` directly for charts/layout/filters.
// No mock ever existed for this service.
export const dashboardService = clickhouseDashboardService
/**
 * The dashboard types the console's features use. They live in the contract
 * (`services/contracts/dashboards`), not in a client, and are exported here so
 * a feature imports them from `@/services` and not from a client
 * (`PR #199 review M SHOULD-FIX BI-16 7`).
 */
export type { Board, BoardTab, LayoutMap, PhoneTile, TileBox } from "./contracts/dashboards"
/** Calculated fields of a mart or SQL source (BI-8). */
export const calcFieldService = calcFieldClientService
/** Reusable metrics and segments (BI-2). */
export const metricService = metricClientService
export const hierarchyService = hierarchyClientService
export const paramService = paramClientService
// Alert rules (WS1 task 1.15) — CRUD + run over `console.alert_rule` in
// ClickHouse, ported by `lakehouse_alerts`. No mock ever existed for this
// service; the feature previously fetched `/api/alerts` directly with no
// `res.ok` check.
export const alertRuleService = clickhouseAlertRuleService
// Lakehouse (WS2 §4) — the read-only Iceberg warehouse/namespace/table
// surface over `/api/lakehouse/*`. No mock ever existed for this service.
export const lakehouseService = icebergLakehouseService
// Gold export page (WS6) — marts/exports/history/consumers all real, via
// lakehouse-api's /api/gold/* routes. No mock ever existed for this
// domain (ADR 0010 was Rust-only from the start).
export const goldService = goldClientService
// Navbar bell (WS5 item F1) — real open alerts + pending approvals from
// Postgres, honest `supported: false` when no pool is configured. No mock
// ever existed for this domain.
export const notificationsService = notificationsClientService
// Uploaded files (DATA-9, ADR 0014) — a delimited text file is stored in the
// warehouse bucket, previewed, and loaded into a raw table by
// `file_ingest_job`; rows live in Postgres, and the outcome of a load is read
// back from `bronze_meta.ingest_run` by the API. Real from the start: no mock
// ever existed for this domain.
export const uploadService = uploadClientService
// Home's per-user layout (cards and shortcuts, in order) over
// `/api/home/layout`; honest `supported: false` when no Postgres pool is
// configured. No mock ever existed for this domain.
export const homeService = homeClientService
// The deployment's report time zone and first day of the week over
// `/api/settings/reporting` (BI-9).
export const settingsService = settingsClientService
// Scheduled reports by email (RPT-1) over `/api/reports/...`. No mock ever existed.
export const reportService = reportClientService
/**
 * The reporting settings a caller sends (the report time zone, the first day
 * of the week, the number locale, the company palette, the fonts and the
 * content sites). The contract owns the shape, and it is exported here so a
 * feature imports it from `@/services` (`PR #199 review M SHOULD-FIX BI-17 7`).
 */
export type { ReportingSettingsInput } from "./contracts/settings"
// The words a user defined for the chat (`PUT /api/ai/terms`), stored when
// they pick an answer to a question the chat asked. No mock ever existed.
export const chatTermService = chatTermClientService
// The semantic entries of a table and its columns (`/api/semantic/{asset}`):
// the drafted meaning and role, and the person's confirmed corrections. No
// mock ever existed.
export const semanticService = semanticClientService
// The governed tag keys and their allowed values (`/api/governance/tag-keys`,
// `DATA-12` part 2). Free tags are not part of this.
export const tagKeysService = tagKeysClientService
// SSO admin page reads the live OIDC configuration
// off the API process. Sessions page lists and revokes
// live browser sessions via `GET /api/auth/sessions` and
// `DELETE /api/auth/sessions/{id}`. Only `providers`,
// `listSessions`, and `deleteSession` are registered — the other auth
// methods (login/logout/me/change-password) stay imported directly by
// `AuthProvider` and the login/change-password pages; see
// `services/clients/auth.ts`'s module doc comment for why the split.
export const authService = {
  providers: authClient.providers,
  listSessions: authClient.listSessions,
  deleteSession: authClient.deleteSession,
}
