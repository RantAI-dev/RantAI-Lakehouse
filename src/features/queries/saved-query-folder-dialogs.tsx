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

/**
 * What the folder dialog is doing: making a folder under a parent, or
 * renaming one that is already there.
 */
export type SavedQueryFolderNaming =
  | {
      readonly mode: "create"
      /** The folder the new one sits in; `null` for the shared space. */
      readonly parentId: string | null
      /** What the dialog says the new folder joins. */
      readonly place: string
    }
  | {
      readonly mode: "rename"
      readonly id: string
      readonly current: string
    }

/** The longest a folder name may be, as the server counts it. */
const MAX_FOLDER_NAME_CHARS = 100

/**
 * The name of a folder: the name typed, and nothing else. The server holds
 * the other rules (no sibling of the same name, four levels, no cycle) and
 * answers with its own sentence when one is broken.
 */
export function SavedQueryFolderNameDialog({
  naming,
  onClose,
  onSubmit,
  busy,
  error,
}: {
  readonly naming: SavedQueryFolderNaming
  readonly onClose: () => void
  readonly onSubmit: (name: string) => void
  readonly busy?: boolean
  readonly error?: string | null
}) {
  const [name, setName] = React.useState(
    naming.mode === "rename" ? naming.current : "",
  )

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>
            {naming.mode === "rename" ? "Rename folder" : "New folder"}
          </DialogTitle>
          <DialogDescription>
            {naming.mode === "rename"
              ? "The name the tree shows."
              : `The folder joins ${naming.place}.`}
          </DialogDescription>
        </DialogHeader>
        <Input
          autoFocus
          value={name}
          maxLength={MAX_FOLDER_NAME_CHARS}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && name.trim()) onSubmit(name.trim())
          }}
          aria-label="Folder name"
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
            disabled={busy || !name.trim()}
            onClick={() => onSubmit(name.trim())}
          >
            {naming.mode === "rename" ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
