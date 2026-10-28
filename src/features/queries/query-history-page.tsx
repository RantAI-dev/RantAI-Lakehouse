"use client"

import * as React from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDateTime, formatDuration } from "@/lib/format"
import { HISTORY_PAGE_LIMIT, historyQueryString } from "@/lib/history-query-params"
import { cn } from "@/lib/utils"
import { queryService } from "@/services"
import type { QueryHistoryFilter, QueryHistoryItem } from "@/services/contracts/queries"
import { QueryStudioTabs } from "./query-studio-tabs"

const STATUSES = ["completed", "failed", "cancelled", "blocked"] as const
const ENGINES = ["clickhouse", "trino"] as const

/** The studio link every row carries: the SQL and its engine (QS-4, D1). */
function studioHref(run: QueryHistoryItem): string {
  return `/query-studio?sql=${encodeURIComponent(run.sql)}&engine=${encodeURIComponent(run.engine)}`
}

/** A date input's `YYYY-MM-DD` as the day's first (or last) UTC instant —
 * the RFC 3339 the server parses. A date means its UTC day. */
function dayBound(date: string, edge: "start" | "end"): string {
  return edge === "start" ? `${date}T00:00:00Z` : `${date}T23:59:59.999Z`
}

/** One pill of the status / engine bars. A native button, pressed or not. */
function Pill({
  pressed,
  onClick,
  children,
}: {
  pressed: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      onClick={onClick}
      className={cn(
        "rounded-md border px-2 py-0.5 text-xs transition-colors",
        pressed
          ? "border-primary bg-primary/10 text-primary"
          : "border-transparent text-muted-foreground hover:bg-muted/40 hover:text-foreground",
      )}
    >
      {children}
    </button>
  )
}

/**
 * Every run this caller made in the last 14 days, filtered, newest first
 * (`QS-4`, D1/D2/D5/D6). The server owns the window and the page size;
 * "Load more" walks backwards with the cursor. A row's SQL opens a new
 * Query Studio tab with the engine it ran on.
 */
