"use client"

import * as React from "react"
import Link from "next/link"
import { FolderPlusIcon, SearchIcon, UploadIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useRefreshable } from "@/hooks/use-refreshable"
import { useServiceAction } from "@/hooks/use-service"
import { formatBytes, formatDateTime, formatNumber } from "@/lib/format"
import {
  ALL_FILES,
  NO_FOLDER,
  canNestIn,
  commonFolder,
  deleteFolderReason,
  folderLabel,
} from "@/lib/upload-folders"
import { uploadService } from "@/services"
import type { Upload, UploadListFilter } from "@/services/contracts/uploads"
import { FolderNameDialog, MoveUploadsDialog } from "./upload-folder-dialogs"
import { UploadFolderTree } from "./upload-folder-tree"
import { UploadStatusPill } from "./upload-parts"

/** How often the list is read again while a row is loading. */
const POLL_MS = 5000

/** How long typing pauses before the search is sent. */
const SEARCH_DELAY_MS = 250

/** What a load of this upload may be started from: it was never loaded, or it failed. */
function canLoad(upload: Upload): boolean {
  return upload.status === "uploaded" || upload.status === "failed"
}

/**
 * The "Uploaded files" tab of Sources: every upload of the active tenant,
 * newest first, grouped in folders (`DATA-35`). Plan T11, feature page
 * `docs/core/features/upload-file.md` (checklist 10, 11, 18) and
 * `docs/core/features/upload-folders.md`.
 *
 * A folder column on the left (All files, No folder, then the tenant's folders), the open
 * folder's files on the right. The search box covers every folder and shows each file's folder,
 * because the API narrows the list BEFORE its newest-100 window is cut. Files can be ticked and
 * moved; a folder is made, renamed and deleted from the buttons above the table, and a Delete
 * that is off says why (decision 8).
 *
 * The list is polled only while a row is loading, so a quiet list costs no
 * requests, and the polling stops with the component. `rows` absent is "Not
 * measured" and never `0` (see `Upload.rows`).
 */
