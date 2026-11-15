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
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectTrigger, SelectValue } from "@/components/ui/select"
import { folderOptionText, isPersonalFolder } from "@/lib/dashboard-organise"
import { buildFolderTree, flattenFolders, type FolderLike } from "@/lib/folder-tree"
import { FolderOptions } from "./folder-options"
import { PRIVATE_HINT } from "./private-badge"

/** Select value for "no folder" (base-ui needs a non-empty value). */
const NO_FOLDER = "__none__"

/** Dialog buat/ubah nama + deskripsi sebuah dashboard. */
export function BoardFormDialog({
  open,
  onOpenChange,
  title,
  description,
  submitLabel,
  initialName,
  initialDescription,
  folders,
  busy,
  onSubmit,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly title: string
  readonly description: string
  readonly submitLabel: string
  readonly initialName: string
  readonly initialDescription: string
  /** Given when creating: offers a folder to file the new dashboard in. */
  readonly folders?: FolderLike[]
  readonly busy: boolean
  readonly onSubmit: (values: { name: string; description: string; folderId: string }) => void
}) {
  const [name, setName] = React.useState(initialName)
  const [desc, setDesc] = React.useState(initialDescription)
  const [folderId, setFolderId] = React.useState(NO_FOLDER)
  const folderOptions = flattenFolders(buildFolderTree(folders ?? [], []).folders)

  // Reset tiap kali dibuka: dialog ini dipakai ulang untuk baris yang
  // berbeda, jadi state lama tidak boleh bocor ke board berikutnya.
  React.useEffect(() => {
    if (open) {
      setName(initialName)
      setDesc(initialDescription)
      setFolderId(NO_FOLDER)
    }
  }, [open, initialName, initialDescription])

  const clean = name.trim()
  const canSubmit = clean.length > 0

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="board-name">Name</Label>
            <Input
              id="board-name"
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Quarterly visitors"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="board-description">Description</Label>
            <Input
              id="board-description"
              value={desc}
              onChange={(e) => setDesc(e.target.value)}
              placeholder="What this dashboard answers"
            />
            <p className="text-xs text-muted-foreground">
              Optional. Shown in the dashboard list so others know what it is for.
            </p>
          </div>
          {folderOptions.length > 0 ? (
            <div className="flex flex-col gap-1.5">
              <Label>Folder</Label>
              <Select value={folderId} onValueChange={(v) => setFolderId(v ?? NO_FOLDER)}>
                <SelectTrigger className="w-full">
                  <SelectValue>
                    {folderOptionText(folders ?? [], folderId === NO_FOLDER ? "" : folderId, "No folder")}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <FolderOptions folders={folders ?? []} noneValue={NO_FOLDER} noneLabel="No folder" />
                </SelectContent>
              </Select>
              {isPersonalFolder(folders ?? [], folderId) ? (
                <p className="text-xs text-muted-foreground">{PRIVATE_HINT}</p>
              ) : null}
            </div>
          ) : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={!canSubmit || busy}
            onClick={() =>
              onSubmit({
                name: clean,
                description: desc.trim(),
                folderId: folderId === NO_FOLDER ? "" : folderId,
              })
            }
          >
            {busy ? "Working…" : submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

