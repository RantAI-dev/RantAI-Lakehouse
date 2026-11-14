import { ReportReaderPage } from "@/features/dashboards/report-reader-page"

/** One report, with its charts drawn live for the reader (`BI-18` part D). */
export default async function Page({ params }: { params: Promise<{ reportId: string }> }) {
  const { reportId } = await params
  return <ReportReaderPage id={decodeURIComponent(reportId)} />
}
