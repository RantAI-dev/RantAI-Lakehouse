import { ChartBuilderPage } from "@/features/dashboards/chart-builder-page"

/**
 * `WEB-7` D2/D6: the chart builder for a new chart is its own page. The
 * board id names where the chart lands and where Cancel and Save go.
 */
export default async function Page({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params
  return <ChartBuilderPage boardId={decodeURIComponent(id)} />
}
