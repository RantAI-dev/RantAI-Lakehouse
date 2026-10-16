"use client"

import { DatabaseIcon } from "lucide-react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { TABLE_NAME_RULE, rawTableFullName, tableNameFault } from "@/lib/uploads"
import { cn } from "@/lib/utils"
import type { UploadLoadMode } from "@/services/contracts/uploads"
import { UploadNotice } from "./upload-parts"

const MODES: readonly { value: UploadLoadMode; label: string; note: string }[] = [
  { value: "replace", label: "Replace its rows", note: "The table ends with this file's rows." },
  { value: "append", label: "Add to its rows", note: "This file's rows are added after the rows already there." },
]

/**
 * Step 3: the raw table's name, where it lands, and, for a table an earlier
 * upload created, what happens to its rows.
 *
 * The hint under the field is the rule while the name is fine and the
 * specific break (`tableNameFault`) when it is not; the API's own rule
 * sentence stays the fallback. The destination shows the name as Data
 * Explorer will list it (`rawTableFullName`), only for a name that passes.
 */
export function UploadTableStep({
  tableName,
  onTableName,
  offersMode,
  mode,
  onMode,
}: {
  readonly tableName: string
  readonly onTableName: (name: string) => void
  /** Whether the name is a table an upload created, so there is a choice to make. */
  readonly offersMode: boolean
  readonly mode: UploadLoadMode
  readonly onMode: (mode: UploadLoadMode) => void
}) {
  const problem = tableName === "" ? null : tableNameFault(tableName)
  // The fieldset below has `min-w-0`: a fieldset defaults to its content's min width, which a long name would push past the card.
  const valid = tableName !== "" && problem === null
  return (
    <div className="space-y-5">
      <div className="space-y-1.5">
        <Label htmlFor="upload-table-name">Table name</Label>
        <Input
          id="upload-table-name"
          value={tableName}
          autoComplete="off"
          spellCheck={false}
          className="font-mono"
          aria-invalid={problem !== null}
          aria-describedby="upload-table-name-note"
          onChange={(e) => onTableName(e.target.value)}
        />
        <p id="upload-table-name-note" className={cn("text-xs", problem ? "text-destructive" : "text-muted-foreground")}>
          {problem ?? TABLE_NAME_RULE}
        </p>
      </div>

      <div className="flex items-start gap-3 rounded-lg border border-border bg-muted/30 px-3 py-2.5">
        <DatabaseIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
        <div className="min-w-0">
          <p className="text-xs font-medium text-muted-foreground">Lands in Data Explorer as</p>
          {valid ? (
            <p className="break-all font-mono text-sm" data-testid="upload-destination">
              {rawTableFullName(tableName)}
            </p>
          ) : (
            <p className="text-sm text-muted-foreground">Enter a valid name to see it.</p>
          )}
        </div>
      </div>

      {offersMode ? (
        <fieldset className="min-w-0 space-y-2">
          <legend className="text-sm font-medium">
            <span className="break-all">{tableName}</span> was created by an earlier upload. What should happen to its rows?
          </legend>
          <div className="grid gap-2 sm:grid-cols-2">
            {MODES.map((option) => (
              <div
                key={option.value}
                className={cn(
                  "min-w-0 rounded-lg border px-3 py-2.5",
                  mode === option.value ? "border-primary bg-primary/5" : "border-border"
                )}
              >
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input
                    type="radio"
                    name="upload-load-mode"
                    value={option.value}
                    checked={mode === option.value}
                    aria-describedby={`upload-mode-${option.value}`}
                    onChange={() => onMode(option.value)}
                  />
                  {option.label}
                </label>
                <p id={`upload-mode-${option.value}`} className="mt-1 pl-6 text-xs text-muted-foreground">
                  {option.note}
                </p>
              </div>
            ))}
          </div>
        </fieldset>
      ) : null}

      <UploadNotice>Every column is stored as text. Numbers and dates have to be converted afterwards.</UploadNotice>
    </div>
  )
}
