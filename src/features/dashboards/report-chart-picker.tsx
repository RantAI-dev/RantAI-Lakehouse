"use client"

/**
 * Choosing a chart for a report (`BI-62`, decisions D5 and D6): the charts
 * as live pictures, grouped by dashboard and searched by title (DESIGN.md §2,
 * "choices are previews, not a list of names"). The same grid is the `/`
 * menu's picker (a dialog) and the Charts tab of the side panel (compact,
 * draggable).
 *
 * Each picture is the chart's own `TileBody` drawn from what its dashboard
 * read for this person (`report-charts.tsx`), drawn at twice the size and
 * scaled down so a number or an axis keeps its proportions; a dashboard's
 * data is read only when one of its pictures is on screen. Only dashboards
 * the person may see are listed, only their data charts, and the server
 * checks both again on save.
 */
import * as React from "react"
import { useTheme } from "next-themes"
import { Search, X } from "lucide-react"

import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog"
import { Skeleton } from "@/components/ui/skeleton"
import { MAX_REPORT_CHARTS } from "@/lib/report-blocks"
import { cn } from "@/lib/utils"
import { CHART_MIME } from "./report-editor-extensions"
import { searchBoards, useReportCharts, type ReportChartBoard, type ReportChartChoice } from "./report-charts"
import { EASE, FOCUS, useOnScreen } from "./report-editor-ui"
import { TileBody } from "./tile-body"

/** A chart drawn small for choosing: the person's own data, no legend, not interactive. */
export function ChartThumb({ id, className }: { readonly id: string; readonly className?: string }) {
  const charts = useReportCharts()
  const { resolvedTheme } = useTheme()
  const entry = charts.entry(id)
  const choice = charts.choice(id)
  const failed = choice ? charts.boardState(choice.board) === "error" : false
  return (
    <div aria-hidden className={cn("pointer-events-none relative overflow-hidden rounded-lg bg-background/70 dark:bg-background/40", className)}>
      {entry && entry.unavailable === undefined ? (
        <div className="absolute top-0 left-0 h-[200%] w-[200%] origin-top-left scale-50 p-3">
          <TileBody spec={entry.spec} cell={entry.cell ?? undefined} dark={resolvedTheme === "dark"} loading={false} hideLegend hideDataFrom />
        </div>
      ) : failed || entry ? (
        <span className="grid h-full place-items-center px-2 text-center text-[11px] leading-4 text-muted-foreground">No picture</span>
      ) : (
        <Skeleton className="absolute inset-2" />
      )}
    </div>
  )
}

