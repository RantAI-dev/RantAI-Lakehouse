import type { Measured } from "@/lib/measured"
import type { EngineCategory, EntityStatus, WorkloadClass } from "@/lib/status"

/**
 * The query-execution engine, chosen by the user in Query Studio (WS2 §4).
 * Not to be confused with `EngineCategory`, which describes a ClickHouse
 * storage tier (hot/warm/cold) rather than which execution engine ran the
 * query.
 */
export type QueryEngine = "clickhouse" | "trino"

export type SavedQuery = {
  id: string
  title: string
  sql: string
  owner: string
  updatedAt: string
  tags: string[]
}

export type QueryHistoryItem = {
  id: string
  sql: string
  user: string
  at: string
  status: Extract<EntityStatus, "completed" | "failed" | "cancelled" | "blocked">
  durationMs: number
  scannedBytes: number
  costUnits: number
  workloadClass: WorkloadClass
  /**
   * The API now records the real execution engine (`"clickhouse"` /
   * `"trino"`) here; old rows written before that change still carry
   * `"hot-store"`, an `EngineCategory` value, hence the union — and
   * `| string` since either side is free to widen without breaking this
   * contract.
   */
  engine: EngineCategory | QueryEngine | string
  cacheAssisted: boolean
  auditEventId?: string
}

/** Simple federated / multi-source execution plan stage for UI. */
export type QueryPlanStage = {
  id: string
  label: string
  location: string
  operation: string
  estimatedBytes?: number
  status?: Extract<EntityStatus, "completed" | "running" | "failed" | "blocked">
}

export type QueryEstimate = {
  estimatedBytes: number
  estimatedCostMin: number
  estimatedCostMax: number
  workloadClass: WorkloadClass
  engine: EngineCategory
  /**
   * `EXPLAIN ESTIMATE` doesn't report cache eligibility — null until a
   * real cache-eligibility check exists.
   */
  cacheEligible: boolean | null
  /** No freshness-lag measurement exists yet. */
  freshnessLagSeconds: Measured
  /** No policy engine exists yet — WS7 builds one. */
  policyObligations: string[] | null
  sources: string[]
  /**
   * WS1 task 1.6: the old plan marked stages that had not run as
   * `status: "completed"`. Null until a real plan exists.
   */
  plan: QueryPlanStage[] | null
}

export type QueryResult = {
  id: string
  columns: string[]
  rows: Record<string, string>[]
  /**
   * The engine this specific run actually executed against — distinct from
   * `metrics.engine` below, which is an `EngineCategory` describing the
   * ClickHouse storage tier the query hit, not which engine ran it.
   */
  engine: QueryEngine | string
  metrics: {
    durationMs: number
    scannedBytes: number
    costUnits: number
    engine: EngineCategory
    workloadClass: WorkloadClass
    /** ClickHouse's response carries no cache-hit flag — not measured. */
    cacheHit: boolean | null
    /** No pushdown computation exists — not measured. */
    pushdowns: string[] | null
    /** No policy engine exists yet — WS7 builds one. */
    policyObligations: string[] | null
  }
  /**
   * WS1 task 1.6: the old plan was a single hardcoded stage claiming
   * "scan + aggregate" for every query. Null until a real plan exists.
   */
  plan: QueryPlanStage[] | null
  auditEventId?: string
}

/**
 * `GET /api/query/scheduling` — a static capability probe. Scheduled
 * execution of a saved query needs a safe per-principal authority model
 * (a scheduled job has no live session to run "as," and a shared service
 * identity would bypass the author's own permissions) that does not exist
 * before WS7's policy-obligations engine, so this is always `supported:
 * false` today — never a real schedule, never a stored `schedule_cron`
 * (WS2 §13, WS2 plan review W10, round 2, item 1).
 */
export type QuerySchedulingCapability = { supported: false; reason: string }

export interface QueryService {
  listSaved(signal?: AbortSignal): Promise<SavedQuery[]>
  listHistory(signal?: AbortSignal): Promise<QueryHistoryItem[]>
  estimate(sql: string, signal?: AbortSignal): Promise<QueryEstimate>
  run(sql: string, options: { engine: QueryEngine }, signal?: AbortSignal): Promise<QueryResult>
  generateSql(question: string, signal?: AbortSignal): Promise<{ sql: string; explanation: string; assumptions: string[] }>
  getSchedulingCapability(signal?: AbortSignal): Promise<QuerySchedulingCapability>
}
