import type { EntityStatus } from "@/lib/status"

export type StreamingJob = {
  id: string
  name: string
  status: EntityStatus
  owner: string
  sources: string[]
  sinks: string[]
  lagSeconds: number
  throughputPerSec: number
  stateSizeBytes: number
  watermarkIntervalSec: number
  lastBarrierAt: string
}

export type StreamingJobDetail = StreamingJob & {
  definitionSql: string
  triggers: { id: string; condition: string; target: string }[]
  checkpoints: { id: string; at: string; sizeBytes: number }[]
}

export interface StreamingService {
  listJobs(signal?: AbortSignal): Promise<StreamingJob[]>
  getJob(id: string, signal?: AbortSignal): Promise<StreamingJobDetail>
}
