import type { EntityStatus } from "@/lib/status"

export type PipelineKind = "batch" | "incremental" | "document" | "vector"

/**
 * Where a pipeline comes from, which decides what can be done to it: an
 * orchestrator job can be triggered and its runs cancelled or retried; a
 * pipeline authored in the console has no engine behind it yet.
 */
export type PipelineOrigin = "orchestrator" | "authored"

export type Pipeline = {
  id: string
  name: string
  kind: PipelineKind
  status: EntityStatus
  owner: string
  source: string
  target: string
  /** Optional link to the ingress connector that feeds this pipeline. */
  connectorId?: string
  /** Catalog asset IDs for cross-navigation (mock → future API). */
  sourceAssetId?: string
  targetAssetId?: string
  schedule: string
  /** Null when the pipeline has never run. */
  lastRunAt: string | null
  nextRunAt?: string
  slaOk: boolean
  /** Null when nothing measures this pipeline's output freshness. */
  freshnessLagSeconds: number | null
  origin: PipelineOrigin
  description?: string
  incrementalColumn?: string
  transforms?: string[]
  fbicEnabled?: boolean
}

/** A pipeline list, plus why the orchestrator's half of it may be missing. */
export type PipelineList = {
  pipelines: Pipeline[]
  orchestratorError: string | null
}

export type PipelineRun = {
  id: string
  pipelineId: string
  status: EntityStatus
  startedAt: string
  endedAt?: string
  /** Row counts the orchestrator does not report; null means unknown. */
  processed: number | null
  accepted: number | null
  rejected: number | null
  retried: number | null
  /** Null: nothing prices a pipeline run yet. */
  costUnits: number | null
  /** How long a finished run took; null while it is still running. */
  durationSeconds?: number | null
  error?: string
  checkpoint?: string
  auditEventId?: string
  /** Output dataset produced by this run when known. */
  outputAssetId?: string
}

export type PipelineDetail = Pipeline & {
  /** Nodes as stored: what this pipeline reads and writes. No per-node
   *  status — nothing here executes it, so nothing knows. */
  graph: { id: string; label: string; kind: string; status?: EntityStatus }[]
  runs: PipelineRun[]
  configSummary: { key: string; value: string }[]
  /** Why there is no run history, when there is none to be had. */
  runsUnavailable?: string | null
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
  description?: string
}

export type GeneratePipelineInput = {
  instruction: string
  database: string
}

export interface PipelineService {
  listPipelines(signal?: AbortSignal): Promise<PipelineList>
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
