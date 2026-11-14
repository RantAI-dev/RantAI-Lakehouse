"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { LayoutTemplate, Trash2 } from "lucide-react"

import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import type { FolderLike } from "@/lib/folder-tree"
import { formatRelativeTime } from "@/lib/format"
import { notifyInfo, withNotify } from "@/lib/notify"
import { dashboardTemplateService } from "@/services"
import type { DashboardTemplate } from "@/services"
import { notifyDashboardsChanged } from "./dashboard-events"
import { UseTemplateDialog } from "./template-dialogs"

/**
 * The Templates tab of the Dashboards page (`BI-18` part D, T6, decisions T1
 * to T4): every template with its name, the dashboard it was saved from, who
 * saved it and when. "Use template" asks for a name and a folder, then opens
 * the new dashboard; Delete asks first. The page shows this tab only to a
 * holder of `dashboard:write`, and the server refuses everyone else anyway.
 */
export function TemplateList({ folders, query = "" }: { readonly folders: FolderLike[]; readonly query?: string }) {
  const router = useRouter()
  const state = useService((s) => dashboardTemplateService.listTemplates(s), [])
  const [using, setUsing] = React.useState<DashboardTemplate | null>(null)
  const [deleting, setDeleting] = React.useState<DashboardTemplate | null>(null)
  const reload = state.reload
  const deleteAction = useServiceAction(
    withNotify(
      { success: "Template deleted", error: "Could not delete the template" },
      (signal, id: string) => dashboardTemplateService.deleteTemplate(id, signal),
    ),
  )

  if (state.status === "loading") return <LoadingSkeleton />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={reload} />
  // The page's search (`BI-59` T1) narrows the list by name and source.
  const q = query.trim().toLowerCase()
  const templates = q
    ? state.data.templates.filter((t) => [t.name, t.sourceBoardName, t.createdBy].some((f) => f.toLowerCase().includes(q)))
    : state.data.templates

  return (
    <div className="flex flex-col gap-4">
      {templates.length === 0 && q ? (
        <EmptyState title="No matching templates" description="No template matches the search." />
      ) : templates.length === 0 ? (
        <EmptyState
          title="No templates yet"
          description="Open a dashboard and choose Save as template in its ⋯ menu. Its layout, charts and filters are saved, without data."
        />
      ) : (
        <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3" aria-label="Templates">
          {templates.map((t) => (
            <li
              key={t.id}
              className="flex flex-col gap-3 rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] transition-colors hover:border-primary/40"
            >
              <div className="flex items-start gap-2">
                <LayoutTemplate className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">{t.name}</p>
                  <p className="truncate text-xs text-muted-foreground">
                    From {t.sourceBoardName || "a dashboard"}
                  </p>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  className="-mt-1.5 -mr-2 size-8 p-0"
                  aria-label={`Delete ${t.name}`}
                  onClick={() => setDeleting(t)}
                >
                  <Trash2 className="size-4" />
                </Button>
              </div>
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span className="tabular-nums">
                  {t.chartCount} {t.chartCount === 1 ? "chart" : "charts"}
                </span>
                {t.createdBy ? <span>{t.createdBy}</span> : null}
                <span>Saved {formatRelativeTime(t.createdAt)}</span>
              </div>
              <Button size="sm" variant="outline" className="self-start" onClick={() => setUsing(t)}>
                Use template
              </Button>
            </li>
          ))}
        </ul>
      )}
      {state.data.truncated ? (
        <p className="text-xs text-muted-foreground">Showing the 500 newest templates.</p>
      ) : null}
      <UseTemplateDialog
        template={using}
        folders={folders}
        onOpenChange={(open) => !open && setUsing(null)}
        onStarted={(started) => {
          setUsing(null)
          notifyDashboardsChanged()
          if (started.leftOut.length > 0) {
            notifyInfo(
              `${started.leftOut.length} chart(s) could not be copied`,
              started.leftOut.map((c) => `${c.title}: ${c.reason}`).join(" "),
            )
          }
          router.push(`/dashboards/${encodeURIComponent(started.board.id)}`)
        }}
      />
      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={`Delete the template ${deleting?.name ?? ""}?`}
        description="The template is deleted for good. Dashboards already made from it are not changed."
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
