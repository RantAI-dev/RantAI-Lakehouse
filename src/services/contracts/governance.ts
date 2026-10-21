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

/** The kind of finding a quality incident records (`DATA-14`). */
export type QualityIncidentKind = "check" | "volume_anomaly" | "freshness_anomaly" | string

/**
 * One quality incident (`GET /api/governance/quality/incidents`): a failed
 * run of an authored rule, or a learned volume or freshness anomaly. It stays
 * `open` until a person resolves it with a reason.
 */
export type QualityIncident = {
  id: string
  /** The table, lower-cased (`silver.orders`). */
  asset: string
  /** The rule that failed; `null` for an anomaly. */
  ruleId: string | null
  /** The rule's name while it still exists. */
  ruleName: string | null
  kind: QualityIncidentKind
  openedAt: string
  /** When a run or a pass last found it still broken. */
  lastSeenAt: string
  detail: string
  // PR #106 review R1 SHOULD-FIX DATA-14 3 / D2
  status: "open" | "stale" | "resolved" | string
  resolvedAt: string | null
  resolvedBy: string | null
  resolution: string | null
}

/** Which incidents to list; open by default. */
export type QualityIncidentFilter = { status?: "open" | "resolved" | "all"; asset?: string }

/** When a table's rules run (`PUT /api/governance/quality/schedules/{asset}`). */
export type QualitySchedule = {
  asset: string
  mode: "on_load" | "interval" | string
  /** Minutes between runs; the backstop for `on_load`. 60 to 1440. */
  intervalMinutes: number
  updatedBy: string | null
  updatedAt: string
}

export type QualitySchedules = {
  schedules: QualitySchedule[]
  /** The interval of a table with no schedule. */
  defaultIntervalMinutes: number
}

export type SetQualityScheduleInput =
  | { mode: "interval"; intervalMinutes: number }
  | { mode: "on_load"; intervalMinutes?: number }

/**
 * Whether a table's anomalies are being raised yet
 * (`GET /api/governance/quality/learning/{asset}`). `message` is the
 * sentence to show while it is still learning, `null` once it has learned.
 */
export type QualityLearning = {
  asset: string
  learned: boolean
  days: number
  loads: number
  requiredDays: number
  requiredLoads: number
  message: string | null
  firstLoadAt: string | null
  lastLoadAt: string | null
}

/** One recorded run of a rule (`GET /api/governance/quality/{id}/runs`). */
export type QualityRunRecord = {
  status: CheckStatus
  value: string
  /** A person's display name, `schedule` or `load`. */
  runBy: string
  at: string
  /** What the rule said when it ran; `null` for a run from before it was kept. */
  threshold: string | null
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
  /** How the connection was recorded: ingest spec, catalog registry, authored pipeline, view definition, Gold export or dataset publisher. */
  kind: "ingest" | "catalog" | "pipeline" | "view" | "export" | "publisher" | string
  /** The platform record this edge comes from; lineage draws nothing it cannot cite. */
  evidence?: string
}

/** One end of a column mapping; `null` when the statement did not let us name it. */
export type ColumnEnd = { table: string | null; column: string | null }

/**
 * One recorded step from a source column to the column built from it
 * (`DATA-13`). `partial` is true when the statement could not be fully
 * traced; `reason` then says why, and `from` may be empty.
 */
export type ColumnMapping = {
  from: ColumnEnd
  to: { table: string; column: string }
  /** `copy` (a bare or renamed column), `cast`, `expression` or `unknown`. */
  transform: "copy" | "cast" | "expression" | "unknown" | string
  partial: boolean
  reason: string | null
  /** What recorded it: an authored pipeline run, a Silver/Gold build from the query log, or a saved dashboard SQL source. */
  producer: { kind: "pipeline" | "build" | "sql_source" | string; ref: string }
  recordedAt: string
  /** Distance from the column asked about; 1 is directly adjacent. */
  hop: number
}

/** A saved chart that reads a column downstream of the one asked about. */
export type ColumnChart = {
  chartId: string
  chartTitle: string
  dashboardId: string
  dashboardTitle: string
}

