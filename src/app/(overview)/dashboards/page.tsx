import { Suspense } from "react"
import { DashboardPage } from "@/features/dashboards/dashboard-page"
import { DashboardPageSkeleton } from "@/features/dashboards/dashboard-skeleton"

/** Thin App Router page for Dashboards. Suspense for useSearchParams. */
export default function Page() {
  return (
    <Suspense fallback={<DashboardPageSkeleton />}>
      <DashboardPage />
    </Suspense>
  )
}
