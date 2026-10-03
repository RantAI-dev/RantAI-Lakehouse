"use client"

import * as React from "react"
import Link from "next/link"
import { UploadIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
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
import { uploadService } from "@/services"
import type { Upload } from "@/services/contracts/uploads"
import { UploadStatusPill } from "./upload-parts"

/** How often the list is read again while a row is loading. */
const POLL_MS = 5000

/** What a load of this upload may be started from: it was never loaded, or it failed. */
function canLoad(upload: Upload): boolean {
  return upload.status === "uploaded" || upload.status === "failed"
}

/**
 * The "Uploaded files" tab of Sources: every upload of the active tenant,
 * newest first. Plan T11, feature page `docs/core/features/upload-file.md`
 * (checklist 10, 11, 18).
 *
 * The list is polled only while a row is loading, so a quiet list costs no
 * requests, and the polling stops with the component. `rows` absent is "Not
 * measured" and never `0` (see `Upload.rows`).
 */
export function UploadsPanel({ pollMs = POLL_MS }: { readonly pollMs?: number }) {
  const list = useRefreshable((s) => uploadService.list(s), "uploads")
  const [pendingDelete, setPendingDelete] = React.useState<Upload | null>(null)
  const remove = useServiceAction((signal, id: string) => uploadService.remove(id, signal))

  const uploads = list.data
  const loading = uploads?.some((u) => u.status === "ingesting") ?? false
  const { refresh } = list

  React.useEffect(() => {
    if (!loading) return
    const timer = window.setInterval(refresh, pollMs)
    return () => window.clearInterval(timer)
  }, [loading, refresh, pollMs])

  if (uploads === null && list.error === null) return <LoadingSkeleton />
  if (uploads === null && list.error !== null) {
    return <ErrorState error={list.error} onRetry={refresh} />
  }
  if (uploads === null) return null

  if (uploads.length === 0) {
    return (
      <EmptyState
        title="No uploaded files"
        description="Upload a CSV or TSV file to bring it in as a raw table."
        action={
          <Button size="sm" render={<Link href="/connectors/upload" />}>
            <UploadIcon data-icon="inline-start" />
            Upload file
          </Button>
        }
      />
    )
  }

  return (
    <div className="space-y-3">
      {list.error !== null ? (
        <p role="alert" className="text-sm text-destructive">
          The list could not be refreshed: {list.error.message}
        </p>
      ) : null}
      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>File</TableHead>
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
            {uploads.map((upload) => (
              <TableRow key={upload.id}>
                <TableCell className="max-w-64 truncate font-medium" title={upload.originalFilename}>
                  {upload.originalFilename}
                </TableCell>
                <TableCell>{formatBytes(upload.sizeBytes)}</TableCell>
                <TableCell>
                  <div className="space-y-1">
                    <UploadStatusPill status={upload.status} />
                    {upload.status === "failed" && upload.error ? (
                      <p className="max-w-64 whitespace-normal text-xs text-destructive">{upload.error}</p>
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
                    <span className="text-muted-foreground">Not measured</span>
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
            refresh()
          }
        }}
      >
        {remove.error !== null ? (
          <p role="alert" className="text-sm text-destructive">
            {remove.error.message}
          </p>
        ) : null}
      </ConfirmActionDialog>
    </div>
  )
}
