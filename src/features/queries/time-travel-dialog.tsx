"use client"

import { Rewind, X } from "lucide-react"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { formatDateTime } from "@/lib/format"
import { pinnedSnapshots } from "@/lib/snapshot-picker"
import { cn } from "@/lib/utils"
import { IcebergTimeTravelControls, type PinnedVersion } from "./time-travel-controls"

/**
 * "Time travel…" from the ⋯ menu (`UX-9`): the version picker in a dialog,
 * out of sight until it is used. Picking a version writes the pin into the
 * SQL and closes the dialog; the chip by the editor then shows it.
 */
export function TimeTravelDialog({
  open,
  onOpenChange,
  sql,
  engine,
  onApply,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly sql: string
  readonly engine: string
  readonly onApply: (nextSql: string, pinned: PinnedVersion) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>Time travel</DialogTitle>
          <DialogDescription>Read a table as it was at an earlier version.</DialogDescription>
        </DialogHeader>
        {open ? (
          <IcebergTimeTravelControls
            sql={sql}
            engine={engine}
            onApply={(next, pinned) => {
              onApply(next, pinned)
              onOpenChange(false)
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

/** A long snapshot id, shortened in the middle: the ends are what tell two apart. */
function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id
}

/**
 * The pin, while there is one, as a chip by the editor: what the query
 * reads and when it was written, a click to pick another version, and ×
 * to read the tables as they are now. The pin is read from the SQL, so one
 * that came in a link shows here as well; `known` adds the table and the
 * time for a version picked in this studio.
 */
export function TimeTravelChip({
  sql,
  known,
  onOpen,
  onRemove,
}: {
  readonly sql: string
  readonly known: Readonly<Record<string, PinnedVersion>>
  readonly onOpen: () => void
  readonly onRemove: () => void
}) {
  const pins = pinnedSnapshots(sql)
  if (pins.length === 0) return null
  const only = pins.length === 1 ? pins[0]! : null
  const picked = only ? known[only] : undefined
  const label = picked
    ? `${picked.table} as of ${picked.at ? formatDateTime(picked.at) : `version ${shortId(picked.snapshotId)}`}`
    : only
      ? `Version ${shortId(only)}`
      : `${pins.length} versions pinned`
  return (
    <span className="inline-flex h-7 min-w-0 items-center gap-0.5 rounded-full bg-violet-500/10 pr-0.5 pl-1 text-xs font-medium text-violet-800 dark:text-violet-300">
      <button
        type="button"
        onClick={onOpen}
        title={`Pinned: ${pins.join(", ")}. Pick another version.`}
        className={cn(
          "inline-flex h-6 min-w-0 items-center gap-1 rounded-full px-1.5 outline-none transition-colors duration-150 ease-ui hover:bg-violet-500/10 focus-visible:ring-3 focus-visible:ring-ring/50"
        )}
      >
        <Rewind className="size-3.5 shrink-0" aria-hidden />
        <span className="truncate">{label}</span>
      </button>
      <button
        type="button"
        aria-label="Read the current version"
        title="Read the current version"
        onClick={onRemove}
        className="grid size-6 shrink-0 place-items-center rounded-full outline-none transition-colors duration-150 ease-ui hover:bg-violet-500/15 focus-visible:ring-3 focus-visible:ring-ring/50"
      >
        <X className="size-3" aria-hidden />
      </button>
    </span>
  )
}
