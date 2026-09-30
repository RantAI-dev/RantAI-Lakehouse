import { DashboardResolver } from "@/features/dashboards/dashboard-resolver"

/**
 * Not a page but a successor: it leads to the board you last had open, or to
 * the only one you have. The list lives at `/dashboards/browse`, and a single
 * dashboard's canvas at `/dashboards/[id]`.
 */
export default function Page() {
  return <DashboardResolver />
}