function BoardSection({
  board,
  compact,
  used,
  current,
  full,
  onPick,
}: {
  readonly board: ReportChartBoard
  readonly compact: boolean
  readonly used: ReadonlySet<string>
  readonly current?: string
  readonly full: boolean
  readonly onPick: (chart: ReportChartChoice) => void
}) {
  const charts = useReportCharts()
  const [ref, seen] = useOnScreen<HTMLElement>()
  const { loadBoard } = charts
  React.useEffect(() => {
    if (seen) loadBoard(board.id)
  }, [seen, board.id, loadBoard])
  return (
    <section ref={ref} className={compact ? "mt-3 first:mt-1" : "mt-3 first:mt-0"} aria-label={board.name}>
      <h3 className={compact ? "mb-1.5 truncate px-0.5 text-xs font-medium text-muted-foreground" : "mb-2 truncate text-sm font-medium"}>{board.name}</h3>
      <ul className={compact ? "grid grid-cols-2 gap-2" : "grid grid-cols-2 gap-3 sm:grid-cols-3"}>
        {board.charts.map((c) => {
          const isCurrent = c.id === current
          return (
            <li key={c.id}>
              <button
                type="button"
                disabled={full || isCurrent}
                draggable={compact && !full}
                onDragStart={(e) => {
                  e.dataTransfer.setData(CHART_MIME, c.id)
                  e.dataTransfer.setData("text/plain", c.title)
                  e.dataTransfer.effectAllowed = "copy"
                }}
                onClick={() => onPick(c)}
                aria-label={compact ? `Add ${c.title} to the report` : undefined}
                className={cn(
                  "group grid w-full rounded-xl bg-muted/50 text-left hover:-translate-y-0.5 hover:bg-muted/80 hover:shadow-[0_10px_28px_-16px_rgb(15_23_42/0.25)] active:scale-[0.98] disabled:pointer-events-none disabled:opacity-55 motion-reduce:hover:translate-y-0 dark:bg-card dark:hover:shadow-none dark:hover:ring-1 dark:hover:ring-white/10",
                  compact ? "cursor-grab gap-1.5 p-1.5 active:cursor-grabbing" : "gap-2 p-2",
                  EASE,
                  FOCUS,
                )}
              >
                <ChartThumb id={c.id} className={compact ? "h-[72px]" : "h-24"} />
                <span className="flex min-w-0 items-center gap-1.5 px-1 pb-0.5">
                  <span className={cn("truncate font-medium", compact ? "text-xs leading-4" : "text-[13px] leading-5")} title={c.title}>
                    {c.title}
                  </span>
                  {isCurrent ? (
                    <span className="ml-auto shrink-0 rounded-full bg-muted px-1.5 text-[11px] leading-5 text-muted-foreground">Current</span>
                  ) : used.has(c.id) ? (
                    compact ? (
                      <span aria-label="In the report" className="ml-auto size-1.5 shrink-0 rounded-full bg-sky-500" />
                    ) : (
                      <span className="ml-auto shrink-0 rounded-full bg-sky-500/12 px-1.5 text-[11px] leading-5 text-sky-800 dark:text-sky-300">In report</span>
                    )
                  ) : null}
                </span>
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}

/** The charts grouped by dashboard, searched by `query`; the dialog's body and the Charts tab's. */
export function ChartChoices({
  query,
  onClearQuery,
  compact = false,
  used,
  current,
  full,
  onPick,
}: {
  readonly query: string
  readonly onClearQuery: () => void
  readonly compact?: boolean
  readonly used: ReadonlySet<string>
  readonly current?: string
  readonly full: boolean
  readonly onPick: (chart: ReportChartChoice) => void
}) {
  const charts = useReportCharts()
  const groups = searchBoards(charts.boards, query)
  if (charts.status === "loading") {
    return (
      <div className="grid grid-cols-2 gap-3 pt-1" aria-busy="true" aria-label="Loading charts">
        {Array.from({ length: 4 }).map((_, i) => (
          <Skeleton key={i} className={compact ? "h-[96px] rounded-xl" : "h-32 rounded-xl"} />
        ))}
      </div>
    )
  }
  if (charts.status === "error") {
    return <p className="px-1 py-8 text-center text-sm text-muted-foreground">The charts could not be loaded.</p>
  }
  if (charts.boards.length === 0) {
    return <p className="px-1 py-8 text-center text-sm text-muted-foreground">No dashboard has a chart yet. Make one first.</p>
  }
  if (groups.length === 0) {
    return (
      <div className="grid place-items-center gap-1 py-10 text-center">
        <p className="text-sm font-medium">No charts match “{query}”</p>
        <button type="button" onClick={onClearQuery} className={cn("rounded text-sm text-primary hover:underline dark:text-sky-300", FOCUS)}>
          Clear the search
        </button>
      </div>
    )
  }
  return (
    <>
      {groups.map((b) => (
        <BoardSection key={b.id} board={b} compact={compact} used={used} current={current} full={full} onPick={onPick} />
      ))}
    </>
  )
}

/** The `/` menu's "Chart…" and a chart's Swap: the choices in a dialog, the first match on Enter. */
export function ReportChartPicker({
  open,
  onOpenChange,
  swapping = false,
  current,
  used,
  count,
  onPick,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly swapping?: boolean
  readonly current?: string
  readonly used: ReadonlySet<string>
  readonly count: number
  readonly onPick: (chart: ReportChartChoice) => void
}) {
  const charts = useReportCharts()
  const [query, setQuery] = React.useState("")
  const full = !swapping && count >= MAX_REPORT_CHARTS
  const first = searchBoards(charts.boards, query)
    .flatMap((b) => b.charts)
    .find((c) => c.id !== current)

  React.useEffect(() => {
    if (open) setQuery("")
  }, [open])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="flex max-h-[min(720px,calc(100dvh-2rem))] w-[calc(100vw-2rem)] max-w-3xl flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl"
      >
        <div className="flex shrink-0 items-center gap-2 px-4 pt-4 pb-3">
          <DialogTitle className="text-base font-semibold">{swapping ? "Swap chart" : "Insert a chart"}</DialogTitle>
          <DialogDescription className="sr-only">
            Pick a chart from one of your dashboards. It stays live: each reader sees it with their own access.
          </DialogDescription>
          <span className="ml-auto" />
          <button
            type="button"
            aria-label="Close"
            onClick={() => onOpenChange(false)}
            className={cn("grid size-8 place-items-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground active:scale-[0.97]", EASE, FOCUS)}
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>
        <div className="shrink-0 px-4 pb-3">
          <label className={cn("flex h-9 items-center gap-2 rounded-lg bg-muted/70 px-3 text-sm focus-within:ring-3 focus-within:ring-ring/40 dark:bg-input/30", EASE)}>
            <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <span className="sr-only">Search charts</span>
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search charts and dashboards"
              onKeyDown={(e) => {
                if (e.key === "Enter" && first && !full) {
                  e.preventDefault()
                  onPick(first)
                }
              }}
              className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted-foreground"
            />
          </label>
        </div>
        {full ? (
          <p className="mx-4 mb-3 rounded-lg bg-amber-500/12 px-3 py-2 text-sm text-amber-900 dark:text-amber-200">
            This report has {MAX_REPORT_CHARTS} charts, the most a report can hold. Remove one to add another.
          </p>
        ) : null}
        <div className="rd-fade-y min-h-0 flex-1 overflow-y-auto px-4 pt-1 pb-4">
          <ChartChoices query={query} onClearQuery={() => setQuery("")} used={used} current={current} full={full} onPick={onPick} />
        </div>
        <p className="shrink-0 bg-muted/40 px-4 py-2.5 text-xs text-muted-foreground dark:bg-muted/20">
          {count} of {MAX_REPORT_CHARTS} charts · each reader sees their own data
        </p>
      </DialogContent>
    </Dialog>
  )
}
