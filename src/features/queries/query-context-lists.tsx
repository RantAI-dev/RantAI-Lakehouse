"use client"

import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { StatusBadge } from "@/components/patterns/status-badge"
import { useService, type ServiceState } from "@/hooks/use-service"
import { formatDuration, formatRelativeTime } from "@/lib/format"
import { queryService } from "@/services"
import type { SavedQuery } from "@/services/contracts/queries"

const MAX_ITEMS = 6

type SavedListState = ServiceState<SavedQuery[]> & { reload: () => void }

/** Compact "Saved" quick list. Clicking an item loads its SQL in the editor. */
export function SavedQuickList({
  state,
  onLoadSql,
}: {
  state: SavedListState
  onLoadSql: (sql: string) => void
}) {
  return (
    <SectionCard title="Saved">
      {state.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        state.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">No saved queries yet.</p>
        ) : (
          <ul className="space-y-1">
            {state.data.slice(0, MAX_ITEMS).map((q) => (
              <li key={q.id}>
                <button
                  type="button"
                  onClick={() => onLoadSql(q.sql)}
                  className="flex w-full items-baseline justify-between gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted"
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

/** Compact history list. Clicking an item loads its SQL in the editor. */
export function HistoryQuickList({
  onLoadSql,
}: {
  onLoadSql: (sql: string) => void
}) {
  const state = useService((s) => queryService.listHistory(s), [])
  return (
    <SectionCard title="History">
      {state.status === "loading" ? <LoadingSkeleton rows={3} /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        state.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">No queries run yet.</p>
        ) : (
          <ul className="space-y-1">
            {state.data.slice(0, MAX_ITEMS).map((h) => (
              <li key={h.id}>
                <button
                  type="button"
                  onClick={() => onLoadSql(h.sql)}
                  className="flex w-full flex-col gap-1 rounded-md px-2 py-1.5 text-left hover:bg-muted"
                >
                  <span className="flex items-center gap-2 text-xs text-muted-foreground">
                    <StatusBadge status={h.status} />
                    <span>{formatRelativeTime(h.at)}</span>
                    <span>·</span>
                    <span>{formatDuration(h.durationMs)}</span>
                  </span>
                  <span className="truncate font-mono text-xs">{h.sql}</span>
                </button>
              </li>
            ))}
          </ul>
        )
      ) : null}
    </SectionCard>
  )
}
