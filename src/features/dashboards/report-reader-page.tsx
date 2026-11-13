"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useTheme } from "next-themes"
import { ArrowLeft, Download, FileText, Pencil, Trash2 } from "lucide-react"

import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot } from "@/features/copilot/use-copilot"
import { folderPath } from "@/lib/folder-tree"
import { formatRelativeTime } from "@/lib/format"
import { notifyError, notifySuccess, withNotify } from "@/lib/notify"
import { buildReportPageContext, chartCount } from "@/lib/report-blocks"
import { dashboardReportService, dashboardService } from "@/services"
import { downloadReportPdf } from "./chart-download"
import { ReportBlocks } from "./report-blocks"
import { ReportingProvider } from "./reporting-context"

/** Where the Reports tab is. */
export const REPORTS_TAB_HREF = "/dashboards/browse?tab=reports"

/**
 * One report, as any reader sees it (`BI-18` part D, T5, decisions R1 to R3):
 * its blocks in order, each chart drawn live from what the server read as this
 * person (their masks and row filters), a chart they may not see as the
 * server's fixed sentence. Edit and Delete are offered by the level the
 * server says the person holds on the report's folder; Download PDF when the
 * deployment can make one and the person may download. The server checks all
 * of it again.
 */
export function ReportReaderPage({ id }: { id: string }) {
  const router = useRouter()
  const { resolvedTheme } = useTheme()
  const auth = useAuth()
  const state = useService((s) => dashboardReportService.getReport(id, s), [id])
  const folders = useService((s) => dashboardService.listFolders(s), [])
  const [deleting, setDeleting] = React.useState(false)
  const [pdfBusy, setPdfBusy] = React.useState(false)
  const deleteAction = useServiceAction(
    withNotify(
      { success: "Moved to the trash", error: "Could not delete the report" },
      (signal) => dashboardReportService.deleteReport(id, signal),
    ),
  )

  // BI-18 D review round 1, SHOULD-FIX 2: tell the assistant which report is on
  // screen, as the dashboard page names its board, so it can change this one.
  const { setPageContext } = useCopilot()
  const shown = state.status === "success" ? state.data.report : null
  React.useEffect(() => {
    if (!shown) return
    setPageContext({
      key: "report-view",
      title: `Reading "${shown.title}"`,
      hint: "Ask about this report, or add a chart or a paragraph to it.",
      suggest: {
        ask: ["Summarize this report", "Which dashboards do these charts come from?"],
        build: ["Add a paragraph that sums up the charts", "Add the trend chart to this report"],
      },
      system: buildReportPageContext({ id: shown.id, title: shown.title, chartCount: chartCount(shown.blocks) }),
    })
    return () => setPageContext(null)
  }, [shown, setPageContext])

  if (state.status === "loading") return <LoadingSkeleton />
  if (state.status === "error") {
    if (state.error.code === "not_found") {
      return (
        <EmptyState
          title="This report is not available"
          description="It may have been deleted, or it is in a folder you cannot open."
          action={
            <Button variant="outline" size="sm" render={<Link href={REPORTS_TAB_HREF} />}>
              <ArrowLeft data-icon="inline-start" />
              All reports
            </Button>
          }
        />
      )
    }
    return <ErrorState error={state.error} onRetry={state.reload} />
  }

  const view = state.data
  const { report } = view
  const canWrite = auth.hasPermission("dashboard:write")
  const level = view.access.level
  const canEdit = canWrite && (level === "edit" || level === "manage")
  const canDelete = canWrite && level === "manage"
  const where = report.folderId && folders.status === "success" ? folderPath(folders.data, report.folderId) : ""
  const pdf = view.pdfSupported && view.canDownload

  async function download() {
    setPdfBusy(true)
    try {
      const name = await downloadReportPdf(report.id, { paper: "a4", orientation: "portrait" })
      notifySuccess(`Downloaded ${name}`)
    } catch (err) {
      notifyError("Could not make the PDF", err)
    } finally {
      setPdfBusy(false)
    }
  }

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-5">
      <Link
        href={REPORTS_TAB_HREF}
        className="inline-flex w-fit items-center gap-1 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <ArrowLeft className="size-3" aria-hidden />
        Reports
      </Link>
      <PageHeader
        title={report.title}
        description={[where || "No folder", report.updatedBy ? `Updated by ${report.updatedBy} ${formatRelativeTime(report.updatedAt)}` : ""]
          .filter(Boolean)
          .join(" · ")}
        actions={
          <div className="flex flex-wrap items-center gap-2">
            {pdf ? (
              <Button variant="outline" size="sm" disabled={pdfBusy} onClick={() => void download()}>
                <Download data-icon="inline-start" />
                {pdfBusy ? "Making the PDF…" : "Download PDF"}
              </Button>
            ) : null}
            {canDelete ? (
              <Button variant="outline" size="sm" onClick={() => setDeleting(true)}>
                <Trash2 data-icon="inline-start" />
                Delete
              </Button>
            ) : null}
            {canEdit ? (
              <Button size="sm" render={<Link href={`/dashboards/reports/${encodeURIComponent(report.id)}/edit`} />}>
                <Pencil data-icon="inline-start" />
                Edit
              </Button>
            ) : null}
          </div>
        }
      />
      {report.blocks.length === 0 ? (
        <EmptyState
          title="This report is empty"
          description={canEdit ? "Add headings, text and charts from your dashboards." : "Nothing has been written here yet."}
          action={
            canEdit ? (
              <Button size="sm" render={<Link href={`/dashboards/reports/${encodeURIComponent(report.id)}/edit`} />}>
                <FileText data-icon="inline-start" />
                Start writing
              </Button>
            ) : null
          }
        />
      ) : (
        <ReportingProvider reporting={view.reporting ?? undefined}>
          <article aria-label={report.title}>
            <ReportBlocks blocks={report.blocks} charts={view.charts} dark={resolvedTheme === "dark"} />
          </article>
        </ReportingProvider>
      )}
      <ConfirmActionDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={`Delete ${report.title}?`}
        description="The report goes to the trash. You can restore it for 30 days. Its charts stay on their dashboards."
        confirmLabel="Delete"
        destructive
        confirming={deleteAction.status === "pending"}
        onConfirm={() => {
          void (async () => {
            const done = await deleteAction.run()
            setDeleting(false)
            if (done !== null) router.push(REPORTS_TAB_HREF)
          })()
        }}
      />
    </div>
  )
}
