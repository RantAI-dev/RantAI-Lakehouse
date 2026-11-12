import { ChartBuilderPage } from "@/features/dashboards/chart-builder-page"

/** `WEB-7` D2/D4/D6: the chart builder for a saved chart, loaded by id. */
export default async function Page({
  params,
}: {
  params: Promise<{ id: string; chartId: string }>
}) {
  const { id, chartId } = await params
  return (
    <ChartBuilderPage boardId={decodeURIComponent(id)} chartId={decodeURIComponent(chartId)} />
  )
}
