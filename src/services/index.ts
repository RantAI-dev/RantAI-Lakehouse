/**
 * Service registry — pages import from here, never from mock modules directly.
 * Swap `mock*` for HTTP/Flight adapters when backends are ready.
 */
import { mockOverviewService } from "./mock/overview"
import { mockAssetService } from "./mock/assets"
import { mockPipelineService } from "./mock/pipelines"
import { mockStreamingService } from "./mock/streaming"
import { mockQueryService } from "./mock/queries"
import { clickhouseQueryService } from "./clients/queries"
import { clickhouseAssetService } from "./clients/assets"
import { dagsterPipelineService } from "./clients/pipelines"
import { clickhouseStorageService } from "./clients/storage"
import { clickhouseOpsService } from "./clients/ops"
import { clickhouseOverviewService } from "./clients/overview"
import { clickhouseGovernanceService } from "./clients/governance"
import { postgresIdentityService } from "./clients/identity"
import { mockKnowledgeService } from "./mock/knowledge"
import { mockAgentService } from "./mock/agents"
import { mockGovernanceService } from "./mock/governance"
import { mockOpsService } from "./mock/ops"
import { mockConnectorService } from "./mock/connectors"
import { mockStorageService } from "./mock/storage"

export const overviewService = clickhouseOverviewService
void mockOverviewService
export const assetService = clickhouseAssetService
void mockAssetService
export const pipelineService = dagsterPipelineService
void mockPipelineService
export const streamingService = mockStreamingService
// Query Studio kini NYATA — eksekusi SQL di ClickHouse (lakehouse kita).
// Sisanya masih mock sampai client-nya dibuat (fase berikutnya).
export const queryService = clickhouseQueryService
void mockQueryService
export const knowledgeService = mockKnowledgeService
export const agentService = mockAgentService
export const governanceService = clickhouseGovernanceService
void mockGovernanceService
export const opsService = clickhouseOpsService
void mockOpsService
// Identity kini NYATA — pengguna/peran/tenant/service identity di Postgres.
// Seluruh method kontrak terlayani, jadi mock/identity.ts sudah dihapus.
export const identityService = postgresIdentityService
export const connectorService = mockConnectorService
export const storageService = clickhouseStorageService
void mockStorageService
