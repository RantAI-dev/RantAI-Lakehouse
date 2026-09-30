"use client"

import { Suspense } from "react"
import { LoadingSkeleton } from "@/components/patterns/page-states"
import { PipelineEditPage } from "@/features/pipelines/pipeline-edit-page"

/** Thin App Router page for PipelineEditPage; the editor reads search params, so it needs Suspense. */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton rows={8} />}>
      <PipelineEditPage />
    </Suspense>
  )
}
