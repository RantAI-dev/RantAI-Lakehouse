"use client"

import * as React from "react"
import { FolderPlusIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { canNestIn, folderLabel } from "@/lib/upload-folders"
import { uploadService } from "@/services"
import type { UploadFolder } from "@/services/contracts/uploads"
import { FolderNameDialog, FolderSelect } from "./upload-folder-dialogs"

/**
 * The folder picker of the Table step (`DATA-35` decision 2): optional, "No folder" by default, with
 * "New folder" beside it. A folder made here sits inside the folder picked, when that one can have
 * folders in it (three levels at most), and at the top level otherwise; it is picked at once.
 *
 * A folder only groups the file on the Uploaded files tab: the table keeps its name.
 */
export function UploadFolderField({
  folders,
  value,
  onChange,
  onCreated,
}: {
  readonly folders: readonly UploadFolder[]
  /** The folder id, or `null` for no folder. */
  readonly value: string | null
  readonly onChange: (value: string | null) => void
  /** Called with a folder just made, so the caller adds it to its list before it is picked. */
  readonly onCreated: (folder: UploadFolder) => void
}) {
  const [creating, setCreating] = React.useState(false)
  const parentId = value !== null && canNestIn(folders, value) ? value : undefined
  return (
    <div className="space-y-1.5">
      <Label htmlFor="upload-folder">Folder (optional)</Label>
      <div className="flex gap-2">
        <div className="min-w-0 flex-1">
          <FolderSelect id="upload-folder" aria-label="Folder (optional)" folders={folders} value={value} onChange={onChange} />
        </div>
        <Button type="button" variant="outline" onClick={() => setCreating(true)}>
          <FolderPlusIcon data-icon="inline-start" />
          New folder
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Folders group files on the Uploaded files tab. The table keeps its name wherever the file is.
      </p>
      <FolderNameDialog
        open={creating}
        onOpenChange={setCreating}
        title={parentId !== undefined ? `New folder in ${folderLabel(folders, parentId)}` : "New folder"}
        description="A folder groups uploaded files on the Uploaded files tab."
        confirmLabel="Create"
        run={(name, signal) => uploadService.createFolder(parentId !== undefined ? { name, parentId } : { name }, signal)}
        onDone={(made) => {
          setCreating(false)
          onCreated(made)
          onChange(made.id)
        }}
      />
    </div>
  )
}
