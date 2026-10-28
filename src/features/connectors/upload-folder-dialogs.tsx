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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useServiceAction } from "@/hooks/use-service"
import { folderChoices, folderLabel } from "@/lib/upload-folders"
import { uploadService } from "@/services"
import type { UploadFolder } from "@/services/contracts/uploads"

// Upload-specific dialogs, not the dashboards' `folder-dialogs.tsx` made generic (plan AD6,
// the developer's choice): those are bound to `dashboardService`, move one board at a time,
// carry the dashboards' personal-space rule and a four-level limit. Generalising them would
// touch dashboard behaviour for no gain here; the tree helpers (`folder-tree.ts`) are shared.

/** Select value for "no folder" (base-ui needs a non-empty value). */
const NONE = "__none__"

/**
 * Asks for a folder name and runs `run` with it: a new folder or a rename. The sentence the API
 * answers with (name length, a sibling of that name, the third level) is shown as it is.
 *
 * The form is its own component so that its state starts fresh each time the dialog opens: the
 * dialog's content is mounted only while open.
 */
export function FolderNameDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  initialName = "",
  run,
  onDone,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly title: string
  readonly description: string
  readonly confirmLabel: string
  readonly initialName?: string
  readonly run: (name: string, signal: AbortSignal) => Promise<UploadFolder>
  /** Called with the folder the API answered, after a success; the caller closes the dialog. */
  readonly onDone: (folder: UploadFolder) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <FolderNameForm
          title={title}
          description={description}
          confirmLabel={confirmLabel}
          initialName={initialName}
          run={run}
          onDone={onDone}
        />
      </DialogContent>
    </Dialog>
  )
}

function FolderNameForm({
  title,
  description,
  confirmLabel,
  initialName,
  run,
  onDone,
}: {
  readonly title: string
  readonly description: string
  readonly confirmLabel: string
  readonly initialName: string
  readonly run: (name: string, signal: AbortSignal) => Promise<UploadFolder>
  readonly onDone: (folder: UploadFolder) => void
}) {
  const [name, setName] = React.useState(initialName)
  const act = useServiceAction((signal, value: string) => run(value, signal))
  const trimmed = name.trim()
  const pending = act.status === "pending"

  async function submit() {
    if (trimmed === "" || pending) return
    const done = await act.run(trimmed)
    if (done !== null) onDone(done)
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>{description}</DialogDescription>
      </DialogHeader>
      <div className="space-y-1.5">
        <Label htmlFor="upload-folder-name">Folder name</Label>
        <Input
          id="upload-folder-name"
          value={name}
          autoFocus
          autoComplete="off"
          aria-invalid={act.error !== null}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit()
          }}
        />
        {act.error !== null ? (
          <p role="alert" className="text-sm text-destructive">
            {act.error.message}
          </p>
        ) : null}
      </div>
      <DialogFooter>
        <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
        <Button size="sm" onClick={() => void submit()} disabled={pending || trimmed === ""}>
          {confirmLabel}
        </Button>
      </DialogFooter>
    </>
  )
}

/**
 * The picker of a folder, "No folder" first: the one renderer of Move to folder and of the Table
 * step. `value` is a folder id or `null` for no folder.
 */
export function FolderSelect({
  folders,
  value,
  onChange,
  disabled = false,
  id,
  "aria-label": ariaLabel,
}: {
  readonly folders: readonly UploadFolder[]
  readonly value: string | null
  readonly onChange: (value: string | null) => void
  readonly disabled?: boolean
  readonly id?: string
  readonly "aria-label"?: string
}) {
  return (
    <Select value={value ?? NONE} onValueChange={(next) => onChange(next === null || next === NONE ? null : next)} disabled={disabled}>
      <SelectTrigger id={id} aria-label={ariaLabel} className="w-full">
        <SelectValue>{folderLabel(folders, value ?? undefined)}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={NONE}>No folder</SelectItem>
        {folderChoices(folders).map((choice) => (
          <SelectItem key={choice.id} value={choice.id}>
            {choice.path}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

/**
 * Move the chosen files to a folder, or out of every folder. All of them are the tenant's or none
 * moves, and the API's sentence is shown if it refuses.
 */
export function MoveUploadsDialog({
  open,
  onOpenChange,
  folders,
  uploadIds,
  currentFolderId,
  onMoved,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly folders: readonly UploadFolder[]
  readonly uploadIds: readonly string[]
  /** The folder all of the files share, when they do (the Move button is off for it). */
  readonly currentFolderId?: string
  readonly onMoved: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <MoveForm folders={folders} uploadIds={uploadIds} currentFolderId={currentFolderId} onMoved={onMoved} />
      </DialogContent>
    </Dialog>
  )
}

function MoveForm({
  folders,
  uploadIds,
  currentFolderId,
  onMoved,
}: {
  readonly folders: readonly UploadFolder[]
  readonly uploadIds: readonly string[]
  readonly currentFolderId?: string
  readonly onMoved: () => void
}) {
  // "No folder" first: the commonest move is out of a folder, and the folder the files share is no destination.
  const [target, setTarget] = React.useState<string | null>(null)
  const move = useServiceAction((signal, folderId: string | null) =>
    uploadService.moveUploads({ uploadIds: [...uploadIds], folderId }, signal)
  )
  const count = uploadIds.length

  async function submit() {
    const done = await move.run(target)
    if (done !== null) onMoved()
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>Move to folder</DialogTitle>
        <DialogDescription>
          {count === 1 ? "Move 1 file" : `Move ${count} files`} to a folder. The files&apos; tables do not change.
        </DialogDescription>
      </DialogHeader>
      <FolderSelect folders={folders} value={target} onChange={setTarget} aria-label="Destination folder" />
      {folders.length === 0 ? (
        <p className="text-xs text-muted-foreground">No folders yet. Create one with New folder on the Uploaded files tab.</p>
      ) : null}
      {move.error !== null ? (
        <p role="alert" className="text-sm text-destructive">
          {move.error.message}
        </p>
      ) : null}
      <DialogFooter>
        <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
        <Button
          size="sm"
          onClick={() => void submit()}
          disabled={move.status === "pending" || (currentFolderId !== undefined && target === currentFolderId)}
        >
          Move
        </Button>
      </DialogFooter>
    </>
  )
}
