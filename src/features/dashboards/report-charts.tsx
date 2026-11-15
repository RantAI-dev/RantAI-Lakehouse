"use client"

/**
 * The charts the report editor can place, and the data each one is drawn
 * with (`BI-62`, decisions D5 and D6): one provider the chart node, the
 * picker and the Charts tab share, so a chart dragged in from a thumbnail is
 * drawn at once from the data the thumbnail already read.
 *
 * - **Which charts**: `listCharts` (every stored chart the person may see,
 *   with its title and dashboard) and `listBoards` (the dashboards' names and
 *   order). The built-in dashboard has no stored charts to point at, and a
 *   tile without data (text, heading, image…) is not a chart.
 * - **Their data**: the report's own read (`GET /api/dashboard/reports/{id}`)
 *   for the charts already in it, as the reader will see them; for any other
 *   chart, its dashboard's payload, read once per dashboard and only when a
 *   picture of one of its charts is on screen (`loadBoard`). That read goes
 *   through `apiFetch` to `/api/dashboard?board=`, the canvas's own read, as
 *   `dashboard-preview.tsx` does: there is no service method for a board's
 *   payload. It is the governed result (masks, row filters, the board's saved
 *   filters), so a thumbnail shows only what the person may see. Once the
 *   report is saved, the reader reads each chart again as the reader.
 * - Nothing is invented: a chart that cannot be read says so.
 */
import * as React from "react"

import { useService } from "@/hooks/use-service"
import { isStaticKind } from "@/lib/dashboard-tabs"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import { dashboardService } from "@/services"
import type { ReportChartEntry } from "@/services/contracts/dashboard-reports"
import type { TileFailure } from "@/services/contracts/dashboards"
import { apiFetch } from "@/services/http"

export type ReportChartChoice = { id: string; title: string; kind?: string; board: string }
export type ReportChartBoard = { id: string; name: string; charts: ReportChartChoice[] }

type BoardPayload = {
  charts?: (ChartRenderSpec & { text?: string; caption?: string })[]
  results?: Record<string, { columns: string[]; rows: Record<string, unknown>[] } | TileFailure>
}

type LoadState = "loading" | "done" | "error"

export type ReportCharts = {
  /** The catalogue: loading until both lists answer. */
  status: "loading" | "error" | "success"
  /** Dashboards with at least one chart, in the dashboards' own order. */
  boards: ReportChartBoard[]
  /** A chart's choice entry by id. */
  choice: (id: string) => ReportChartChoice | undefined
  /** The data a chart is drawn with, once read. */
  entry: (id: string) => ReportChartEntry | undefined
  /** Whether a dashboard's payload is being read, read, or failed. */
  boardState: (board: string) => LoadState | undefined
  /** Read a dashboard's payload, once. */
  loadBoard: (board: string) => void
  /** Read what `id` needs to be drawn (its dashboard's payload). */
  ensure: (id: string) => void
}

const ReportChartsContext = React.createContext<ReportCharts | null>(null)

/** The sentence for a chart whose dashboard could not be read. */
export const CHART_NOT_READ = "This chart could not be drawn right now."

