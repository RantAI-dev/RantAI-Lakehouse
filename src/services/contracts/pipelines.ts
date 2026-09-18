import type { Measured } from "@/lib/measured"
import type { EntityStatus } from "@/lib/status"

/** The kinds a pipeline can be authored as. */
export type PipelineKind = "batch" | "incremental"

export type Pipeline = {
  id: string
  name: string
  /** Widened: rows authored before WS1 may still carry `document` or `vector`, which the database constraint still permits. */
  kind: PipelineKind | string
  status: EntityStatus
  owner: string
  /** Null for a Dagster job: no per-job lineage exists until WS4 reads the op graph. Authored pipelines keep their real user-entered source. */
  source: string | null
  /** Null for a Dagster job: same as `source`. Authored pipelines keep their real user-entered target. */
  target: string | null
  /** Optional link to the ingress connector that feeds this pipeline. */
  connectorId?: string
  /** Catalog asset IDs for cross-navigation (mock → future API). */
  sourceAssetId?: string
  targetAssetId?: string
  schedule: string
  /** Null when the pipeline has never run — an authored draft always, a Dagster job until its first run. */
  lastRunAt: string | null
  nextRunAt?: string
  /** Null: no SLA is defined anywhere yet — WS5 adds `dataset_sla`. */
  slaOk: boolean | null
  /** Null until WS2 derives freshness from Iceberg snapshot timestamps. */
  freshnessLagSeconds: Measured
}

export type PipelineRun = {
  id: string
  pipelineId: string
  status: EntityStatus
  startedAt: string
  endedAt?: string
  /** Row counts are not tracked yet — WS4 reads them from step materializations. */
  processed: Measured
  accepted: Measured
  rejected: Measured
  retried: Measured
  /** Null for a just-launched run and for cancel/retry responses. */
  costUnits: Measured
  error?: string
  checkpoint?: string
  auditEventId?: string
  /** Output dataset produced by this run when known. */
  outputAssetId?: string
}

/** One op node in a job's dependency graph (`GET /api/pipelines/{id}`'s `graph.ops`). */
export type PipelineOpNode = {
  name: string
  description: string | null
  sourceRef: string | null
  commit: string | null
  sql: string | null
}

/** One dependency edge: `from` runs before `to`. */
export type PipelineOpEdge = { from: string; to: string }

/**
 * An authored pipeline's stored definition, exactly the fields
 * `CreatePipelineInput` already sends and `rust/crates/lakehouse-api/src/routes/pipelines.rs`'s
 * `authored_detail` now persists and returns (WS4 item D-series).
 */
export type AuthoredDefinition = {
  sourceZone: string
  sourceTable: string
  incrementalColumn: string | null
  transforms: string[]
  fbicEnabled: boolean
  targetZone: string
  targetTable: string
  connectorId: string | null
}

export type PipelineDetail = Pipeline & {
  /** `"dagster"` for a Dagster-native job, `"authored"` for a Postgres-defined pipeline. */
  engine: "dagster" | "authored"
  /** No free-text description exists for a Dagster job today (WS4 item C1); null for both engines. */
  description: string | null
  /** Null: no graph is knowable — an authored pipeline with no run yet, or a
   * Dagster job Dagster itself could not resolve. Real (populated from
   * `job_graph`) for a Dagster job whose graph resolved. */
  graph: { ops: PipelineOpNode[]; edges: PipelineOpEdge[] } | null
  /** Real per-run config when recorded; `[]` (not null) is a genuine
   * measurement of "no configuration recorded", distinct from `graph: null`'s
   * "not knowable at all". */
  config: { key: string; value: string }[]
  /** Non-null only for an authored (`pl-`) pipeline. */
  definition: AuthoredDefinition | null
  /**
   * NOT part of `GET /api/pipelines/{id}`. `routes::pipelines::detail`
   * (WS4 item C1) returns no `runs` field -- runs have their own route,
   * `GET /api/pipelines/{id}/runs`, reached through `listRuns`. Declaring
   * it here made every detail page crash on `state.data.runs[0]`.
   */
  runs?: PipelineRun[]
}

/** `GET /api/pipelines/{id}/source?op=` response — one op's read-only source text. */
export type PipelineSource = {
  sourceRef: string
  commit: string
  language: "python" | "sql"
  text: string
}

/** One entry of `GET /api/pipelines/{id}/runs/{runId}/steps`. */
export type PipelineRunStep = {
  stepKey: string
  status: EntityStatus
  startMs: number | null
  endMs: number | null
  materializations: { assetKey: string | null; rows: Measured }[]
}

/** One bounded page of `GET /api/pipelines/{id}/runs/{runId}/logs`. */
export type PipelineRunLogsPage = {
  lines: { ts: number; level: string; stepKey: string | null; message: string }[]
  cursor: string
}

export type CreatePipelineInput = {
  name: string
  kind: PipelineKind
  sourceZone: string
  sourceTable: string
  incrementalColumn?: string
  transforms: string[]
  fbicEnabled?: boolean
  targetZone: string
  targetTable: string
  schedule: string
  owner?: string
  /** Optional ingress connector this pipeline reads through (WS4 item F5) — mirrors `Pipeline.connectorId`. */
  connectorId?: string
}

export interface PipelineService {
  listPipelines(signal?: AbortSignal): Promise<Pipeline[]>
  getPipeline(id: string, signal?: AbortSignal): Promise<PipelineDetail>
  listRuns(pipelineId: string, signal?: AbortSignal): Promise<PipelineRun[]>
  createPipeline(input: CreatePipelineInput, signal?: AbortSignal): Promise<Pipeline>
  triggerRun(id: string, signal?: AbortSignal): Promise<PipelineRun>
  cancelRun(runId: string, signal?: AbortSignal): Promise<PipelineRun>
  retryRun(runId: string, signal?: AbortSignal): Promise<PipelineRun>
  pausePipeline(id: string, signal?: AbortSignal): Promise<Pipeline>
  resumePipeline(id: string, signal?: AbortSignal): Promise<Pipeline>
  /** `GET /api/pipelines/{id}/source?op=` — one op's read-only source text (WS4 item F1). */
  getPipelineSource(id: string, op: string, signal?: AbortSignal): Promise<PipelineSource>
  /** `GET /api/pipelines/{id}/runs/{runId}/steps` (WS4 item F1). */
  getRunSteps(id: string, runId: string, signal?: AbortSignal): Promise<PipelineRunStep[]>
  /** `GET /api/pipelines/{id}/runs/{runId}/logs?cursor=` — one bounded page,
   * forward from `cursor` when given (WS4 item F1). */
  getRunLogs(
    id: string,
    runId: string,
    cursor?: string,
    signal?: AbortSignal
  ): Promise<PipelineRunLogsPage>
  /** `POST /api/pipelines/{id}/status` — the only console caller is WS4 item
   * F4's "Activate" action, moving a draft authored pipeline to `"ready"`
   * (WS4 item F1, judge review V10). */
  setPipelineStatus(id: string, status: string, signal?: AbortSignal): Promise<Pipeline>
}
