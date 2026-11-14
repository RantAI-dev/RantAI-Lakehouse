import { Suspense } from "react"
import { DashboardListPage } from "@/features/dashboards/dashboard-list-page"
import { DashboardListPageSkeleton } from "@/features/dashboards/dashboard-list-skeleton"

/**
 * Daftar semua dashboard — surface manajemen (rename, bagikan, hapus), dituju
 * dengan sengaja lewat board switcher. `/dashboards` sendiri meneruskan ke
 * board terakhir, karena melewati daftar tiap kali hanya menambah satu klik.
 * Suspense karena state tabel (search/filter) dibaca dari query string; while
 * it waits it draws the page's own placeholder, the one the loading shell
 * draws for this route, not a line of text (owner QA 2026-11-14).
 */
export default function Page() {
  return (
    <Suspense fallback={<DashboardListPageSkeleton />}>
      <DashboardListPage />
    </Suspense>
  )
}
