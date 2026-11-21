"use client"

import * as React from "react"
import { Trash2Icon } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { levelAtLeast } from "@/lib/access-level"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { readableErrorMessage } from "@/lib/notify-message"
import { queryService } from "@/services"
import type { QueryFolder } from "@/services/contracts/queries"
import { SHARED_SPACE_LABEL } from "./saved-query-tree"

/**
 * The trash of the saved queries (`QS-3` C2).
 *
 * The server sends only the queries the caller may edit — the same set
 * that may delete and restore one (plan 55 D5) — so the console lists what
 * it gets and does not filter it again. A Restore needs **edit**, and a
 * refusal is the server's sentence, shown as it was given (C6).
 */
export function SavedQueryTrashDialog({
  folders,
  level,
  onClose,
  onRestored,
}: {
  /** The folders the caller may see, to name the folder each query waits in. */
  readonly folders: QueryFolder[]
  /** The level the caller holds on the trashed queries. */
  readonly level: string
  readonly onClose: () => void
  /** After something came back, so the page reads its list again. */
  readonly onRestored: () => void
}) {
  const trash = useService((signal) => queryService.listSavedTrash(signal), [])
  const restore = useServiceAction((signal, id: string) =>
    queryService.restoreSaved(id, signal),
  )
  const entries = trash.data ?? []
  const mayRestore = levelAtLeast(level, "edit")

  async function bringBack(id: string) {
    const done = await restore.run(id)
    if (!done) return
    // The query that came back leaves the trash, so the list is read again.
    trash.reload()
    onRestored()
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Trash2Icon className="size-4" aria-hidden />
            Trash
          </DialogTitle>
          <DialogDescription>
            The queries that wait to come back.
          </DialogDescription>
        </DialogHeader>
        {trash.status === "loading" ? (
          <p className="text-sm text-muted-foreground">Loading…</p>
        ) : null}
        {trash.status === "error" ? (
          <p role="alert" className="text-sm text-destructive">
            {readableErrorMessage(trash.error.message)}
          </p>
        ) : null}
        {trash.status === "success" && entries.length === 0 ? (
          <p className="text-sm text-muted-foreground">The trash is empty.</p>
        ) : null}
        <ul className="flex max-h-[50vh] flex-col divide-y overflow-y-auto">
          {entries.map((entry) => {
            const folder = folders.find((item) => item.id === entry.folderId)
            return (
              <li key={entry.id} className="flex flex-col gap-1 py-2">
                <div className="flex items-center gap-2 text-sm">
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {entry.title}
                  </span>
                  {mayRestore ? (
                    <Button
                      size="xs"
                      variant="outline"
                      disabled={restore.status === "pending"}
                      onClick={() => void bringBack(entry.id)}
                    >
                      Restore
                    </Button>
                  ) : null}
                </div>
                <p className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
                  <span>{folder ? folder.name : SHARED_SPACE_LABEL}</span>
                  <span aria-hidden="true">·</span>
                  <span>deleted {formatRelativeTime(entry.deletedAt)}</span>
                  <span aria-hidden="true">·</span>
                  <span>{entry.daysLeft} days left</span>
                </p>
              </li>
            )
          })}
        </ul>
        {restore.error ? (
          <p role="alert" className="text-sm text-destructive">
            {readableErrorMessage(restore.error.message)}
          </p>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
