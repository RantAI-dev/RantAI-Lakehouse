import { DashboardPage } from "@/features/dashboards/dashboard-page"

/**
 * Kanvas satu dashboard. Sebelumnya ini adalah `/dashboards?board=<id>`;
 * `/dashboards` kini dipakai halaman daftar, jadi board pindah ke segmen
 * path. `default` adalah board bawaan.
 */
export default async function Page({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params
  return <DashboardPage boardId={decodeURIComponent(id)} />
}
