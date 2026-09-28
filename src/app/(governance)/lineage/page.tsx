"use client"

import { Suspense } from "react"
import { LineagePage } from "@/features/governance/lineage-page"
import { LoadingSkeleton } from "@/components/patterns/page-states"

/** Thin App Router page for LineagePage; Suspense because the page reads `?focus=` via `useSearchParams`. */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton />}>
      <LineagePage />
    </Suspense>
  )
}