export function UploadsPanel({
  pollMs = POLL_MS,
  searchDelayMs = SEARCH_DELAY_MS,
}: {
  readonly pollMs?: number
  readonly searchDelayMs?: number
}) {
  const [view, setView] = React.useState(ALL_FILES)
  const [searchText, setSearchText] = React.useState("")
  const [query, setQuery] = React.useState("")
  React.useEffect(() => {
    const timer = window.setTimeout(() => setQuery(searchText.trim()), searchDelayMs)
    return () => window.clearTimeout(timer)
  }, [searchText, searchDelayMs])

  const folderList = useRefreshable((s) => uploadService.listFolders(s), "upload-folders")
  const folders = React.useMemo(() => folderList.data ?? [], [folderList.data])
  // A folder that was deleted (or never existed) is not an open view: fall back to every file.
  const openFolder = folders.find((f) => f.id === view)
  const effectiveView =
    view === ALL_FILES || view === NO_FOLDER || openFolder !== undefined || folderList.data === null ? view : ALL_FILES

  // A search covers every folder, so it replaces the open folder as the filter.
  const filter: UploadListFilter | undefined =
    query !== "" ? { q: query } : effectiveView === ALL_FILES ? undefined : { folderId: effectiveView }
  const filterKey = JSON.stringify(filter ?? {})
  const list = useRefreshable((s) => uploadService.list(s, filter), `uploads:${filterKey}`)

  const [selected, setSelected] = React.useState<ReadonlySet<string>>(new Set())
  const [pendingDelete, setPendingDelete] = React.useState<Upload | null>(null)
  const [folderDialog, setFolderDialog] = React.useState<"create" | "subfolder" | "rename" | null>(null)
  const [pendingFolderDelete, setPendingFolderDelete] = React.useState(false)
  const [moveOpen, setMoveOpen] = React.useState(false)
  const remove = useServiceAction((signal, id: string) => uploadService.remove(id, signal))
  const removeFolder = useServiceAction((signal, id: string) => uploadService.deleteFolder(id, signal))

  const uploads = list.data
  const loading = uploads?.some((u) => u.status === "ingesting") ?? false
  const { refresh } = list
  const refreshFolders = folderList.refresh

  React.useEffect(() => {
    if (!loading) return
    const timer = window.setInterval(refresh, pollMs)
    return () => window.clearInterval(timer)
  }, [loading, refresh, pollMs])

  const select = (next: string) => {
    setView(next)
    setSelected(new Set())
  }
  const everything = () => {
    refresh()
    refreshFolders()
  }

  if (uploads === null && list.error === null) return <LoadingSkeleton />
  if (uploads === null && list.error !== null) {
    return <ErrorState error={list.error} onRetry={refresh} />
  }
  if (uploads === null) return null
  if (folderList.data === null && folderList.error === null) return <LoadingSkeleton />

  if (uploads.length === 0 && filter === undefined && folders.length === 0) {
    return (
      <EmptyState
        title="No uploaded files"
        description="Upload CSV, TSV, Excel or Parquet files to bring them in as raw tables."
        action={
          <Button size="sm" render={<Link href="/connectors/upload" />}>
            <UploadIcon data-icon="inline-start" />
            Upload file
          </Button>
        }
      />
    )
  }

  const searching = query !== ""
  const showFolderColumn = searching || effectiveView === ALL_FILES
  const chosen = uploads.filter((u) => selected.has(u.id))
  const allChosen = uploads.length > 0 && chosen.length === uploads.length
  const deleteReason = openFolder ? deleteFolderReason(openFolder) : null
  const toggle = (id: string, on: boolean) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (on) next.add(id)
      else next.delete(id)
      return next
    })

  return (
    <div className="space-y-3">
      {list.error !== null ? (
        <p role="alert" className="text-sm text-destructive">
          The list could not be refreshed: {list.error.message}
        </p>
      ) : null}
      {folderList.error !== null ? (
        <p role="alert" className="text-sm text-destructive">
          The folders could not be loaded: {folderList.error.message}
        </p>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <div className="relative w-full sm:w-72">
          <SearchIcon className="pointer-events-none absolute left-2.5 top-2.5 size-4 text-muted-foreground" aria-hidden />
          <Input
            value={searchText}
            onChange={(e) => setSearchText(e.target.value)}
            placeholder="Search files and tables"
            aria-label="Search uploaded files"
            className="pl-8"
          />
        </div>
        {searching ? (
          <p className="text-sm text-muted-foreground">Searching every folder.</p>
        ) : null}
        <div className="ml-auto flex flex-wrap items-center gap-2">
          {chosen.length > 0 ? (
            <>
              <span className="text-sm text-muted-foreground">{chosen.length} selected</span>
              <Button size="sm" variant="outline" onClick={() => setMoveOpen(true)}>
                Move to folder…
              </Button>
            </>
          ) : null}
          <Button size="sm" variant="outline" onClick={() => setFolderDialog("create")}>
            <FolderPlusIcon data-icon="inline-start" />
            New folder
          </Button>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-[16rem_minmax(0,1fr)]">
        <aside className="space-y-3">
          <UploadFolderTree folders={folders} selected={effectiveView} onSelect={select} />
          {openFolder ? (
            <div className="space-y-2 border-t pt-3">
              <p className="truncate text-xs font-medium text-muted-foreground" title={folderLabel(folders, openFolder.id)}>
                {folderLabel(folders, openFolder.id)}
              </p>
              <div className="flex flex-wrap gap-1.5">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!canNestIn(folders, openFolder.id)}
                  title={canNestIn(folders, openFolder.id) ? undefined : "Folders can be nested at most 3 levels deep."}
                  onClick={() => setFolderDialog("subfolder")}
                >
                  New subfolder
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  aria-label={`Rename folder ${openFolder.name}`}
                  onClick={() => setFolderDialog("rename")}
                >
                  Rename
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  className="text-destructive hover:text-destructive"
                  disabled={deleteReason !== null}
                  aria-label={`Delete folder ${openFolder.name}`}
                  onClick={() => {
                    removeFolder.reset()
                    setPendingFolderDelete(true)
                  }}
                >
                  Delete folder
                </Button>
              </div>
              {deleteReason !== null ? (
                <p className="text-xs text-muted-foreground" data-testid="folder-delete-reason">
                  {deleteReason}
                </p>
              ) : null}
            </div>
          ) : null}
        </aside>

        <div className="min-w-0 rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8">
                  <Checkbox
                    aria-label="Select every file shown"
                    checked={allChosen}
                    onCheckedChange={(on) => setSelected(on === true ? new Set(uploads.map((u) => u.id)) : new Set())}
                  />
                </TableHead>
                <TableHead>File</TableHead>
                {showFolderColumn ? <TableHead>Folder</TableHead> : null}
                <TableHead>Size</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Table</TableHead>
                <TableHead>Rows</TableHead>
                <TableHead>Uploaded by</TableHead>
                <TableHead>Uploaded</TableHead>
                <TableHead>
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {uploads.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={showFolderColumn ? 10 : 9} className="py-8 text-center text-muted-foreground">
                    {searching
                      ? "No file or table matches the search."
                      : effectiveView === NO_FOLDER
                        ? "Every file is in a folder."
                        : "No files in this folder."}
                  </TableCell>
                </TableRow>
              ) : null}
              {uploads.map((upload) => (
                <TableRow key={upload.id}>
                  <TableCell>
                    <Checkbox
                      aria-label={`Select ${upload.originalFilename}`}
                      checked={selected.has(upload.id)}
                      onCheckedChange={(on) => toggle(upload.id, on === true)}
                    />
                  </TableCell>
                  <TableCell className="max-w-64 truncate font-medium" title={upload.originalFilename}>
                    {upload.originalFilename}
                  </TableCell>
                  {showFolderColumn ? (
                    <TableCell className="max-w-48 truncate" title={folderLabel(folders, upload.folderId)}>
                      {upload.folderId !== undefined ? (
                        folderLabel(folders, upload.folderId)
                      ) : (
                        <span className="text-muted-foreground">No folder</span>
                      )}
                    </TableCell>
                  ) : null}
                  <TableCell>
                    {/* A file still being received has stored no bytes yet: show the size it announced. */}
                    {formatBytes(upload.status === "receiving" ? (upload.declaredSize ?? 0) : upload.sizeBytes)}
                  </TableCell>
                  <TableCell>
                    <div className="space-y-1">
                      <UploadStatusPill status={upload.status} />
                      {upload.status === "failed" && upload.error ? (
                        <p className="max-w-64 whitespace-normal text-xs text-destructive">{upload.error}</p>
                      ) : null}
                      {upload.loadedWith !== undefined ? (
                        // `DATA-17` slice 5: a file of a load of several files; the first has the run and the count.
                        <p className="max-w-64 whitespace-normal text-xs text-muted-foreground">
                          Loaded with {uploads.find((u) => u.id === upload.loadedWith)?.originalFilename ?? "the first file"}
                        </p>
                      ) : null}
                    </div>
                  </TableCell>
                  <TableCell>
                    {upload.assetId !== undefined && upload.bronzeTable !== undefined ? (
                      <Link
                        href={`/data/assets/${encodeURIComponent(upload.assetId)}`}
                        className="text-primary underline-offset-4 hover:underline"
                      >
                        {upload.bronzeTable}
                      </Link>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </TableCell>
                  <TableCell>
                    {upload.rows === undefined ? (
                      <span className="text-muted-foreground">
                        {upload.loadedWith !== undefined ? "Counted with the first file" : "Not measured"}
                      </span>
                    ) : (
                      formatNumber(upload.rows)
                    )}
                  </TableCell>
                  <TableCell>{upload.uploadedBy}</TableCell>
                  <TableCell>{formatDateTime(upload.createdAt)}</TableCell>
                  <TableCell>
                    <div className="flex justify-end gap-1">
                      {canLoad(upload) ? (
                        <Button
                          size="sm"
                          variant="outline"
                          render={<Link href={`/connectors/upload?id=${encodeURIComponent(upload.id)}`} />}
                        >
                          Load
                        </Button>
                      ) : null}
                      <Button
                        size="sm"
                        variant="ghost"
                        className="text-destructive hover:text-destructive"
                        disabled={upload.status === "ingesting"}
                        title={
                          upload.status === "ingesting"
                            ? "This upload is being loaded, so it cannot be deleted yet."
                            : undefined
                        }
                        aria-label={`Delete ${upload.originalFilename}`}
                        onClick={() => {
                          remove.reset()
                          setPendingDelete(upload)
                        }}
                      >
                        Delete
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </div>

      <ConfirmActionDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null)
        }}
        title={pendingDelete ? `Delete ${pendingDelete.originalFilename}?` : "Delete upload?"}
        description="The file is removed from storage and from this list."
        impact={
          pendingDelete?.bronzeTable
            ? `The table ${pendingDelete.bronzeTable} stays, with its rows.`
            : "A table this file was loaded into stays, with its rows."
        }
        confirmLabel="Delete"
        confirming={remove.status === "pending"}
        destructive
        onConfirm={async () => {
          if (pendingDelete === null) return
          const done = await remove.run(pendingDelete.id)
          // `remove` answers 204 with no body, which `run` returns as
          // `undefined`; a refusal is `null`.
          if (done !== null) {
            setPendingDelete(null)
            everything()
          }
        }}
      >
        {remove.error !== null ? (
          <p role="alert" className="text-sm text-destructive">
            {remove.error.message}
          </p>
        ) : null}
      </ConfirmActionDialog>

      <ConfirmActionDialog
        open={pendingFolderDelete && openFolder !== undefined}
        onOpenChange={setPendingFolderDelete}
        title={openFolder ? `Delete the folder ${openFolder.name}?` : "Delete folder?"}
        description="The folder is empty, so no file is affected."
        confirmLabel="Delete"
        confirming={removeFolder.status === "pending"}
        destructive
        onConfirm={async () => {
          if (openFolder === undefined) return
          const done = await removeFolder.run(openFolder.id)
          if (done !== null) {
            setPendingFolderDelete(false)
            setView(ALL_FILES)
            everything()
          }
        }}
      >
        {removeFolder.error !== null ? (
          <p role="alert" className="text-sm text-destructive">
            {removeFolder.error.message}
          </p>
        ) : null}
      </ConfirmActionDialog>

      <FolderNameDialog
        open={folderDialog === "create" || folderDialog === "subfolder"}
        onOpenChange={(open) => {
          if (!open) setFolderDialog(null)
        }}
        title={folderDialog === "subfolder" && openFolder ? `New folder in ${openFolder.name}` : "New folder"}
        description="A folder groups uploaded files on this tab. It does not change where a table lives."
        confirmLabel="Create"
        run={(name, signal) =>
          uploadService.createFolder(
            folderDialog === "subfolder" && openFolder ? { name, parentId: openFolder.id } : { name },
            signal
          )
        }
        onDone={(made) => {
          setFolderDialog(null)
          select(made.id)
          everything()
        }}
      />
      <FolderNameDialog
        open={folderDialog === "rename"}
        onOpenChange={(open) => {
          if (!open) setFolderDialog(null)
        }}
        title="Rename folder"
        description="Names are unique among the folders in the same place."
        confirmLabel="Rename"
        initialName={openFolder?.name ?? ""}
        run={(name, signal) =>
          openFolder ? uploadService.updateFolder(openFolder.id, { name }, signal) : Promise.reject(new Error("no folder"))
        }
        onDone={() => {
          setFolderDialog(null)
          everything()
        }}
      />
      <MoveUploadsDialog
        open={moveOpen}
        onOpenChange={setMoveOpen}
        folders={folders}
        uploadIds={chosen.map((u) => u.id)}
        currentFolderId={commonFolder(chosen)}
        onMoved={() => {
          setMoveOpen(false)
          setSelected(new Set())
          everything()
        }}
      />
    </div>
  )
}
