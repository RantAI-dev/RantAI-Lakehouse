"use client"

import * as React from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useService } from "@/hooks/use-service"
import { formatDateTime, formatDuration } from "@/lib/format"
import { HISTORY_PAGE_LIMIT, historyQueryString } from "@/lib/history-query-params"
import { queryEngineLabel } from "@/lib/status"
import { cn } from "@/lib/utils"
import { queryService } from "@/services"
import type { QueryHistoryFilter, QueryHistoryItem } from "@/services/contracts/queries"
import { QueryStudioTabs } from "./query-studio-tabs"

/**
 * The status pills: the word the person reads and the value the server
 * filters on. A run stopped with Stop is `stopped`; a policy refusal is
 * stored as `blocked` and reads "Refused" here (`query-history.md`
 * decision 3). `cancelled` is not a pill: nothing writes it any more.
 */
const STATUSES = [
  { value: "completed", label: "Completed" },
  { value: "failed", label: "Failed" },
  { value: "stopped", label: "Stopped" },
  { value: "blocked", label: "Refused" },
] as const
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

/**
 * The page numbers to draw: the first, the last, and the pages next to
 * the current one, with `"gap"` where numbers are left out. Seven pages
 * or fewer are all drawn.
 */
function pageNumbers(current: number, last: number): (number | "gap")[] {
  const shown = new Set([1, last, current - 1, current, current + 1])
  const numbers = [...shown].filter((n) => n >= 1 && n <= last).sort((a, b) => a - b)
  const out: (number | "gap")[] = []
  numbers.forEach((n, i) => {
    if (i > 0 && n - numbers[i - 1]! > 1) out.push("gap")
    out.push(n)
  })
  return out
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
 * the page numbers walk its 1-based `page` (QS-2/QS-4 QA of 2026-11-05).
 * A row's SQL opens a new Query Studio tab with the engine it ran on.
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

  // The page number belongs to the question it was chosen under. A new
  // filter is a new question, so it starts on page 1: the stored page is
  // reset while rendering (React re-renders at once, with no frame that
  // asks for the old page under the new filter).
  const [pageAt, setPageAt] = React.useState({ query, page: 1 })
  if (pageAt.query !== query) setPageAt({ query, page: 1 })
  const page = pageAt.query === query ? pageAt.page : 1
  const goToPage = (next: number) => setPageAt({ query, page: next })

  const state = useService(
    (s) => queryService.listHistoryPage({ ...filter, page }, s),
    [query, page],
  )

  const rows = state.status === "success" ? state.data.runs : []
  const pageSize = state.status === "success" ? state.data.pageSize : HISTORY_PAGE_LIMIT
  const total = state.status === "success" ? state.data.total : 0
  const lastPage = Math.max(1, Math.ceil(total / pageSize))

  // The data shrank while the person was on a later page: it answers no
  // runs for a page that is gone, but still counts some. Step back to the
  // last page that exists instead of showing an empty table.
  const pastTheEnd = state.status === "success" && rows.length === 0 && total > 0 && page > lastPage
  React.useEffect(() => {
    if (pastTheEnd) setPageAt({ query, page: lastPage })
  }, [pastTheEnd, query, lastPage])

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
            <Pill key={s.value} pressed={status === s.value} onClick={() => setStatus(s.value)}>
              {s.label}
            </Pill>
          ))}
        </div>
        <div className="flex items-center gap-1" role="group" aria-label="Filter by engine">
          <Pill pressed={engine === "all"} onClick={() => setEngine("all")}>
            All engines
          </Pill>
          {ENGINES.map((e) => (
            <Pill key={e} pressed={engine === e} onClick={() => setEngine(e)}>
              {queryEngineLabel(e)}
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
      {state.status === "success" && !pastTheEnd ? (
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
                      <StatusBadge status={run.status} label={run.status === "blocked" ? "Refused" : undefined} />
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
      {state.status === "success" && rows.length > 0 ? (
        <nav aria-label="History pages" className="flex items-center justify-center gap-1">
          <Button
            variant="outline"
            size="sm"
            disabled={page <= 1}
            onClick={() => goToPage(page - 1)}
          >
            Previous
          </Button>
          {pageNumbers(page, lastPage).map((n, i) =>
            n === "gap" ? (
              <span key={`gap-${i}`} className="px-1 text-xs text-muted-foreground" aria-hidden>
                …
              </span>
            ) : (
              <Button
                key={n}
                variant={n === page ? "default" : "ghost"}
                size="sm"
                aria-current={n === page ? "page" : undefined}
                onClick={() => goToPage(n)}
              >
                {n}
              </Button>
            ),
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={page >= lastPage}
            onClick={() => goToPage(page + 1)}
          >
            Next
          </Button>
          <span className="ml-2 text-xs text-muted-foreground">
            Page {page} of {lastPage}
          </span>
        </nav>
      ) : null}
    </div>
  )
}
