import { Suspense } from "react"
import { DashboardListPage } from "@/features/dashboards/dashboard-list-page"
import { DashboardListPageSkeleton } from "@/features/dashboards/dashboard-list-skeleton"

/**
 * The list of every dashboard: the managing surface (rename, share, delete),
 * reached on purpose through the board switcher. `/dashboards` itself goes on
 * to the last board, because passing through the list every time only adds a
 * click. Suspense, because the table's state (search, filters) is read from
 * the query string; while it waits it draws the page's own placeholder, the
 * one the loading shell draws for this route, not a line of text (owner QA
 * 2026-11-14).
 */
export default function Page() {
  return (
    <Suspense fallback={<DashboardListPageSkeleton />}>
      <DashboardListPage />
    </Suspense>
  )
}
