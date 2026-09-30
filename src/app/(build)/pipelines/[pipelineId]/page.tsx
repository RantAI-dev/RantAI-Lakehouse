"use client"

import { Suspense } from "react"
import { LoadingSkeleton } from "@/components/patterns/page-states"
import { PipelineDetailPage } from "@/features/pipelines/pipeline-detail-page"

/**
 * Thin App Router page for PipelineDetailPage. The page reads `?run=` and
 * `?tab=` with `useSearchParams`, which needs a Suspense boundary.
 */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton rows={8} />}>
      <PipelineDetailPage />
    </Suspense>
  )
}
