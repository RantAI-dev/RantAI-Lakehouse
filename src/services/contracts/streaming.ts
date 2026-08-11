import type { EntityStatus } from "@/lib/status"

export type StreamingJob = {
  id: string
  name: string
  status: EntityStatus
  owner: string
  sources: string[]
  sinks: string[]
  /** Catalog assets produced by sinks when known. */
  sinkAssetIds?: string[]
  lagSeconds: number
  throughputPerSec: number
  stateSizeBytes: number
  watermarkIntervalSec: number
  lastBarrierAt: string
}

export type StreamingTrigger = {
  id: string
  condition: string
  /** Display label for the trigger target. */
  target: string
  /** In-app href when the target is a product route. */
  targetHref?: string
}

export type StreamingJobDetail = StreamingJob & {
  definitionSql: string
  triggers: StreamingTrigger[]
  checkpoints: { id: string; at: string; sizeBytes: number }[]
}

export type CreateStreamingJobInput = {
  name: string
  sources: string[]
  sinks: string[]
  definitionSql: string
  watermarkIntervalSec: number
  triggerCondition: string
  owner?: string
}

export interface StreamingService {
  listJobs(signal?: AbortSignal): Promise<StreamingJob[]>
  getJob(id: string, signal?: AbortSignal): Promise<StreamingJobDetail>
  createStreamingJob(
    input: CreateStreamingJobInput,
    signal?: AbortSignal
  ): Promise<StreamingJob>
  pauseJob(id: string, signal?: AbortSignal): Promise<StreamingJob>
  resumeJob(id: string, signal?: AbortSignal): Promise<StreamingJob>
}
