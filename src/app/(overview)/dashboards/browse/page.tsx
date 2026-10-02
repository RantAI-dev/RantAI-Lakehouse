import { Suspense } from "react"
import { DashboardListPage } from "@/features/dashboards/dashboard-list-page"

/**
 * Daftar semua dashboard — surface manajemen (rename, bagikan, hapus), dituju
 * dengan sengaja lewat board switcher. `/dashboards` sendiri meneruskan ke
 * board terakhir, karena melewati daftar tiap kali hanya menambah satu klik.
 * Suspense karena state tabel (search/filter) dibaca dari query string.
 */
export default function Page() {
  return (
    <Suspense fallback={<div className="p-4 text-sm text-muted-foreground">Loading…</div>}>
      <DashboardListPage />
    </Suspense>
  )
}