export function QueryHistoryPage() {
  const [status, setStatus] = React.useState<QueryHistoryItem["status"] | "all">("all")
  const [engine, setEngine] = React.useState<string>("all")
  const [minDuration, setMinDuration] = React.useState("")
  const [table, setTable] = React.useState("")
  const [from, setFrom] = React.useState("")
  const [to, setTo] = React.useState("")

  const filter = React.useMemo<QueryHistoryFilter>(
    () => ({
      status: status === "all" ? undefined : status,
      engine: engine === "all" ? undefined : engine,
      minDurationMs: minDuration === "" ? undefined : Number(minDuration),
      table: table === "" ? undefined : table,
      from: from === "" ? undefined : dayBound(from, "start"),
      to: to === "" ? undefined : dayBound(to, "end"),
    }),
    [status, engine, minDuration, table, from, to],
  )
  const query = historyQueryString(filter)

  const state = useService((s) => queryService.listHistory(filter, s), [query])

  // "Load more" appends the page before the oldest row shown. A new
  // filter is a new question, so what an older question appended is
  // dropped when the query string changes.
  const [older, setOlder] = React.useState<QueryHistoryItem[]>([])
  // Whether the newest answer still promises an older page: only a page
  // of HISTORY_PAGE_LIMIT rows does. A shorter page — including the
  // first — was the last one, and the button stays hidden.
  const [lastPageFull, setLastPageFull] = React.useState(false)

  // The older page is a call the person asks for, so it goes through the
  // action hook: the button reads its pending state, and a refusal
  // carries the server's sentence.
  const loadMoreAction = useServiceAction(
    (signal, nextFilter: QueryHistoryFilter, before: { at: string; id: string }) =>
      queryService.listHistory({ ...nextFilter, before }, signal),
  )
  const { reset: resetLoadMore } = loadMoreAction

  // The question the older page was asked for. The answer arrives after
  // the render that asked for it, so the question is kept in a ref; it is
  // written outside the render, never during it.
  const queryRef = React.useRef(query)

  React.useEffect(() => {
    // The question changed: what the older question appended is dropped,
    // the refusal that belonged to it goes with it, and the ref remembers
    // the new question for the guard in loadMore.
    queryRef.current = query
    setOlder([])
    setLastPageFull(false)
    resetLoadMore()
  }, [query, resetLoadMore])

  // The first page lands through the hook, not through loadMore, so its
  // fullness is read here.
  React.useEffect(() => {
    if (state.status === "success") {
      setLastPageFull(state.data.length >= HISTORY_PAGE_LIMIT)
    }
  }, [state])

  const rows = React.useMemo(
    () => [...(state.status === "success" ? state.data : []), ...older],
    [state.status, state.data, older],
  )

  async function loadMore() {
    const last = rows[rows.length - 1]
    if (!last || loadMoreAction.status === "pending") return
    const forQuery = queryRef.current
    // The hook owns the call: it aborts it, holds the refusal, and hands
    // back the page. A failure resolves to null and keeps the rows.
    const next = await loadMoreAction.run(filter, { at: last.at, id: last.id })
    if (next === null) return
    if (queryRef.current !== forQuery) return // the question changed under us
    const seen = new Set(rows.map((r) => r.id))
    setOlder((prev) => [...prev, ...next.filter((r) => !seen.has(r.id))])
    setLastPageFull(next.length >= HISTORY_PAGE_LIMIT)
  }

  const hasFilter = query !== ""

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Query History"
        description="Every run of yours that reached an answer, in the last 14 days."
      />
      <QueryStudioTabs />
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-md border bg-card px-3 py-2">
        <div className="flex items-center gap-1" role="group" aria-label="Filter by status">
          <Pill pressed={status === "all"} onClick={() => setStatus("all")}>
            All statuses
          </Pill>
          {STATUSES.map((s) => (
            <Pill key={s} pressed={status === s} onClick={() => setStatus(s)}>
              {s[0]!.toUpperCase() + s.slice(1)}
            </Pill>
          ))}
        </div>
        <div className="flex items-center gap-1" role="group" aria-label="Filter by engine">
          <Pill pressed={engine === "all"} onClick={() => setEngine("all")}>
            All engines
          </Pill>
          {ENGINES.map((e) => (
            <Pill key={e} pressed={engine === e} onClick={() => setEngine(e)}>
              {e === "trino" ? "Trino" : "ClickHouse"}
            </Pill>
          ))}
        </div>
        <Input
          type="number"
          min={0}
          placeholder="Min duration (ms)"
          aria-label="Minimum duration in milliseconds"
          value={minDuration}
          onChange={(e) => setMinDuration(e.target.value)}
          className="h-7 w-36 text-xs"
        />
        <Input
          type="text"
          placeholder="Table contains…"
          aria-label="Table contains"
          value={table}
          onChange={(e) => setTable(e.target.value)}
          className="h-7 w-44 text-xs"
        />
        <label className="flex items-center gap-1 text-xs text-muted-foreground">
          From
          <Input
            type="date"
            aria-label="From date"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            className="h-7 w-36 text-xs"
          />
        </label>
        <label className="flex items-center gap-1 text-xs text-muted-foreground">
          To
          <Input
            type="date"
            aria-label="To date"
            value={to}
            onChange={(e) => setTo(e.target.value)}
            className="h-7 w-36 text-xs"
          />
        </label>
        {hasFilter ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs"
            onClick={() => {
              setStatus("all")
              setEngine("all")
              setMinDuration("")
              setTable("")
              setFrom("")
              setTo("")
            }}
          >
            Clear filters
          </Button>
        ) : null}
      </div>
      {state.status === "loading" ? <LoadingSkeleton rows={6} /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        rows.length === 0 ? (
          <EmptyState
            title="No runs in the last 14 days"
            description={
              hasFilter
                ? "Nothing matches these filters. Clear one and try again."
                : "Run a query in the Studio and it lands here."
            }
            action={
              <Button size="sm" render={<Link href="/query-studio" />}>
                Open the Studio
              </Button>
            }
          />
        ) : (
          <div className="overflow-hidden rounded-md border">
            <table className="w-full text-sm">
              <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
                <tr>
                  <th className="px-3 py-2 font-medium">Time</th>
                  <th className="px-3 py-2 font-medium">Status</th>
                  <th className="px-3 py-2 font-medium">Engine</th>
                  <th className="px-3 py-2 font-medium">Duration</th>
                  <th className="px-3 py-2 font-medium">Tables</th>
                  <th className="px-3 py-2 font-medium">SQL</th>
                </tr>
              </thead>
              <tbody className="divide-y">
                {rows.map((run) => (
                  <tr key={run.id} className="align-top hover:bg-muted/20">
                    <td className="px-3 py-2 text-xs whitespace-nowrap text-muted-foreground" title={formatDateTime(run.at)}>
                      {formatDateTime(run.at)}
                    </td>
                    <td className="px-3 py-2">
                      <StatusBadge status={run.status} />
                    </td>
                    <td className="px-3 py-2 text-xs">{run.engine}</td>
                    <td className="px-3 py-2 text-xs whitespace-nowrap">
                      {formatDuration(run.durationMs)}
                    </td>
                    <td className="max-w-48 px-3 py-2 text-xs text-muted-foreground">
                      {run.tablesRead.length === 0 ? (
                        <span title="This run recorded no table list.">
                          —
                        </span>
                      ) : (
                        <span className="line-clamp-2" title={run.tablesRead.join(", ")}>
                          {run.tablesRead.join(", ")}
                        </span>
                      )}
                    </td>
                    <td className="max-w-96 px-3 py-2">
                      <Link
                        href={studioHref(run)}
                        className="line-clamp-2 font-mono text-xs hover:underline"
                        title={run.sql}
                      >
                        {run.sql}
                      </Link>
                      {run.message ? (
                        <p className="mt-1 text-xs text-destructive">{run.message}</p>
                      ) : null}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : null}
      {loadMoreAction.status === "error" ? (
        <p className="text-sm text-destructive">{loadMoreAction.error.message}</p>
      ) : null}
      {state.status === "success" && rows.length > 0 && lastPageFull ? (
        <Button
          variant="outline"
          size="sm"
          className="self-center"
          disabled={loadMoreAction.status === "pending"}
          onClick={() => {
            void loadMore()
          }}
        >
          {loadMoreAction.status === "pending" ? "Loading…" : "Load more"}
        </Button>
      ) : null}
    </div>
  )
}
