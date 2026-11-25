"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { EllipsisIcon, ExternalLinkIcon, ScrollTextIcon } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { useService } from "@/hooks/use-service"
import { formatDateTime, formatDuration } from "@/lib/format"
import {
  EMPTY_HISTORY_ADDRESS,
  HISTORY_PAGE_LIMIT,
  historyAddress,
  historyQueryString,
  readHistoryAddress,
  type HistoryAddress,
} from "@/lib/history-query-params"
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

/**
 * QS-12 (LH-QS-4-07), D15: a row opens its run by id. The SQL stays in the
 * browser, not in the address; the old `?sql=` handoff belongs to the
 * catalog's starter-query links, which carry a query rather than a person's
 * run.
 */
function studioHref(run: QueryHistoryItem): string {
  return `/query-studio?run=${encodeURIComponent(run.id)}`
}

/** Where the filters come from on first render (D17), guarded for SSR. */
function initialAddress(): HistoryAddress {
  return typeof window === "undefined" ? EMPTY_HISTORY_ADDRESS : readHistoryAddress(window.location.search)
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

/** The row menu of one run: open it in the Studio, and its audit event when it has one. */
function RunActions({ run }: { run: QueryHistoryItem }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="size-7"
            aria-label={`Actions for the run at ${formatDateTime(run.at)}`}
          />
        }
      >
        <EllipsisIcon className="size-4" aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem render={<Link href={studioHref(run)} />}>
          <ExternalLinkIcon className="size-3.5" data-icon="inline-start" />
          Open in Studio
        </DropdownMenuItem>
        {run.auditEventId ? (
          <DropdownMenuItem render={<Link href={`/audit?event=${run.auditEventId}`} />}>
            <ScrollTextIcon className="size-3.5" data-icon="inline-start" />
            View in Audit
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * Every run this caller made in the last 14 days, filtered, newest first
 * (`QS-4`, D1/D2/D5/D6). The server owns the window and the page size;
 * the page numbers walk its 1-based `page` (QS-2/QS-4 QA of 2026-11-05).
 * A row's SQL opens a new Query Studio tab with the engine it ran on.
 */
export function QueryHistoryPage() {
  const router = useRouter()
  // QS-12 (LH-QS-4-06), D17: the address is the source of truth on entry.
  const initial = React.useMemo(() => initialAddress(), [])
  const [status, setStatus] = React.useState<QueryHistoryItem["status"] | "all">(
    initial.status as QueryHistoryItem["status"] | "all"
  )
  const [engine, setEngine] = React.useState<string>(initial.engine)
  const [minDuration, setMinDuration] = React.useState(initial.minDuration)
  const [table, setTable] = React.useState(initial.table)
  const [from, setFrom] = React.useState(initial.from)
  const [to, setTo] = React.useState(initial.to)

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
  const [pageAt, setPageAt] = React.useState({ query, page: initial.page })
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

  // QS-12 (LH-QS-4-06), D17: every filter change is written back with
  // replaceState, so the back button is not filled with one entry per
  // keystroke. Read from window.location on entry, never useSearchParams, so
  // the page needs no Suspense boundary.
  React.useEffect(() => {
    if (typeof window === "undefined") return
    const address = historyAddress({ status, engine, minDuration, table, from, to, page })
    window.history.replaceState(
      null,
      "",
      `${window.location.pathname}${address ? `?${address}` : ""}`
    )
  }, [status, engine, minDuration, table, from, to, page])

  const clearFilters = React.useCallback(() => {
    setStatus("all")
    setEngine("all")
    setMinDuration("")
    setTable("")
    setFrom("")
    setTo("")
  }, [])

  const hasFilter = query !== ""

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Query History"
        description="Every run of yours that reached an answer, in the last 14 days."
        // UX-9: the sections are the header's own control now.
        actions={<QueryStudioTabs />}
      />
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
            onClick={clearFilters}
          >
            Clear filters
          </Button>
        ) : null}
      </div>
      {state.status === "loading" ? <LoadingSkeleton rows={6} /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && !pastTheEnd ? (
        rows.length === 0 ? (
          // QS-12 (LH-QS-4-06), D18: when a filter emptied the list, blame the
          // filter, not the 14-day window, and offer to clear it.
          <EmptyState
            title={hasFilter ? "No runs match these filters" : "No runs in the last 14 days"}
            description={
              hasFilter
                ? "Clear a filter and try again."
                : "Run a query in the Studio and it lands here."
            }
            action={
              hasFilter ? (
                <Button size="sm" variant="outline" onClick={clearFilters}>
                  Clear filters
                </Button>
              ) : (
                <Button size="sm" render={<Link href="/query-studio" />}>
                  Open the Studio
                </Button>
              )
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
                  <th className="w-10 px-1 py-2">
                    <span className="sr-only">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y">
                {rows.map((run) => (
                  <tr
                    key={run.id}
                    className="cursor-pointer align-top hover:bg-muted/20"
                    // QS-12 (LH-QS-4-07), D16: the whole row opens the run by
                    // its id. A click that ended a text selection is not a
                    // navigation. QS round review R2: a click on the SQL link
                    // is the browser's own navigation (often "open in a new
                    // tab"), so the row must not push as well — nor a click
                    // someone else already handled or made with a modifier.
                    onClick={(event) => {
                      if (window.getSelection()?.toString()) return
                      if (event.defaultPrevented) return
                      if (event.button !== 0) return
                      if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
                      if ((event.target as HTMLElement).closest("a")) return
                      // UX-9: the row menu renders in a portal, and React
                      // bubbles its clicks here; a click that did not land
                      // in the row's own cells is not the row's.
                      if (!event.currentTarget.contains(event.target as Node)) return
                      if ((event.target as HTMLElement).closest("button")) return
                      router.push(studioHref(run))
                    }}
                  >
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
                        className="line-clamp-2 font-mono text-xs text-primary underline-offset-4 hover:underline"
                        title={run.sql}
                      >
                        {run.sql}
                      </Link>
                      {run.message ? (
                        <p className="mt-1 text-xs text-destructive">{run.message}</p>
                      ) : null}
                    </td>
                    {/* UX-9: the Studio's History card is gone; its per-run
                        "Audit" link lives in this row menu, beside the way
                        back into the Studio the row click already is. */}
                    <td className="px-1 py-1.5 text-right">
                      <RunActions run={run} />
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
