/**
 * Service registry — pages import from here, never from mock modules directly.
 * Swap `mock*` for HTTP/Flight adapters when backends are ready.
 */
import { mockAssetService } from "./mock/assets"
import { clickhouseQueryService } from "./clients/queries"
import { clickhouseAssetService } from "./clients/assets"
import { dagsterPipelineService } from "./clients/pipelines"
import { clickhouseStorageService } from "./clients/storage"
import { clickhouseOpsService } from "./clients/ops"
import { clickhouseOverviewService } from "./clients/overview"
import { clickhouseGovernanceService } from "./clients/governance"
import { postgresIdentityService } from "./clients/identity"
import { postgresConnectorService } from "./clients/connectors"
import { postgresKnowledgeService } from "./clients/knowledge"
import { postgresAgentService } from "./clients/agents"

// Overview kini NYATA sepenuhnya — summary/activity dari ClickHouse+Dagster,
// alerts (list/ack/resolve) dari Postgres (Task 2.6). mock/overview.ts
// sudah dihapus.
export const overviewService = clickhouseOverviewService
export const assetService = clickhouseAssetService
void mockAssetService
// Pipelines kini NYATA sepenuhnya — list/get/runs/trigger dari Dagster,
// create/generate dari Postgres + LLM, cancel/retry/pause/resume adalah
// mutation Dagster nyata. mock/pipelines.ts sudah dihapus.
export const pipelineService = dagsterPipelineService
// Query Studio kini NYATA sepenuhnya — eksekusi SQL, saved/history/
// collaboration, dan generateSql semua lewat backend Rust (ClickHouse +
// Postgres + LLM). mock/queries.ts sudah dihapus.
export const queryService = clickhouseQueryService
// Knowledge SEBAGIAN NYATA (Task 2.8) — sources/vector jobs (list+create)
// dari Postgres. `search` TETAP mendelegasikan ke mock: tidak ada vector
// database/mesin embedding/index pencarian di mana pun (lihat
// `rust/crates/lakehouse-store/src/knowledge.rs`). mock/knowledge.ts TIDAK
// dihapus karena `search` masih membutuhkannya.
export const knowledgeService = postgresKnowledgeService
// Agents kini NYATA sepenuhnya — employees/tools/workflows/runs/approvals
// di Postgres (Task 2.9). Tidak ada runtime eksekusi agent/tool (kontrak
// memang tidak memintanya). mock/agents.ts sudah dihapus.
export const agentService = postgresAgentService
// Governance kini NYATA sepenuhnya — reads dari ClickHouse/Dagster,
// policies + create*Rule dari Postgres. mock/governance.ts sudah dihapus.
export const governanceService = clickhouseGovernanceService
// Ops kini NYATA sepenuhnya — observability/usage/workloads/services dari
// ClickHouse+Dagster, cancelWorkload adalah KILL QUERY nyata. mock/ops.ts
// sudah dihapus.
export const opsService = clickhouseOpsService
// Identity kini NYATA — pengguna/peran/tenant/service identity di Postgres.
// Seluruh method kontrak terlayani, jadi mock/identity.ts sudah dihapus.
export const identityService = postgresIdentityService
// Connectors kini NYATA — definisi konektor (CRUD + testConnection) di
// Postgres (Task 2.7). Kredensial TIDAK PERNAH disimpan/dikembalikan: hanya
// `secretRef` (referensi, mis. "env:FOO"); lihat
// `rust/crates/lakehouse-store/src/connectors.rs` untuk catatan keputusan.
// mock/connectors.ts sudah dihapus.
export const connectorService = postgresConnectorService
// Storage kini NYATA sepenuhnya — overview dari ClickHouse/Iceberg,
// policies/operations/restore dari Postgres (Task 2.6). mock/storage.ts
// sudah dihapus.
export const storageService = clickhouseStorageService
