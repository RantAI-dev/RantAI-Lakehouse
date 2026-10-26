"use client"

import { CheckIcon, XIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { formatBytes } from "@/lib/format"
import { mismatchLines } from "@/lib/uploads"
import type { Upload } from "@/services/contracts/uploads"
import { FileMark, Refusal } from "./upload-parts"
import type { UploadGroupView } from "./use-upload-group"

/**
 * The files of a load of several files into one table (`DATA-17` slice 5, decision 4), on the
 * Check step: the first file (the one the preview and the column list below show) and each of the
 * others with a mark, or, when its columns differ, the columns it lacks and the ones it adds. A
 * file that differs does not load: it has a Remove button, and the step's Next stays off until it
 * is gone (`UploadGroupView.ready`). A file still being sent says so.
 *
 * It is not rendered for one file.
 */
export function UploadFilesList({
  first,
  group,
  onRemove,
}: {
  readonly first: Upload
  readonly group: UploadGroupView
  readonly onRemove: (key: string) => void
}) {
  if (group.files.length === 0) return null
  return (
    <div className="space-y-2">
      <p className="text-sm font-medium">
        {group.files.length + 1} files load into one table. Their columns are matched by name, in any order.
      </p>
      <ul aria-label="Files in this load" className="divide-y divide-border overflow-hidden rounded-lg border border-border text-sm">
        <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
          <FileMark name={first.originalFilename} variant="inline" />
          <span className="min-w-0 flex-1 basis-40 truncate font-medium" title={first.originalFilename}>
            {first.originalFilename}
          </span>
          <span className="text-xs text-muted-foreground">{formatBytes(first.sizeBytes)}</span>
          <span className="text-xs text-muted-foreground">First file, shown below</span>
        </li>
        {group.files.map((file) => {
          const lines = file.mismatch === null ? [] : mismatchLines(file.mismatch)
          return (
            <li key={file.key} className="space-y-1 px-3 py-2" data-file-state={file.state}>
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <FileMark name={file.name} variant="inline" />
                <span className="min-w-0 flex-1 basis-40 truncate font-medium" title={file.name}>
                  {file.name}
                </span>
                <span className="text-xs text-muted-foreground">{formatBytes(file.size)}</span>
                {file.state !== "received" ? (
                  <span className="text-xs text-muted-foreground">Being sent</span>
                ) : file.mismatch !== null ? (
                  <span className="text-xs font-medium text-destructive">Different columns</span>
                ) : file.compared ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <CheckIcon className="size-3.5" aria-hidden /> Same columns
                  </span>
                ) : (
                  <span className="text-xs text-muted-foreground">Comparing…</span>
                )}
                <Button type="button" variant="ghost" size="sm" aria-label={`Remove ${file.name}`} onClick={() => onRemove(file.key)}>
                  <XIcon aria-hidden />
                </Button>
              </div>
              {lines.length > 0 ? (
                <ul className="space-y-0.5 text-xs text-destructive">
                  {lines.map((line) => (
                    <li key={line} className="[overflow-wrap:anywhere]">
                      {line}
                    </li>
                  ))}
                </ul>
              ) : null}
            </li>
          )
        })}
      </ul>
      {group.error !== null ? (
        <div className="space-y-2">
          <Refusal error={group.error} />
          <Button type="button" variant="outline" size="sm" onClick={group.retry}>
            Compare again
          </Button>
        </div>
      ) : null}
      {group.problem !== null ? (
        <p role="status" className="text-sm text-muted-foreground">
          {group.problem}
        </p>
      ) : null}
    </div>
  )
}
