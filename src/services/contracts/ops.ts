import type { EngineCategory, Health, WorkloadClass, WorkloadStatus } from "@/lib/status"
import type { Measured } from "@/lib/measured"

export type WorkloadItem = {
  id: string
  principal: string
  tenant: string
  // No workload classifier exists yet (WS1 honesty pass); null replaces
  // the invented "hot-analytics" literal every row used to carry.
  class: WorkloadClass | null
  engine: EngineCategory
  status: WorkloadStatus
  elapsedMs: number
  // No cost model exists yet; null replaces the literal `1`.
  estimatedCost: Measured
  queueReason?: string
  // Derived server-side from `elapsed` in the same ClickHouse query, not
  // this process's clock; null when that derivation fails to parse.
  startedAt: string | null
}

export type ObservabilitySummary = {
  queryP95Ms: number
  queryErrorRate: number
  // Nothing measures these yet (WS1 honesty pass); null replaces the
  // literal zeros. `streamingLagSeconds` was removed outright — it had no
  // consumer and nothing measures streaming either.
  ingestLagSeconds: Measured
  cacheHitRate: Measured
  policyDecisionP95Ms: Measured
  agentSuccessRate: Measured
  activeIncidents: Measured
  slos: { name: string; target: string; current: string; ok: boolean }[]
}

export type PlatformService = {
  id: string
  name: string
  health: Health
  // `true` only for ClickHouse and Dagster, the two services actually
  // probed this request; Iceberg/Lakekeeper and RustFS are never probed
  // (WS5 is expected to add real probes for them).
  checked: boolean
  // Nothing measures version, replica count, error rate, or latency for
  // any service today; null replaces the "-"/1/0/0 literals.
  version: string | null
  site: string
  replicas: Measured
  errorRate: Measured
  latencyMs: Measured
  dependencies: string[]
}

export interface OpsService {
  listWorkloads(signal?: AbortSignal): Promise<WorkloadItem[]>
  cancelWorkload(id: string, signal?: AbortSignal): Promise<WorkloadItem>
  getObservability(signal?: AbortSignal): Promise<ObservabilitySummary>
  listServices(signal?: AbortSignal): Promise<PlatformService[]>
}
