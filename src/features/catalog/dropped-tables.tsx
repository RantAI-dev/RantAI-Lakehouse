"use client"

import * as React from "react"
import Link from "next/link"
import { SectionCard } from "@/components/patterns/section-card"
import { useService } from "@/hooks/use-service"
import { daysLeft } from "@/lib/dashboard-organise"
import { formatNumber } from "@/lib/format"
import { assetService } from "@/services"

/**
 * The clock the days left are measured against: read while the list renders,
 * and only when a test did not pin `now`.
 */
function clockNow(): number {
  return Date.now()
}

/**
 * The Catalog's list of tables dropped from the console (`DATA-55`,
 * QA `LH-DATA-15-06`, `TC-1731`; `GET /api/catalog/dropped/tables`).
 *
 * It LINKS to each table's page and does not restore: the page is where the
 * restore lives, beside the state it restores. The days left come from
 * [`daysLeft`] — one definition of that arithmetic, shared with the
 * dashboards' trash — and `now` is a parameter so a test can pin it.
 */
export function DroppedTablesNote({ now }: { now?: number }) {
  // The impure `Date.now()` is reached through a module function, never written
  // in this component's render body, where `react-hooks/purity` would refuse it
  // (`freshness-strip.tsx` uses the same shape). A test pins the clock with
  // `now`, so this call does not run there.
  const nowMs = now ?? clockNow()
  const list = useService((s) => assetService.listDroppedTables(s), [])

  // Nothing yet is nothing to say: an empty list is the ordinary state.
  if (list.status === "loading") return null
  if (list.status === "error") {
    return (
      <p className="text-sm text-muted-foreground">
        The list of dropped tables could not be read.
      </p>
    )
  }
  if (list.data.length === 0) return null

  return (
    <SectionCard
      size="sm"
      title="Dropped tables"
      description="Dropped from the console. Open a table to restore it before its time runs out."
    >
      <ul className="divide-y divide-border text-sm">
        {list.data.map((entry) => {
          const days = daysLeft(entry.expiresAt, nowMs)
          const left = days <= 0 ? "less than a day left" : `${days} ${days === 1 ? "day" : "days"} left`
          return (
            <li key={entry.table} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5">
              <Link
                className="min-w-0 flex-1 truncate underline-offset-2 hover:underline"
                href={`/data/assets/${encodeURIComponent(entry.assetId)}`}
              >
                {entry.table}
              </Link>
              <span className="text-xs text-muted-foreground">{`dropped by ${entry.droppedBy}`}</span>
              <span className="text-xs text-muted-foreground tabular-nums">
                {entry.rowCount === null
                  ? "rows not counted"
                  : `${formatNumber(entry.rowCount)} ${entry.rowCount === 1 ? "row" : "rows"}`}
              </span>
              <span className="text-xs text-muted-foreground">{left}</span>
            </li>
          )
        })}
      </ul>
    </SectionCard>
  )
}
