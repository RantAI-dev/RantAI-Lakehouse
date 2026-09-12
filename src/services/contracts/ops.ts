import type {
  EngineCategory,
  Health,
  StorageTier,
  WorkloadClass,
  WorkloadStatus,
} from "@/lib/status"

export type WorkloadItem = {
  id: string
  principal: string
  tenant: string
  class: WorkloadClass
  engine: EngineCategory
  status: WorkloadStatus
  elapsedMs: number
  estimatedCost: number
  queueReason?: string
  startedAt: string
}

export type ObservabilitySummary = {
  queryP95Ms: number
  queryErrorRate: number
  ingestLagSeconds: number
  cacheHitRate: number
  policyDecisionP95Ms: number
  agentSuccessRate: number
  activeIncidents: number
  slos: { name: string; target: string; current: string; ok: boolean }[]
}

export type PlatformService = {
  id: string
  name: string
  health: Health
  version: string
  site: string
  replicas: number
  errorRate: number
  latencyMs: number
  dependencies: string[]
}

export type UsageSummary = {
  computeUnits7d: number
  scannedBytes7d: number
  storageByTier: Record<StorageTier, number>
  pipelineRuns7d: number
  agentBudgetUsedRate: number
  tenants: {
    id: string
    name: string
    computeUnits: number
    budgetLimit: number
    budgetSpent: number
  }[]
}

export interface OpsService {
  listWorkloads(signal?: AbortSignal): Promise<WorkloadItem[]>
  cancelWorkload(id: string, signal?: AbortSignal): Promise<WorkloadItem>
  getObservability(signal?: AbortSignal): Promise<ObservabilitySummary>
  listServices(signal?: AbortSignal): Promise<PlatformService[]>
  getUsage(signal?: AbortSignal): Promise<UsageSummary>
}
