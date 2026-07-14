export type PipelineStatus = "Success" | "Failed" | "Running"

/** Sample rows written by the last successful run (for UI preview). */
export interface PipelineRunPreview {
  columns: string[]
  rows: string[][]
}

/** Metrics and table preview after a successful pipeline run. */
export interface PipelineRunResult {
  runId: string
  completedAt: string
  outputTable: string
  rowsWritten: string
  duration: string
  preview: PipelineRunPreview
  /** Shown when pipeline status is Running but preview is from a prior success. */
  previewNote?: string
}

export interface PipelineStepDetail {
  title: string
  description: string
}

export interface PipelineDetail {
  steps: PipelineStepDetail[]
  sourceConfig: string
  transformLogic: string
  destination: string
}

export interface PipelineItem {
  id: string
  name: string
  description: string
  flow: { from: string; to: string }
  status: PipelineStatus
  lastRun: string
  records: string
  schedule: string
  successRate: string
  detail?: PipelineDetail
  /** Present when there is output to show from a completed successful run. */
  lastSuccessResult?: PipelineRunResult
}
