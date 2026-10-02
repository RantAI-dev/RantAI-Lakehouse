"use client"

import { useParams } from "next/navigation"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService } from "@/hooks/use-service"
import { pipelineService } from "@/services"
import { PipelineEditor } from "./pipeline-create-page"

/**
 * `/pipelines/[pipelineId]/edit`: the create form, filled from the stored
 * definition. Only an authored pipeline has one; a Dagster job is defined
 * in code and is changed there.
 */
export function PipelineEditPage() {
  const { pipelineId } = useParams<{ pipelineId: string }>()
  const state = useService((s) => pipelineService.getPipeline(pipelineId, s), [pipelineId])
  if (state.status === "loading") return <LoadingSkeleton rows={8} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  if (state.data.engine !== "authored" || !state.data.definition) {
    return (
      <EmptyState
        title="This pipeline is defined in code"
        description="A Dagster job is changed in its code location, not in the console."
      />
    )
  }
  return <PipelineEditor existing={state.data} />
}
