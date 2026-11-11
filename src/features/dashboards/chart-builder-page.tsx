"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter, useSearchParams } from "next/navigation"

import { EmptyState, ErrorState, LoadingSkeleton, PermissionState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useAuth } from "@/features/auth/auth-provider"
import { useCopilot } from "@/features/copilot/use-copilot"
import { useService } from "@/hooks/use-service"
import { offerFor } from "@/lib/access-level"
import { dashboardService } from "@/services"
import { ChartBuilder, type ChartDef } from "./chart-builder"
import {
  builtInBoardEditable,
  chartParamValues,
  isBuiltInBoard,
  takeChartDraft,
  type ChartDraft,
} from "./chart-page"

/**
 * `/dashboards/[id]/charts/new` and `.../[chartId]/edit` (`WEB-7` T6, D2/D6):
 * the chart builder as a page.
 *
 * The page is what the dashboard's buttons and the assistant's draft card
 * open. It loads the board's own access level first, because the server lets
 * a person who holds `edit` on this dashboard build a chart on it without
 * holding `dashboard:write` anywhere (`policy.rs`: `POST`/`PUT
 * /api/dashboard/specs` are `RequiresAuth`, and the handler resolves the
 * level). So the route's `ROUTE_ACCESS` entry is `[]` — any signed-in person
 * may open the address — and this page decides: below `edit`, it shows the
 * same "You don't have access" the guard would.
 *
 * Edit loads the chart by id through the chart list (`dashboardService.getChart`),
 * so a reload works. An id that names nothing shows the not-found state.
 *
 * The dashboard's parameter values arrive in the address (`chartParamValues`),
 * and the assistant's draft arrives in `sessionStorage` (`takeChartDraft`,
 * read once). Neither is saved with the chart.
 */
export function ChartBuilderPage({ boardId, chartId }: { boardId: string; chartId?: string }) {
  const router = useRouter()
  const searchParams = useSearchParams()
  const { hasPermission } = useAuth()
  const copilot = useCopilot()
  const backHref = `/dashboards/${encodeURIComponent(boardId)}`
  // The built-in dashboard is not a stored row, so `GET /api/dashboard/access`
  // refuses it; its level is the caller's starting point, which the server
  // reads at the root (`builtInBoardEditable`).
  const builtIn = isBuiltInBoard(boardId)

  const access = useService(
    (signal) =>
      builtIn ? Promise.resolve(null) : dashboardService.getAccess("board", boardId, signal),
    [boardId, builtIn]
  )
  const boards = useService((signal) => dashboardService.listBoards(signal), [])
  const chart = useService(
    (signal) =>
      chartId ? dashboardService.getChart(chartId, signal) : Promise.resolve(null),
    [chartId]
  )
  // `undefined` = the draft has not been read yet (sessionStorage is absent
  // during server rendering), `null` = there is none.
  const [draft, setDraft] = React.useState<ChartDraft | null | undefined>(undefined)
  React.useEffect(() => {
    setDraft(chartId ? null : takeChartDraft(boardId))
  }, [boardId, chartId])

  if (
    access.status === "loading" ||
    boards.status === "loading" ||
    chart.status === "loading" ||
    draft === undefined
  ) {
    return <LoadingSkeleton rows={8} />
  }
  if (access.status === "error") return <ErrorState error={access.error} onRetry={access.reload} />
  const canEdit = builtIn
    ? builtInBoardEditable(hasPermission)
    : access.status === "success" && access.data !== null
      ? offerFor(access.data.yourLevel, hasPermission("dashboard:write")).canEdit
      : false
  if (!canEdit) return <PermissionState />
  if (chart.status === "error") return <ErrorState error={chart.error} onRetry={chart.reload} />
  if (chartId && !chart.data) {
    return (
      <EmptyState
        title="Chart not found"
        description="This chart was deleted, or the address names one that never existed."
        action={
          <Button variant="outline" size="sm" render={<Link href={backHref} />}>
            Back to the dashboard
          </Button>
        }
      />
    )
  }

  // The page that opens an existing chart is the one that names it; a draft
  // is only for a new one.
  const initial = chartId
    ? ((chart.data?.def ?? {}) as ChartDef)
    : ((draft?.def ?? undefined) as ChartDef | undefined)

  return (
    <ChartBuilder
      board={boardId}
      boards={boards.data ?? []}
      initial={initial}
      editId={chartId}
      paramValues={chartParamValues(searchParams.toString())}
      backHref={backHref}
      onCancel={() => router.push(backHref)}
      // `WEB-7` D6: the board is reloaded by going back to it, which is what
      // the dialog's `onSaved` did by re-fetching in place. A draft the
      // assistant made marks its own step saved first, so the chat does not
      // offer the chart again; the Copilot provider is the app frame's and
      // survives this navigation.
      onSaved={(saved) => {
        if (draft?.complete) {
          copilot.completeToolStep(draft.complete.messageIndex, draft.complete.stepIndex, {
            created: true,
            title: saved.title,
            kind: saved.kind,
            mart: saved.mart,
            board: saved.board,
            via: "builder",
          })
        }
        router.push(backHref)
      }}
    />
  )
}
