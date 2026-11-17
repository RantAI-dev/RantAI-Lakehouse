"use client"

import { Suspense } from "react"
import { CatalogPage } from "@/features/catalog/catalog-page"
import { LoadingSkeleton } from "@/components/patterns/page-states"

/**
 * Thin App Router page for the Catalog. Suspense because the page reads
 * its table state from the URL (`useSearchParams`).
 */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton />}>
      <CatalogPage />
    </Suspense>
  )
}
