"use client"

/**
 * Bulk actions on the ticked rows of the Dashboards page (`BI-59` T5): Move to
 * folder, Favourite and Move to trash, each through the service that already
 * does it for one item, one item after the other; no new API. A refusal stops
 * nothing: the rest still go, and the result names what was refused.
 */
import { FolderInput, Star, Trash2, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { readableErrorMessage } from "@/lib/notify-message"
import { dashboardReportService, dashboardService } from "@/services"
import type { LibraryItem } from "./library-items"
import { forgetLastBoard } from "./last-board"

/** Whether a row can be ticked: the built-in board cannot be moved or deleted, and templates are never filed. */
export function pickable(item: LibraryItem): boolean {
  return !item.builtin && item.kind !== "template"
}

export type BulkOutcome = { done: number; refused: { name: string; reason: string }[] }

function reasonOf(err: unknown): string {
  return readableErrorMessage(err instanceof Error ? err.message : String(err))
}

/**
 * Marks the ticked dashboards as favourites (those not already). Reports are
 * skipped: the favourites list holds dashboards and charts only
 * (`lakehouse_bi::items::Kind`), and `skipped` says how many.
 */
export async function favouriteItems(items: readonly LibraryItem[], isFavourite: (item: LibraryItem) => boolean): Promise<BulkOutcome & { skipped: number }> {
  const out = { done: 0, refused: [] as BulkOutcome["refused"], skipped: 0 }
  for (const item of items) {
    if (item.kind !== "dashboard") {
      out.skipped += 1
      continue
    }
    if (isFavourite(item)) continue
    try {
      await dashboardService.setFavourite("board", item.id, true)
      out.done += 1
    } catch (err) {
      out.refused.push({ name: item.name, reason: reasonOf(err) })
    }
  }
  return out
}

/** Moves the ticked dashboards and reports to the trash, where they stay restorable for 30 days. */
export async function trashItems(items: readonly LibraryItem[]): Promise<BulkOutcome> {
  const out: BulkOutcome = { done: 0, refused: [] }
  for (const item of items) {
    try {
      if (item.kind === "dashboard") {
        await dashboardService.deleteBoard(item.id)
        // `/dashboards` must not lead back to a board that is in the trash.
        forgetLastBoard(item.id)
      } else if (item.kind === "report") {
        await dashboardReportService.deleteReport(item.id)
      } else {
        continue
      }
      out.done += 1
    } catch (err) {
      out.refused.push({ name: item.name, reason: reasonOf(err) })
    }
  }
  return out
}

/**
 * The bar that takes the place of the filters while rows are ticked, in the
 * same spot, so the eye does not have to look for it (mockup C).
 */
export function BulkBar({ count, canFavourite, onClear, onMove, onFavourite, onTrash, busy }: {
  readonly count: number
  /** False when no ticked row is a dashboard. */
  readonly canFavourite: boolean
  readonly onClear: () => void
  readonly onMove: () => void
  readonly onFavourite: () => void
  readonly onTrash: () => void
  readonly busy: boolean
}) {
  const on = "text-background hover:bg-background/15 hover:text-background disabled:opacity-50"
  return (
    <div role="toolbar" aria-label="Selected items" className="flex w-full flex-wrap items-center gap-1 rounded-xl bg-foreground px-2 py-1 text-background">
      <Button variant="ghost" size="icon-sm" aria-label="Clear selection" onClick={onClear} className={on}>
        <X />
      </Button>
      <span className="pr-2 text-sm font-medium tabular-nums">{count.toLocaleString("en-US")} selected</span>
      <span className="flex-1" />
      <Button variant="ghost" size="sm" className={on} onClick={onMove} disabled={busy}>
        <FolderInput data-icon="inline-start" /> Move to folder
      </Button>
      <Button
        variant="ghost"
        size="sm"
        className={on}
        onClick={onFavourite}
        disabled={busy || !canFavourite}
        title={canFavourite ? undefined : "Reports cannot be favourites; tick a dashboard."}
      >
        <Star data-icon="inline-start" /> Favourite
      </Button>
      <Button variant="ghost" size="sm" className="text-background hover:bg-rose-500/25 hover:text-background disabled:opacity-50" onClick={onTrash} disabled={busy}>
        <Trash2 data-icon="inline-start" /> Move to trash
      </Button>
    </div>
  )
}
