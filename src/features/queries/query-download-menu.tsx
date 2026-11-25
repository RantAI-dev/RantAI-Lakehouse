"use client"

import * as React from "react"
import { Download, LoaderCircle } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuGroupLabel,
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
      {/* UX-9: an icon in the result's header row; its name says the same
          as the words did, and the reason it is off is its title. */}
      <DropdownMenuTrigger
        render={
          <Button
            size="icon-sm"
            variant="ghost"
            disabled={disabled}
            title={reason ?? (busy !== null ? "Downloading…" : "Download")}
            className="aria-expanded:bg-muted"
          />
        }
      >
        {busy !== null ? (
          <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" aria-hidden />
        ) : (
          <Download className="size-4" aria-hidden />
        )}
        <span className="sr-only">{busy !== null ? "Downloading…" : "Download"}</span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-44">
        <DropdownMenuGroup>
          <DropdownMenuGroupLabel>Download as</DropdownMenuGroupLabel>
        {FORMATS.map((choice) => (
          <DropdownMenuItem
            key={choice.format}
            onClick={() => onDownload(choice.format)}
          >
            {choice.label}
          </DropdownMenuItem>
        ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
