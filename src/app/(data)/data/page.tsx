"use client"

import { Suspense } from "react"
import { DataExplorerPage } from "@/features/catalog/data-explorer-page"
import { LoadingSkeleton } from "@/components/patterns/page-states"

/** Thin App Router page for Data Explorer. */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton />}>
      <DataExplorerPage />
    </Suspense>
  )
}
