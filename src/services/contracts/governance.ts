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
  // Raw, still-unparsed `conditions` JSON string (WS7 item A2). Absent for
  // a legacy policy authored before conditions were structured. A legacy
  // value may be free-text prose rather than JSON — never assume it parses
  // and never crash the policy list page on a `JSON.parse` failure.
  conditions?: string
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
  // `null` until the rule is run (`POST /api/governance/quality/{id}/run`)
  // — never a placeholder verdict.
  lastStatus: CheckStatus | null
  // `null` for the same reason as `lastStatus`.
  lastRunAt: string | null
  /** What the latest run measured, e.g. "97.2% not null". Authored rules only. */
  lastValue?: string | null
  /**
   * Whether the rule's threshold is written in the form the evaluator
   * reads; `hint` says how to write one when it is not. Absent on a
   * verdict a quality job recorded, which is not run from here.
   */
  evaluable?: boolean
  hint?: string | null
}

/** What one run of a rule found (`routes::quality::run_rule`). */
export type QualityRunResult = {
  id: string
  status: CheckStatus
  value: string
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
  /** False when the build has no lineage capture; `reason` says why. */
  supported: boolean
  reason?: string
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

export type CreatePolicyInput = {
  name: string
  kind: string
  subjects: string
  resources: string
  effect: string
  conditions?: string
  activate?: boolean
  owner?: string
}

export type CreateQualityRuleInput = {
  name: string
  asset: string
  dimension: string
  threshold: string
  severity: Severity
}

/** What rewriting a rule may change; its name stays. */
export type UpdateQualityRuleInput = {
  asset: string
  threshold: string
  severity: Severity
}

export type CreateClassificationRuleInput = {
  asset: string
  column?: string
  classification: Classification
  maskingRule?: string
}

/**
 * One row of `GET /api/governance/maintenance` — a single Dagster
 * `bronze_maintenance_job` run against one Bronze Iceberg table. Only
 * `expire_snapshots` is a real, working verb on ClickHouse 26.3
 * (`remove_orphan_files` does not exist for Iceberg tables; `OPTIMIZE`
 * parses but fails at runtime with an HTTP 403) — see
 * `docs/plans/G3-RESULT.md` and ADR 0009. `skippedVerbs` names the verbs
 * that were not attempted, so the UI never implies a compaction ran when it
 * did not.
 */
export type MaintenanceRun = {
  tableName: string
  runAt: string
  dryRun: { deletedDataFiles: string; deletedManifestFiles: string }
  applied: { deletedDataFiles: string; deletedManifestFiles: string }
  skippedVerbs: string
}

/**
 * One row of `GET /api/governance/replication` — a point-in-time snapshot
 * of one Postgres logical-replication slot backing a CDC connector
 * (Debezium Server, P5). `status` is `"ok" | "warning" | "critical"`,
 * computed in `dagster/dispar_orchestrate/replication_metrics.py` from WAL
 * retention thresholds and whether the slot is still `active` — an
 * inactive slot is flagged `critical` regardless of byte thresholds,
 * because a disconnected consumer still pins WAL indefinitely (R5).
 */
export type ReplicationSlot = {
  connectorId: string
  slotName: string
  checkedAt: string
  active: boolean
  walRetainedBytes: string
  confirmedFlushLagBytes: string
  status: "ok" | "warning" | "critical" | string
}

/**
 * A per-table freshness expectation, authored by an operator and read by
 * the Overview "Freshness" strip (WS5 item E2) and `lakehouse-alerts`'s
 * `Freshness` rule kind (item C1). `GET`/`PUT /api/governance/sla`.
 */
export type DatasetSla = {
  tableName: string
  expectedIntervalMinutes: number
  owner?: string
}

export interface GovernanceService {
  listPolicies(signal?: AbortSignal): Promise<Policy[]>
  listClassifications(signal?: AbortSignal): Promise<ClassificationRule[]>
  listQuality(signal?: AbortSignal): Promise<QualityRule[]>
  getLineage(focusId: string, signal?: AbortSignal): Promise<LineageGraph>
  listAudit(signal?: AbortSignal): Promise<AuditEvent[]>
  /** Bronze Iceberg maintenance runs (P4/P6) — `GET /api/governance/maintenance`. */
  listMaintenanceRuns(signal?: AbortSignal): Promise<MaintenanceRun[]>
  /** CDC replication slot health (P5/P6) — `GET /api/governance/replication`. */
  listReplicationSlots(signal?: AbortSignal): Promise<ReplicationSlot[]>
  createPolicy(input: CreatePolicyInput, signal?: AbortSignal): Promise<Policy>
  /**
   * Enforce a policy (`"ready"`) or stop enforcing it (`"draft"`); needs
   * `policy:write`. It applies from the next query on.
   */
  setPolicyStatus(id: string, status: "ready" | "draft", signal?: AbortSignal): Promise<Policy>
  /** Remove a policy, enforced or not; needs `policy:write`. */
  deletePolicy(id: string, signal?: AbortSignal): Promise<void>
  createQualityRule(
    input: CreateQualityRuleInput,
    signal?: AbortSignal
  ): Promise<QualityRule>
  /** Evaluate one authored rule now; needs `query:read`. */
  runQualityRule(id: string, signal?: AbortSignal): Promise<QualityRunResult>
  /**
   * Rewrite an authored rule; needs `governance:write`. Changing its table
   * or threshold clears the results recorded for it.
   */
  updateQualityRule(
    id: string,
    input: UpdateQualityRuleInput,
    signal?: AbortSignal
  ): Promise<QualityRule>
  /** Remove an authored rule and its recorded runs; needs `governance:write`. */
  deleteQualityRule(id: string, signal?: AbortSignal): Promise<void>
  createClassificationRule(
    input: CreateClassificationRuleInput,
    signal?: AbortSignal
  ): Promise<ClassificationRule>
  /** Every authored dataset freshness SLA — `GET /api/governance/sla`. */
  listDatasetSla(signal?: AbortSignal): Promise<DatasetSla[]>
  /** Author or replace one table's freshness SLA — `PUT /api/governance/sla`. */
  putDatasetSla(input: DatasetSla, signal?: AbortSignal): Promise<DatasetSla>
}
