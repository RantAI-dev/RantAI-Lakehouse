"use client"

import * as React from "react"
import { HistoryIcon } from "lucide-react"
import { CodeBlock } from "@/components/patterns/code-block"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
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
import type { SavedQueryVersion } from "@/services/contracts/queries"

/**
 * What a version did to the parameters (`QS-5`); nothing when it left them
 * as the version before it had them. A change of the parameters alone writes
 * a version, so without this line two versions with one SQL would look equal.
 * `older` is the next row of the list, which the server sends newest first.
 */
function ParametersLine({
  version,
  older,
}: {
  readonly version: SavedQueryVersion
  readonly older: SavedQueryVersion | undefined
}) {
  if (!older || JSON.stringify(version.parameters) === JSON.stringify(older.parameters)) return null
  return (
    <p className="text-xs text-muted-foreground">
      {version.parameters.length === 0
        ? "Parameters removed"
        : `Parameters changed: ${version.parameters.map((def) => def.name).join(", ")}`}
    </p>
  )
}

/**
 * The history of a saved query (`QS-3` C2).
 *
 * Seeing the history is part of seeing the query, so the dialog opens at
 * **view**; going back to a version needs **edit**, and it is confirmed
 * first. The server keeps the newest versions and sends them newest first,
 * and the label of a version is the server's own — "Restored version 2" is
 * composed by the store, so the console shows what it was given and writes
 * none of its own (C6).
 */
export function SavedQueryHistoryDialog({
  queryId,
  title,
  level,
  onClose,
  onReverted,
}: {
  /** The query whose history is shown. */
  readonly queryId: string
  /** The query's title, for the dialog's title. */
  readonly title: string
  /** The level the caller holds on the query. */
  readonly level: string
  readonly onClose: () => void
  /** After a successful revert, so the page reads its list again. */
  readonly onReverted: () => void
}) {
  const history = useService(
    (signal) => queryService.listSavedVersions(queryId, signal),
    [queryId],
  )
  const revert = useServiceAction((signal, version: number) =>
    queryService.revertSavedVersion(queryId, version, signal),
  )
  const [asking, setAsking] = React.useState<number | null>(null)
  const versions = history.data ?? []
  // The newest version holds the query as it is now, so it is what the
  // query is: going back to it would write a copy of itself.
  const newest = versions[0]?.version
  const mayRevert = levelAtLeast(level, "edit")

  async function goBack(version: number) {
    const done = await revert.run(version)
    if (!done) return
    setAsking(null)
    // The revert wrote a new version, so the history is read again.
    history.reload()
    onReverted()
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
            <HistoryIcon className="size-4" aria-hidden />
            History of {title}
          </DialogTitle>
          <DialogDescription>
            The versions the server keeps, newest first.
          </DialogDescription>
        </DialogHeader>
        {history.status === "loading" ? (
          <p className="text-sm text-muted-foreground">Loading…</p>
        ) : null}
        {history.status === "error" ? (
          <p role="alert" className="text-sm text-destructive">
            {readableErrorMessage(history.error.message)}
          </p>
        ) : null}
        {history.status === "success" && versions.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No versions yet. The next change to this query is the first.
          </p>
        ) : null}
        <ul className="flex max-h-[50vh] flex-col divide-y overflow-y-auto">
          {versions.map((version, at) => (
            <li key={version.version} className="flex flex-col gap-1 py-2">
              <div className="flex items-center gap-2 text-sm">
                <span className="font-medium tabular-nums">v{version.version}</span>
                <span className="min-w-0 flex-1 truncate">{version.title}</span>
                {version.version === newest ? (
                  <span className="text-xs text-muted-foreground">Current</span>
                ) : mayRevert ? (
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => {
                      revert.reset()
                      setAsking(version.version)
                    }}
                  >
                    Revert
                  </Button>
                ) : null}
              </div>
              <CodeBlock>{version.sql}</CodeBlock>
              <ParametersLine version={version} older={versions[at + 1]} />
              <p className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
                <span>{version.authorName}</span>
                <span aria-hidden="true">·</span>
                <span>{formatRelativeTime(version.createdAt)}</span>
                {version.label ? (
                  <>
                    <span aria-hidden="true">·</span>
                    <span>{version.label}</span>
                  </>
                ) : null}
              </p>
            </li>
          ))}
        </ul>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
        </DialogFooter>
        {asking !== null ? (
          <ConfirmActionDialog
            open
            onOpenChange={(open) => {
              if (!open) setAsking(null)
            }}
            title={`Revert to version ${asking}`}
            description="The query goes back to the content of that version. The revert writes a new version, so it can be undone."
            // The button names the version it goes back to, so it does not read like Cancel (`#422 S3`).
            confirmLabel={`Restore version ${asking}`}
            confirming={revert.status === "pending"}
            onConfirm={() => {
              void goBack(asking)
            }}
          >
            {revert.error ? (
              <p role="alert" className="text-sm text-destructive">
                {readableErrorMessage(revert.error.message)}
              </p>
            ) : null}
          </ConfirmActionDialog>
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