export function ReportChartsProvider({
  seed,
  children,
}: {
  /** The report's own read of the charts it holds (`ReportView.charts`). */
  readonly seed?: Readonly<Record<string, ReportChartEntry>>
  readonly children: React.ReactNode
}) {
  const boardsState = useService((s) => dashboardService.listBoards(s), [])
  const chartsState = useService((s) => dashboardService.listCharts(s), [])
  const [loaded, setLoaded] = React.useState<Record<string, ReportChartEntry>>({})
  const [states, setStates] = React.useState<Record<string, LoadState>>({})
  const started = React.useRef(new Set<string>())
  const aborts = React.useRef(new Set<AbortController>())

  React.useEffect(() => {
    const live = aborts.current
    return () => {
      for (const a of live) a.abort()
    }
  }, [])

  const boards = React.useMemo<ReportChartBoard[]>(() => {
    if (boardsState.status !== "success" || chartsState.status !== "success") return []
    const charts = chartsState.data.filter((c) => !c.kind || !isStaticKind(c.kind))
    return boardsState.data
      .filter((b) => !b.builtin)
      .map((b) => ({ id: b.id, name: b.name, charts: charts.filter((c) => c.board === b.id) }))
      .filter((b) => b.charts.length > 0)
  }, [boardsState, chartsState])

  const byId = React.useMemo(() => {
    const map = new Map<string, ReportChartChoice>()
    for (const b of boards) for (const c of b.charts) map.set(c.id, c)
    return map
  }, [boards])
  const names = React.useMemo(() => new Map(boards.map((b) => [b.id, b.name])), [boards])

  const loadBoard = React.useCallback(
    (board: string) => {
      if (!board || started.current.has(board)) return
      started.current.add(board)
      setStates((s) => ({ ...s, [board]: "loading" }))
      const ctrl = new AbortController()
      aborts.current.add(ctrl)
      void (async () => {
        try {
          // Deliberately `apiFetch` from a feature (see the module comment): the canvas's own read.
          const res = await apiFetch(`/api/dashboard?${new URLSearchParams({ board })}`, { cache: "no-store", signal: ctrl.signal })
          if (!res.ok) throw new Error("board payload refused")
          const payload = (await res.json()) as BoardPayload
          const boardName = names.get(board) ?? ""
          const next: Record<string, ReportChartEntry> = {}
          for (const spec of payload.charts ?? []) {
            if (isStaticKind(spec.kind)) continue
            next[spec.id] = { spec, cell: payload.results?.[spec.id] ?? null, board, boardName }
          }
          setLoaded((l) => ({ ...l, ...next }))
          setStates((s) => ({ ...s, [board]: "done" }))
        } catch {
          if (ctrl.signal.aborted) return
          // The route's text is never shown; the chart says it could not be drawn.
          setStates((s) => ({ ...s, [board]: "error" }))
        } finally {
          aborts.current.delete(ctrl)
        }
      })()
    },
    [names],
  )

  const value = React.useMemo<ReportCharts>(() => {
    const status =
      boardsState.status === "error" || chartsState.status === "error"
        ? "error"
        : boardsState.status === "success" && chartsState.status === "success"
          ? "success"
          : "loading"
    return {
      status,
      boards,
      choice: (id) => byId.get(id),
      // The report's own read wins: it is what the reader sees.
      entry: (id) => seed?.[id] ?? loaded[id],
      boardState: (board) => states[board],
      loadBoard,
      ensure: (id) => {
        if (seed?.[id] || loaded[id]) return
        const c = byId.get(id)
        if (c) loadBoard(c.board)
      },
    }
  }, [boardsState.status, chartsState.status, boards, byId, seed, loaded, states, loadBoard])

  return <ReportChartsContext.Provider value={value}>{children}</ReportChartsContext.Provider>
}

const EMPTY: ReportCharts = {
  status: "loading",
  boards: [],
  choice: () => undefined,
  entry: () => undefined,
  boardState: () => undefined,
  loadBoard: () => {},
  ensure: () => {},
}

export function useReportCharts(): ReportCharts {
  return React.useContext(ReportChartsContext) ?? EMPTY
}

/** What a chart in the document shows while it has no data: loading, or why not. */
export function chartWaiting(charts: ReportCharts, id: string): "loading" | { sentence: string } {
  if (charts.status === "loading") return "loading"
  const c = charts.choice(id)
  if (!c) return { sentence: "This chart is not available." }
  const s = charts.boardState(c.board)
  if (s === "error") return { sentence: CHART_NOT_READ }
  return "loading"
}

/** Search the catalogue by chart title or dashboard name. */
export function searchBoards(boards: readonly ReportChartBoard[], query: string): ReportChartBoard[] {
  const q = query.trim().toLowerCase()
  if (!q) return [...boards]
  return boards
    .map((b) => (b.name.toLowerCase().includes(q) ? b : { ...b, charts: b.charts.filter((c) => c.title.toLowerCase().includes(q)) }))
    .filter((b) => b.charts.length > 0)
}
