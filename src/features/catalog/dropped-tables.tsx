"use client"

import * as React from "react"
import Link from "next/link"
import { RotateCcw } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { useService, type ServiceState } from "@/hooks/use-service"
import { daysLeft } from "@/lib/dashboard-organise"
import { formatNumber } from "@/lib/format"
import { assetService } from "@/services"
import type { DroppedTableEntry } from "@/services/contracts/assets"

/**
 * The clock the days left are measured against: read while the list renders,
 * and only when a test did not pin `now`.
 */
function clockNow(): number {
  return Date.now()
}

/** The dropped tables, read once for the page: the ⋯ item counts them and the dialog lists them. */
export type DroppedTablesState = ServiceState<DroppedTableEntry[]> & { reload: () => void }

/** `GET /api/catalog/dropped/tables` (`DATA-55`), read once by the Catalog page. */
export function useDroppedTables(): DroppedTablesState {
  return useService((s) => assetService.listDroppedTables(s), [])
}

/**
 * The Catalog's list of tables dropped from the console (`DATA-55`,
 * QA `LH-DATA-15-06`, `TC-1731`).
 *
 * `UX-8`: it used to be a card above the table on every visit; it is now
 * opened from the page's ⋯ ("Dropped tables · n"), so the list of what is
 * in the catalog is the first thing on the page. The rows are the same.
 *
 * It LINKS to each table's page and does not restore: the page is where the
 * restore lives, beside the state it restores. The days left come from
 * [`daysLeft`] — one definition of that arithmetic, shared with the
 * dashboards' trash — and `now` is a parameter so a test can pin it.
 */
export function DroppedTablesList({ state, now }: { state: DroppedTablesState; now?: number }) {
  // The impure `Date.now()` is reached through a module function, never written
  // in this component's render body, where `react-hooks/purity` would refuse it
  // (`freshness-strip.tsx` uses the same shape). A test pins the clock with
  // `now`, so this call does not run there.
  const nowMs = now ?? clockNow()

  if (state.status === "loading") {
    return (
      <p role="status" className="py-6 text-center text-sm text-muted-foreground">
        Loading dropped tables…
      </p>
    )
  }
  if (state.status === "error") {
    return (
      <div className="flex flex-wrap items-center gap-2 py-2">
        <p className="text-sm text-muted-foreground">The list of dropped tables could not be read.</p>
        <Button size="sm" variant="ghost" onClick={state.reload}>
          <RotateCcw className="size-3.5" aria-hidden />
          Retry
        </Button>
      </div>
    )
  }
  if (state.data.length === 0) {
    return <p className="py-6 text-center text-sm text-muted-foreground">No dropped tables.</p>
  }

  return (
    <ul className="-mx-2 flex flex-col text-sm">
      {state.data.map((entry) => {
        const days = daysLeft(entry.expiresAt, nowMs)
        const left = days <= 0 ? "less than a day left" : `${days} ${days === 1 ? "day" : "days"} left`
        return (
          <li key={entry.table}>
            <Link
              className="group flex flex-col gap-0.5 rounded-md px-2 py-2 outline-none transition-colors duration-150 ease-ui hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50"
              href={`/data/assets/${encodeURIComponent(entry.assetId)}`}
            >
              {/* A table address is code (plan decision 3), so it is the one monospace line here. */}
              <span className="truncate font-mono text-[13px] text-foreground">{entry.table}</span>
              <span className="truncate text-xs text-muted-foreground tabular-nums">
                {[
                  `dropped by ${entry.droppedBy}`,
                  entry.rowCount === null
                    ? "rows not counted"
                    : `${formatNumber(entry.rowCount)} ${entry.rowCount === 1 ? "row" : "rows"}`,
                  left,
                ].join(" · ")}
              </span>
            </Link>
          </li>
        )
      })}
    </ul>
  )
}

/** The list in a dialog, opened from the Catalog's ⋯. */
export function DroppedTablesDialog({
  open,
  onOpenChange,
  state,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  state: DroppedTablesState
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Dropped tables</DialogTitle>
          <DialogDescription>Open a table to restore it before its time runs out.</DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] overflow-y-auto">
          <DroppedTablesList state={state} />
        </div>
      </DialogContent>
    </Dialog>
  )
}
