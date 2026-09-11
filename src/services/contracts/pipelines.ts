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

export type PipelineDetail = Pipeline & {
  /** Populated by WS4 from the Dagster op graph. Absent until then. */
  graph?: { id: string; label: string; kind: string; status: EntityStatus }[]
  /** Populated by WS4 from the job definition. Absent until then. */
  description?: string
  /** Populated by WS4 from the run config. Absent until then. */
  configSummary?: { key: string; value: string }[]
  runs: PipelineRun[]
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
}

export type GeneratePipelineInput = {
  model: string
  instruction: string
  fileName?: string
  database: string
}

export interface PipelineService {
  listPipelines(signal?: AbortSignal): Promise<Pipeline[]>
  getPipeline(id: string, signal?: AbortSignal): Promise<PipelineDetail>
  listRuns(pipelineId: string, signal?: AbortSignal): Promise<PipelineRun[]>
  createPipeline(input: CreatePipelineInput, signal?: AbortSignal): Promise<Pipeline>
  generatePipelineFromPrompt(
    input: GeneratePipelineInput,
    signal?: AbortSignal
  ): Promise<Pipeline>
  triggerRun(id: string, signal?: AbortSignal): Promise<PipelineRun>
  cancelRun(runId: string, signal?: AbortSignal): Promise<PipelineRun>
  retryRun(runId: string, signal?: AbortSignal): Promise<PipelineRun>
  pausePipeline(id: string, signal?: AbortSignal): Promise<Pipeline>
  resumePipeline(id: string, signal?: AbortSignal): Promise<Pipeline>
}
