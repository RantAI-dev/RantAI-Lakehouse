"use client"

import * as React from "react"
import { BarChart3 } from "lucide-react"

import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useService } from "@/hooks/use-service"
import { cn } from "@/lib/utils"
import { dashboardService } from "@/services"

/** The tile kinds that carry no data, so cannot be a report's chart (`ChartKind::is_static`). */
const STATIC_KINDS = new Set(["text", "heading", "divider", "section", "image", "web"])

export type PickedChart = { chart: string; board: string; title: string; boardName: string }

/**
 * Pick a chart from an existing dashboard for a report's chart block
 * (`BI-18` part D, T5, decision R1): a dashboard, then one of its charts.
 * Only dashboards the person may see are listed, and only their data charts
 * (a text or section tile is not a chart); the server checks both again on
 * save.
 */
export function ReportChartPicker({
  open,
  onOpenChange,
  onPick,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly onPick: (picked: PickedChart) => void
}) {
  const boards = useService((s) => dashboardService.listBoards(s), [])
  const [board, setBoard] = React.useState("")
  const [chart, setChart] = React.useState("")
  React.useEffect(() => {
    if (open) {
      setBoard("")
      setChart("")
    }
  }, [open])
  // The built-in dashboard has no stored charts to point at.
  const choices = React.useMemo(
    () => (boards.status === "success" ? boards.data.filter((b) => !b.builtin) : []),
    [boards],
  )
  const charts = useService(
    (s) => (board ? dashboardService.listBoardCharts(board, s) : Promise.resolve([])),
    [board],
  )
  const dataCharts = charts.status === "success" ? charts.data.filter((c) => !c.kind || !STATIC_KINDS.has(c.kind)) : []
  const boardName = choices.find((b) => b.id === board)?.name ?? ""
  const picked = dataCharts.find((c) => c.id === chart)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Add a chart</DialogTitle>
          <DialogDescription>
            The chart stays live: each reader sees it with their own access, as on its dashboard.
          </DialogDescription>
        </DialogHeader>
        {boards.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
        {boards.status === "error" ? <ErrorState error={boards.error} onRetry={boards.reload} /> : null}
        {boards.status === "success" && choices.length === 0 ? (
          <EmptyState title="No dashboards to pick from" description="Make a dashboard with a chart first." />
        ) : null}
        {choices.length > 0 ? (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-1.5">
              <Label>Dashboard</Label>
              <Select value={board || null} onValueChange={(v) => { setBoard(String(v ?? "")); setChart("") }}>
                <SelectTrigger className="w-full" aria-label="Dashboard">
                  <SelectValue>{boardName || "Choose a dashboard"}</SelectValue>
                </SelectTrigger>
                <SelectContent>
                  {choices.map((b) => (
                    <SelectItem key={b.id} value={b.id}>
                      {b.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {board ? (
              <div className="flex flex-col gap-1.5">
                <Label id="report-chart-list">Chart</Label>
                {charts.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
                {charts.status === "error" ? <ErrorState error={charts.error} onRetry={charts.reload} /> : null}
                {charts.status === "success" && dataCharts.length === 0 ? (
                  <p className="rounded-md border border-dashed px-3 py-4 text-center text-sm text-muted-foreground">
                    This dashboard has no charts.
                  </p>
                ) : null}
                {dataCharts.length > 0 ? (
                  <ul role="listbox" aria-labelledby="report-chart-list" className="max-h-64 overflow-y-auto rounded-md border p-1">
                    {dataCharts.map((c) => (
                      <li key={c.id} role="option" aria-selected={c.id === chart}>
                        <button
                          type="button"
                          onClick={() => setChart(c.id)}
                          className={cn(
                            "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                            c.id === chart && "bg-primary/10 text-primary",
                          )}
                        >
                          <BarChart3 className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                          <span className="truncate">{c.title}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                ) : null}
              </div>
            ) : null}
          </div>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={!picked}
            onClick={() => {
              if (!picked) return
              onPick({ chart: picked.id, board, title: picked.title, boardName })
              onOpenChange(false)
            }}
          >
            Add chart
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
