import { ReportEditorPage } from "@/features/dashboards/report-editor-page"

/** Change a report's title, folder and blocks (`BI-18` part D). */
export default async function Page({ params }: { params: Promise<{ reportId: string }> }) {
  const { reportId } = await params
  return <ReportEditorPage id={decodeURIComponent(reportId)} />
}
