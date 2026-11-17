"use client"

import * as React from "react"
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
import { Input } from "@/components/ui/input"
import type { SavedQuery } from "@/services/contracts/queries"

/**
 * Rename a saved query: the name the list shows, and nothing else. The
 * statement itself is changed in the Studio (`QS-3` C4), and the server
 * refuses a blank title.
 */
export function SavedQueryRenameDialog({
  query,
  onClose,
  onRename,
  busy,
  error,
}: {
  readonly query: SavedQuery
  readonly onClose: () => void
  readonly onRename: (title: string) => void
  readonly busy?: boolean
  readonly error?: string | null
}) {
  const [title, setTitle] = React.useState(query.title)

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>Rename query</DialogTitle>
          <DialogDescription>
            The name the list shows. The statement itself is changed in the
            Studio.
          </DialogDescription>
        </DialogHeader>
        <Input
          autoFocus
          value={title}
          onChange={(event) => setTitle(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && title.trim()) onRename(title.trim())
          }}
          aria-label="Query title"
        />
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={busy || !title.trim()}
            onClick={() => onRename(title.trim())}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
