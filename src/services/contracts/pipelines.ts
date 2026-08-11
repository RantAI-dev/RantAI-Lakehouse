import type { EntityStatus } from "@/lib/status"

export type PipelineKind = "batch" | "incremental" | "document" | "vector"

export type Pipeline = {
  id: string
  name: string
  kind: PipelineKind
  status: EntityStatus
  owner: string
  source: string
  target: string
  schedule: string
  lastRunAt: string
  nextRunAt?: string
  slaOk: boolean
  freshnessLagSeconds: number
}

export type PipelineRun = {
  id: string
  pipelineId: string
  status: EntityStatus
  startedAt: string
  endedAt?: string
  processed: number
  accepted: number
  rejected: number
  retried: number
  costUnits: number
  error?: string
}

export type PipelineDetail = Pipeline & {
  description: string
  graph: { id: string; label: string; kind: string; status: EntityStatus }[]
  runs: PipelineRun[]
  configSummary: { key: string; value: string }[]
}

export interface PipelineService {
  listPipelines(signal?: AbortSignal): Promise<Pipeline[]>
  getPipeline(id: string, signal?: AbortSignal): Promise<PipelineDetail>
  listRuns(pipelineId: string, signal?: AbortSignal): Promise<PipelineRun[]>
}
