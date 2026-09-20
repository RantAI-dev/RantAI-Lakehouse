"use client"

import * as React from "react"
import Link from "next/link"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { type ServiceState } from "@/hooks/use-service"
import { formatDateTime, formatDuration, formatRelativeTime } from "@/lib/format"
import type { QueryHistoryItem, SavedQuery } from "@/services/contracts/queries"

/** How many entries each rail list shows before "Show more". */
const PAGE_SIZE = 6

type SavedListState = ServiceState<SavedQuery[]> & { reload: () => void }
type HistoryListState = ServiceState<QueryHistoryItem[]> & { reload: () => void }

/** Compact "Saved" quick list. Clicking an item loads its SQL in the editor. */
export function SavedQuickList({
  state,
  onLoadSql,
}: {
  state: SavedListState
  onLoadSql: (sql: string) => void
}) {
  return (
    <SectionCard
      title="Saved"
      action={
        <Link
          href="/query-studio/saved"
          className="text-xs text-primary hover:underline"
        >
          View all
        </Link>
      }
    >
      {state.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        state.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No saved queries yet. Save one from the SQL tab.
          </p>
        ) : (
          <ul className="divide-y divide-border">
            {state.data.slice(0, PAGE_SIZE).map((q) => (
              <li key={q.id}>
                <button
                  type="button"
                  onClick={() => onLoadSql(q.sql)}
                  className="-mx-2 flex w-full items-baseline justify-between gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-muted/40"
                  title={q.sql}
                >
                  <span className="truncate font-medium">{q.title}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {formatRelativeTime(q.updatedAt)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )
      ) : null}
    </SectionCard>
  )
}

/**
 * The runs this user has made, newest first. Clicking one loads its SQL.
 *
 * History is per-user on the server, so this is your own trail — not the
 * whole team's, which is what it used to show.
 */
export function HistoryQuickList({
  state,
  onLoadSql,
}: {
  // Owned by the page's hook rather than by this component, so a finished
  // run can refresh it.
  state: HistoryListState
  onLoadSql: (sql: string) => void
}) {
  const [expanded, setExpanded] = React.useState(false)
  const all = state.status === "success" ? state.data : []
  const shown = expanded ? all.slice(0, PAGE_SIZE * 4) : all.slice(0, PAGE_SIZE)

  return (
    <SectionCard title="History" description="Your recent runs.">
      {state.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        all.length === 0 ? (
          <p className="text-sm text-muted-foreground">No queries run yet.</p>
        ) : (
          <>
            <ul className="divide-y divide-border">
              {shown.map((h) => (
                <li key={h.id} className="py-1.5">
                  <button
                    type="button"
                    onClick={() => onLoadSql(h.sql)}
                    className="-mx-2 flex w-full flex-col gap-1 rounded-md px-2 py-1 text-left transition-colors hover:bg-muted/40"
                    title={h.sql}
                  >
                    <span className="flex items-center gap-2 text-xs text-muted-foreground">
                      <StatusBadge status={h.status} />
                      <span title={formatDateTime(h.at)}>
                        {formatRelativeTime(h.at)}
                      </span>
                      {h.status === "completed" ? (
                        <>
                          <span>·</span>
                          <span>{formatDuration(h.durationMs)}</span>
                        </>
                      ) : null}
                    </span>
                    {/* Two lines, not one: a truncated single line of SQL
                        rarely reaches the part that identifies it. */}
                    <span className="line-clamp-2 font-mono text-xs text-foreground">
                      {h.sql}
                    </span>
                  </button>
                  {h.auditEventId ? (
                    <Link
                      href={`/audit?event=${h.auditEventId}`}
                      className="ml-0 inline-block text-xs text-primary hover:underline"
                    >
                      Audit
                    </Link>
                  ) : null}
                </li>
              ))}
            </ul>
            {all.length > PAGE_SIZE ? (
              <Button
                variant="ghost"
                size="sm"
                className="mt-1 w-full"
                onClick={() => setExpanded((v) => !v)}
              >
                {expanded ? "Show less" : `Show more (${all.length - PAGE_SIZE})`}
              </Button>
            ) : null}
          </>
        )
      ) : null}
    </SectionCard>
  )
}