/** `GET /api/governance/lineage/columns`. */
export type ColumnLineage = {
  supported: boolean
  table: string
  column: string
  direction: "up" | "down" | string
  mappings: ColumnMapping[]
  /** How long column lineage is kept, and how far back this answer's records go. */
  retention: { days: number; oldestRecordedAt: string | null }
  /** What this answer leaves out and why (tenant and catalog rules). */
  coverage?: string[]
  /** Downstream only, for a caller with `dashboard:read`. */
  charts?: ColumnChart[]
  /** Downstream only: the caller may not list dashboards. */
  chartsHidden?: boolean
  /** Downstream only: the charts could not be read just now. */
  chartsUnavailable?: boolean
}

export type LineageGraph = {
  focus: string
  nodes: { id: string; label: string; kind: string }[]
  edges: LineageEdge[]
  /** The recorded column mappings that touch the focus table. */
  columnMappings: ColumnMapping[]
  /** False when the build has no lineage capture; `reason` says why. */
  supported: boolean
  reason?: string
  /** Set when there is no graph to show: no focus, or nothing recorded mentions it. */
  note?: string
  /** What this graph can and cannot contain (e.g. tables loaded outside the platform have no incoming edge). */
  coverage?: string[]
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
  /**
   * The columns a column comes from (`up`) or the columns and saved charts
   * built from it (`down`); needs `lineage:read`, and charts need
   * `dashboard:read`.
   */
  getColumnLineage(
    table: string,
    column: string,
    direction: "up" | "down",
    signal?: AbortSignal
  ): Promise<ColumnLineage>
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
   * or threshold leaves it without a result until it runs again; the runs it
   * had stay in its history.
   */
  updateQualityRule(
    id: string,
    input: UpdateQualityRuleInput,
    signal?: AbortSignal
  ): Promise<QualityRule>
  /**
   * Remove an authored rule and its recorded runs; needs `governance:write`.
   * Its open incidents are resolved as "rule deleted".
   */
  deleteQualityRule(id: string, signal?: AbortSignal): Promise<void>
  /** The last 50 runs of a rule, newest first, with the threshold each ran with. */
  listQualityRuns(id: string, signal?: AbortSignal): Promise<QualityRunRecord[]>
  /** Quality incidents, newest first (open by default). */
  listQualityIncidents(
    filter?: QualityIncidentFilter,
    signal?: AbortSignal
  ): Promise<QualityIncident[]>
  /** Resolve an open incident with a reason; needs `governance:write`. */
  resolveQualityIncident(
    id: string,
    resolution: string,
    signal?: AbortSignal
  ): Promise<QualityIncident>
  /** Every table's schedule; a table not listed runs every default interval. */
  listQualitySchedules(signal?: AbortSignal): Promise<QualitySchedules>
  /** Run a table's rules on every load, or every N minutes; needs `governance:write`. */
  setQualitySchedule(
    asset: string,
    input: SetQualityScheduleInput,
    signal?: AbortSignal
  ): Promise<QualitySchedule>
  /** Whether a table's volume and freshness anomalies are being raised yet. */
  getQualityLearning(asset: string, signal?: AbortSignal): Promise<QualityLearning>
  createClassificationRule(
    input: CreateClassificationRuleInput,
    signal?: AbortSignal
  ): Promise<ClassificationRule>
  /** Every authored dataset freshness SLA — `GET /api/governance/sla`. */
  listDatasetSla(signal?: AbortSignal): Promise<DatasetSla[]>
  /** Author or replace one table's freshness SLA — `PUT /api/governance/sla`. */
  putDatasetSla(input: DatasetSla, signal?: AbortSignal): Promise<DatasetSla>
  /** Remove a table's freshness SLA; needs `governance:write`. */
  deleteDatasetSla(tableName: string, signal?: AbortSignal): Promise<void>
  /**
   * Remove a classification rule; needs `governance:write`. An older rule
   * for the same asset or column applies again, or the default level.
   */
  deleteClassificationRule(id: string, signal?: AbortSignal): Promise<void>
}
