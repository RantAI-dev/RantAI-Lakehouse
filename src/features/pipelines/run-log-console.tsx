"use client"

import * as React from "react"
import { CopyIcon, DownloadIcon, SearchIcon, XIcon } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { pipelineService } from "@/services"
import {
  filterLogLines,
  formatLogOffset,
  levelRank,
  logsToText,
  type LogLevelFilter,
  type LogLine,
} from "./run-logs"

/**
 * Pages read on open before the console stops and says the log goes on.
 * The route caps a page at 200 lines (`MAX_LOG_LINES_PER_PAGE`), so this
 * is 5,000 lines: enough for any run here, bounded for a runaway one.
 */
const MAX_PAGES = 25

export type RunLogState = {
  lines: LogLine[]
  loading: boolean
  error: string | null
  /** True when MAX_PAGES were read and the log still had more. */
  truncated: boolean
}

/**
 * Reads a run's whole log by the backend's opaque, monotonic cursor, then
 * keeps polling from the last cursor while the run is live. Pages are only
 * ever appended, never re-requested from zero (WS4 item C4), so a line is
 * never shown twice or dropped. An error keeps the lines already read and
 * retries on the next tick (backing off to 5 s).
 */
export function useRunLogs(pipelineId: string, runId: string, live: boolean): RunLogState {
  const [state, setState] = React.useState<RunLogState>({
    lines: [],
    loading: true,
    error: null,
    truncated: false,
  })
  const liveRef = React.useRef(live)
  liveRef.current = live
  // The loop below stops polling once the run is no longer live. A run
  // that turns live after the log was opened (a retry of it was just
  // launched) restarts the read.
  const wasLive = React.useRef(live)
  const [restart, setRestart] = React.useState(0)
  React.useEffect(() => {
    if (live && !wasLive.current) setRestart((n) => n + 1)
    wasLive.current = live
  }, [live])

  React.useEffect(() => {
    let alive = true
    let cursor: string | undefined
    let timer: ReturnType<typeof setTimeout> | undefined
    const controller = new AbortController()
    setState({ lines: [], loading: true, error: null, truncated: false })

    async function drain() {
      let pages = 0
      try {
        while (alive && pages < MAX_PAGES) {
          const page = await pipelineService.getRunLogs(pipelineId, runId, cursor, controller.signal)
          if (!alive) return
          pages++
          const moved = page.cursor !== cursor
          cursor = page.cursor
          if (page.lines.length > 0) {
            setState((prev) => ({ ...prev, lines: [...prev.lines, ...page.lines] }))
          }
          if (page.lines.length === 0 || !moved) break
        }
        if (!alive) return
        setState((prev) => ({
          ...prev,
          loading: false,
          error: null,
          truncated: pages >= MAX_PAGES && !liveRef.current,
        }))
        if (liveRef.current) timer = setTimeout(drain, 3000)
      } catch (err) {
        if (!alive || controller.signal.aborted) return
        setState((prev) => ({
          ...prev,
          loading: false,
          error: err instanceof Error ? err.message : "the log could not be read",
        }))
        if (liveRef.current) timer = setTimeout(drain, 5000)
      }
    }
    drain()
    return () => {
      alive = false
      controller.abort()
      if (timer) clearTimeout(timer)
    }
  }, [pipelineId, runId, restart])

  return state
}

const LEVELS: { id: LogLevelFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "info", label: "Info+" },
  { id: "warning", label: "Warnings" },
  { id: "error", label: "Errors" },
]

function levelTone(level: string): string {
  const rank = levelRank(level)
  if (rank >= 3) return "text-red-400"
  if (rank === 2) return "text-amber-300"
  if (rank === 1) return "text-sky-300"
  return "text-zinc-500"
}

/**
 * The run's event log as a terminal: offset from the run's start, level,
 * step, message. The panel stays dark in both themes, the way a terminal
 * does, so levels keep the same contrast. Bare lifecycle markers with no
 * text are hidden, and the footer counts them, so the reader knows they
 * exist.
 */
