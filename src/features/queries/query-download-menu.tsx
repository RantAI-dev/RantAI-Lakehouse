"use client"

import * as React from "react"
import { Download } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { QueryDownloadFormat } from "@/services/contracts/queries"

/** The file kinds the menu offers, and the name each of them is offered under. */
const FORMATS: ReadonlyArray<{ format: QueryDownloadFormat; label: string }> = [
  { format: "csv", label: "CSV" },
  { format: "tsv", label: "TSV" },
  { format: "xlsx", label: "Excel" },
  { format: "parquet", label: "Parquet" },
]

/**
 * The engines a result can be downloaded for: ClickHouse, and the
 * placeholder the rows recorded before the change carry. The server refuses
 * every other engine ("download supports ClickHouse runs only"), so the
 * console does not offer a download it knows will be refused.
 */
function isDownloadableEngine(engine: string): boolean {
  return engine === "clickhouse" || engine === "hot-store"
}

/**
 * What the console says when a role's downloads are switched off. The
 * server's own sentence (`DOWNLOADS_OFF`, `routes/settings.rs`), so what the
 * person reads here matches the refusal a direct request gets (`QS-9`,
 * `LH-QS-2-06`, decision 10).
 */
export const DOWNLOADS_OFF_REASON = "Downloads are switched off for your role."

/**
 * The menu that asks for the result as a file.
 *
 * The server makes the file, not the browser: it re-runs the stored
 * statement with the caller's current masks and row filters, so the file
 * holds the whole result and says so when a limit cut it short.
 */
export function QueryDownloadMenu({
  resultId,
  engine,
  busy,
  canDownload,
  onDownload,
}: {
  /** The run's id to download, or null when there is nothing to download. */
  readonly resultId: string | null
  /** The engine the result ran on; only ClickHouse runs can be downloaded. */
  readonly engine: string
  /** The file kind being downloaded, or null when none is. */
  readonly busy: QueryDownloadFormat | null
  /**
   * QA `LH-QS-2-06` (`QS-9`): false when the person's roles may not
   * download. It turns the menu off before the engine reason does.
   */
  readonly canDownload: boolean
  readonly onDownload: (format: QueryDownloadFormat) => void
}) {
  const downloadable = isDownloadableEngine(engine)
  const disabled = resultId === null || !canDownload || !downloadable || busy !== null
  const reason = !canDownload
    ? DOWNLOADS_OFF_REASON
    : downloadable
      ? undefined
      : "Downloads run on ClickHouse only."
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button size="sm" variant="outline" disabled={disabled} title={reason} />
        }
      >
        <Download className="size-4" aria-hidden />
        {busy !== null ? "Downloading…" : "Download"}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {FORMATS.map((choice) => (
          <DropdownMenuItem
            key={choice.format}
            onClick={() => onDownload(choice.format)}
          >
            {choice.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
