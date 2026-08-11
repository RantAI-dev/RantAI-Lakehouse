import type {
  ActorKind,
  AuditOutcome,
  CheckStatus,
  Classification,
  EngineCategory,
  EntityStatus,
  Severity,
} from "@/lib/status"

export type Policy = {
  id: string
  name: string
  status: EntityStatus
  kind: string
  subjects: string
  resources: string
  effect: string
  version: number
  owner: string
  updatedAt: string
}

export type ClassificationRule = {
  id: string
  asset: string
  column?: string
  classification: Classification
  confidence: number
  reviewStatus: "auto" | "reviewed" | "needs-review"
  maskingRule?: string
}

export type QualityRule = {
  id: string
  name: string
  asset: string
  dimension: string
  threshold: string
  severity: Severity
  lastStatus: CheckStatus
  lastRunAt: string
}

export type LineageEdge = {
  id: string
  from: string
  to: string
  kind: "pipeline" | "query" | "agent" | "transform"
}

export type LineageGraph = {
  focus: string
  nodes: { id: string; label: string; kind: string }[]
  edges: LineageEdge[]
  columnMappings: { source: string; target: string; transform: string }[]
}

export type AuditEvent = {
  id: string
  at: string
  actor: string
  actorKind: ActorKind
  delegatedActor?: string
  tenant: string
  action: string
  resource: string
  outcome: AuditOutcome
  policyDecision: string
  obligations: string[]
  engineCategory?: EngineCategory
  estimatedCost?: number
  actualCost?: number
  approvalId?: string
}

export type ResidencyRule = {
  id: string
  tenant: string
  classification: Classification
  approvedSites: string[]
  crossSiteAllowed: boolean
  allowedOutput: string
  violations7d: number
}

export interface GovernanceService {
  listPolicies(signal?: AbortSignal): Promise<Policy[]>
  listClassifications(signal?: AbortSignal): Promise<ClassificationRule[]>
  listQuality(signal?: AbortSignal): Promise<QualityRule[]>
  getLineage(focusId: string, signal?: AbortSignal): Promise<LineageGraph>
  listAudit(signal?: AbortSignal): Promise<AuditEvent[]>
  listResidency(signal?: AbortSignal): Promise<ResidencyRule[]>
}