export function RunLogConsole({
  logs,
  originMs,
  level,
  onLevel,
  stepKey,
  onClearStep,
  live,
  fileName,
}: {
  logs: RunLogState
  originMs: number
  level: LogLevelFilter
  onLevel: (level: LogLevelFilter) => void
  stepKey: string | null
  onClearStep: () => void
  live: boolean
  fileName: string
}) {
  const [query, setQuery] = React.useState("")
  const [follow, setFollow] = React.useState(true)
  const scrollRef = React.useRef<HTMLDivElement>(null)

  const shown = React.useMemo(
    () => filterLogLines(logs.lines, { level, stepKey, query }),
    [logs.lines, level, stepKey, query]
  )
  const blank = logs.lines.filter((l) => l.message.trim() === "").length
  const counts = React.useMemo(() => {
    const byFilter: Record<LogLevelFilter, number> = { all: 0, info: 0, warning: 0, error: 0 }
    for (const id of ["all", "info", "warning", "error"] as const) {
      byFilter[id] = filterLogLines(logs.lines, { level: id, stepKey, query: "" }).length
    }
    return byFilter
  }, [logs.lines, stepKey])

  React.useEffect(() => {
    if (!live || !follow) return
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [shown.length, live, follow])

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(logsToText(shown))
      toast.success(`Copied ${shown.length} log lines`)
    } catch {
      toast.error("The browser refused clipboard access")
    }
  }
  const download = () => {
    const blob = new Blob([logsToText(logs.lines)], { type: "text/plain" })
    const url = URL.createObjectURL(blob)
    const a = document.createElement("a")
    a.href = url
    a.download = fileName
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-200 shadow-[0_24px_60px_-40px_rgba(0,0,0,0.6)]">
      <div className="flex flex-wrap items-center gap-2 border-b border-zinc-800 bg-zinc-900/70 px-3 py-2">
        <div className="flex rounded-md bg-zinc-800/80 p-0.5" role="radiogroup" aria-label="Log level">
          {LEVELS.map((l) => (
            <button
              key={l.id}
              type="button"
              role="radio"
              aria-checked={level === l.id}
              onClick={() => onLevel(l.id)}
              className={cn(
                "rounded px-2 py-0.5 text-xs font-medium transition-colors",
                level === l.id ? "bg-zinc-100 text-zinc-900" : "text-zinc-400 hover:text-zinc-100"
              )}
            >
              {l.label}
              <span className={cn("ml-1 font-mono tabular-nums", level === l.id ? "text-zinc-500" : "text-zinc-600")}>
                {counts[l.id]}
              </span>
            </button>
          ))}
        </div>
        {stepKey ? (
          <button
            type="button"
            onClick={onClearStep}
            className="inline-flex items-center gap-1 rounded-md border border-primary/40 bg-primary/15 px-2 py-0.5 font-mono text-xs text-sky-200 hover:bg-primary/25"
          >
            step: {stepKey}
            <XIcon className="size-3" aria-hidden />
            <span className="sr-only">Show every step</span>
          </button>
        ) : null}
        <label className="ml-auto flex h-7 min-w-44 items-center gap-1.5 rounded-md border border-zinc-800 bg-zinc-900 px-2 text-xs focus-within:border-zinc-600">
          <SearchIcon className="size-3.5 text-zinc-500" aria-hidden />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search log"
            aria-label="Search log"
            className="w-full bg-transparent text-zinc-100 outline-none placeholder:text-zinc-600"
          />
        </label>
        {live ? (
          <button
            type="button"
            onClick={() => setFollow((f) => !f)}
            aria-pressed={follow}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs",
              follow ? "text-emerald-300" : "text-zinc-500 hover:text-zinc-200"
            )}
          >
            <span className={cn("size-1.5 rounded-full bg-current", follow && "animate-pulse")} aria-hidden />
            {follow ? "Following" : "Paused"}
          </button>
        ) : null}
        <Button
          size="icon-sm"
          variant="ghost"
          className="text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100"
          onClick={copy}
          disabled={shown.length === 0}
          aria-label="Copy the shown lines"
          title="Copy the shown lines"
        >
          <CopyIcon />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          className="text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100"
          onClick={download}
          disabled={logs.lines.length === 0}
          aria-label="Download the whole log"
          title="Download the whole log"
        >
          <DownloadIcon />
        </Button>
      </div>

      <div
        ref={scrollRef}
        className="max-h-[28rem] min-h-40 overflow-auto py-1.5 font-mono text-[12px] leading-5"
        role="log"
        aria-live={live ? "polite" : "off"}
      >
        {logs.loading && logs.lines.length === 0 ? (
          <p className="px-3 py-2 text-zinc-500">Reading the log…</p>
        ) : shown.length === 0 ? (
          <p className="px-3 py-2 text-zinc-500">
            {logs.lines.length === 0 ? "The orchestrator has recorded no log lines for this run." : "No line matches these filters."}
          </p>
        ) : (
          shown.map((line, i) => {
            const error = levelRank(line.level) >= 3
            return (
              <div
                key={`${line.ts}-${i}`}
                className={cn(
                  "grid grid-cols-[6.5rem_4.25rem_minmax(0,10rem)_1fr] gap-3 border-l-2 px-3 hover:bg-zinc-900",
                  error ? "border-red-500/80 bg-red-500/[0.07]" : "border-transparent"
                )}
              >
                <span className="tabular-nums text-zinc-500" title={new Date(line.ts).toISOString()}>
                  {formatLogOffset(line.ts, originMs)}
                </span>
                <span className={cn("font-semibold", levelTone(line.level))}>{line.level}</span>
                <span className="truncate text-zinc-500" title={line.stepKey ?? "run"}>
                  {line.stepKey ?? "·"}
                </span>
                <span className={cn("whitespace-pre-wrap break-words", error ? "text-red-100" : "text-zinc-200")}>
                  {line.message}
                </span>
              </div>
            )
          })
        )}
      </div>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-zinc-800 px-3 py-1.5 text-[11px] text-zinc-500">
        <span>
          {shown.length} of {logs.lines.length - blank} lines
        </span>
        {blank > 0 ? <span>{blank} empty lifecycle markers hidden</span> : null}
        {logs.truncated ? (
          <span className="text-amber-300">Showing the first {logs.lines.length} lines; the log continues. Download reads only these.</span>
        ) : null}
        {logs.error ? <span className="text-red-400">{logs.error}</span> : null}
      </div>
    </div>
  )
}
