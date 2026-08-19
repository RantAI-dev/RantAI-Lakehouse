import { Suspense } from "react"
import { DashboardPage } from "@/features/dashboards/dashboard-page"

/** Thin App Router page untuk Dashboards. Suspense untuk useSearchParams. */
export default function Page() {
  return (
    <Suspense fallback={<div className="p-4 text-sm text-muted-foreground">Memuat…</div>}>
      <DashboardPage />
    </Suspense>
  )
}
