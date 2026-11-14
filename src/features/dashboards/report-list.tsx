"use client"

import * as React from "react"
import Link from "next/link"
import { FileText, Folder, Lock, MoreHorizontal, Pencil, Trash2 } from "lucide-react"

import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useService, useServiceAction } from "@/hooks/use-service"
import { folderPath, type FolderLike } from "@/lib/folder-tree"
import { formatRelativeTime } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { dashboardReportService } from "@/services"
import type { ReportSummary } from "@/services"

/**
 * The Reports tab of the Dashboards page (`BI-18` part D, T5, decisions R1 to
 * R4): every report the person may read, grouped by the dashboard folder it is
 * filed in (`""` is "No folder"), newest first within a folder.
 *
 * The page's search narrows it (`query`), and "New report" is in the page's
 * New menu (`BI-59` T1). Edit and Delete are offered only with `dashboard:write`
 * (`BI-18D-AC8`); the server refuses them anyway, and judges the folder's
 * level. Delete moves the report to the trash, which the Trash dialog lists
 * with the dashboards.
 */
export function ReportList({
  folders,
  canWrite,
  query,
}: {
  readonly folders: readonly (FolderLike & { ownerId?: string })[]
  readonly canWrite: boolean
  readonly query: string
}) {
  const state = useService((s) => dashboardReportService.listReports(s), [])
  const [deleting, setDeleting] = React.useState<ReportSummary | null>(null)
  const reload = state.reload
  const deleteAction = useServiceAction(
    withNotify(
      { success: "Moved to the trash", error: "Could not delete the report" },
      (signal, id: string) => dashboardReportService.deleteReport(id, signal),
    ),
  )

  const reports = React.useMemo(() => state.data?.reports ?? [], [state.data])
  const pathOf = React.useCallback(
    (r: ReportSummary) => (r.folderId ? folderPath([...folders], r.folderId) : ""),
    [folders],
  )
  const shown = React.useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return reports
    return reports.filter((r) =>
      [r.title, r.createdBy, r.updatedBy, pathOf(r)].some((field) => field.toLowerCase().includes(q)),
    )
  }, [reports, query, pathOf])
  const sections = React.useMemo(() => {
    const by = new Map<string, ReportSummary[]>()
    for (const r of shown) {
      const key = r.folderId && folders.some((f) => f.id === r.folderId) ? r.folderId : ""
      by.set(key, [...(by.get(key) ?? []), r])
    }
    return [...by.entries()]
      .map(([folderId, items]) => ({ folderId, path: folderId ? folderPath([...folders], folderId) : "", items }))
      .sort((a, b) => (a.folderId === "" ? 1 : b.folderId === "" ? -1 : a.path.localeCompare(b.path)))
  }, [shown, folders])

  if (state.status === "loading") return <LoadingSkeleton />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={reload} />

  return (
    <div className="flex flex-col gap-4">
      {reports.length === 0 ? (
        <EmptyState
          title="No reports yet"
          description={
            canWrite
              ? "A report is a written page with live charts between the paragraphs. Pick the charts from your dashboards."
              : "Nobody has shared a report with you yet."
          }
        />
      ) : (
        <>
          {state.data.truncated ? (
            <p className="text-xs text-muted-foreground">
              Showing the 1,000 most recently changed reports. Search to find an older one.
            </p>
          ) : null}
          {shown.length === 0 ? (
            <EmptyState title="No matching reports" description="No report matches the search." />
          ) : (
            <div className="flex flex-col gap-5">
              {sections.map((section) => {
                const personal = folders.find((f) => f.id === section.folderId)?.ownerId
                return (
                  <section key={section.folderId || "none"} className="flex flex-col gap-2" aria-label={section.path || "No folder"}>
                    <h3 className="flex items-center gap-2 px-1 text-sm font-medium text-foreground">
                      {section.folderId ? (
                        personal ? <Lock className="size-4 text-muted-foreground" aria-hidden /> : <Folder className="size-4 text-muted-foreground" aria-hidden />
                      ) : null}
                      {section.path || "No folder"}
                      <span className="text-xs font-normal text-muted-foreground tabular-nums">{section.items.length}</span>
                    </h3>
                    <ul className="divide-y rounded-xl border bg-card">
                      {section.items.map((r) => (
                        <ReportRow key={r.id} report={r} canWrite={canWrite} onDelete={() => setDeleting(r)} />
                      ))}
                    </ul>
                  </section>
                )
              })}
            </div>
          )}
        </>
      )}
      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={`Delete ${deleting?.title ?? "report"}?`}
        description="The report goes to the trash. You can restore it for 30 days. Its charts stay on their dashboards."
        confirmLabel="Delete"
        destructive
        confirming={deleteAction.status === "pending"}
        onConfirm={() => {
          const target = deleting
          if (!target) return
          void (async () => {
            await deleteAction.run(target.id)
            setDeleting(null)
            reload()
          })()
        }}
      />
    </div>
  )
}

function ReportRow({
  report: r,
  canWrite,
  onDelete,
}: {
  readonly report: ReportSummary
  readonly canWrite: boolean
  readonly onDelete: () => void
}) {
  return (
    <li className="group relative flex items-center gap-3 px-4 py-3 transition-colors hover:bg-muted/40">
      <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      <div className="min-w-0 flex-1">
        <Link
          href={`/dashboards/reports/${encodeURIComponent(r.id)}`}
          className="block truncate text-sm font-medium outline-none after:absolute after:inset-0 group-hover:underline focus-visible:after:ring-2 focus-visible:after:ring-inset focus-visible:after:ring-ring/50"
        >
          {r.title}
        </Link>
        <p className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-muted-foreground">
          <span className="tabular-nums">
            {r.chartCount} {r.chartCount === 1 ? "chart" : "charts"}
          </span>
          {r.updatedBy ? <span>{r.updatedBy}</span> : null}
          {r.updatedAt ? <span>Updated {formatRelativeTime(r.updatedAt)}</span> : null}
        </p>
      </div>
      {canWrite ? (
        <div className="relative z-10">
          <DropdownMenu>
            <DropdownMenuTrigger
              render={<Button variant="ghost" size="icon" className="size-8 p-0" aria-label={`Actions for ${r.title}`} />}
            >
              <MoreHorizontal className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuItem render={<Link href={`/dashboards/reports/${encodeURIComponent(r.id)}/edit`} />}>
                <Pencil className="size-4" /> Edit
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem variant="destructive" onClick={onDelete}>
                <Trash2 className="size-4" /> Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      ) : null}
    </li>
  )
}
