"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { SheetsDial } from "@/services/contracts/connectors"

/** `dial` for the `sheets` adapter — spreadsheet id plus a repeatable
 * A1-notation range list, matching `ingest_spec.rs`'s `SheetsDial`. */
export function SheetsDialForm({
  value,
  onChange,
}: {
  value: SheetsDial | null
  onChange: (next: SheetsDial) => void
  /** UX-4: kept for `DialFormFor`'s shared call signature. Every input on
   * this form is required, so it renders no legend and no stars. */
  showLegend?: boolean
}) {
  const dial: SheetsDial = value ?? { spreadsheetId: "", ranges: [""] }

  function setRange(index: number, next: string) {
    onChange({ ...dial, ranges: dial.ranges.map((r, i) => (i === index ? next : r)) })
  }
  function addRange() {
    onChange({ ...dial, ranges: [...dial.ranges, ""] })
  }
  function removeRange(index: number) {
    onChange({ ...dial, ranges: dial.ranges.filter((_, i) => i !== index) })
  }

  return (
    <div className="grid gap-3">
      {/* UX-4: every input on this form is required (spreadsheet id, every
          range), so no star and no legend; `aria-required` stays. */}
      <div className="space-y-1.5">
        <Label htmlFor="sheets-dial-spreadsheet-id">Spreadsheet id</Label>
        <Input
          id="sheets-dial-spreadsheet-id"
          aria-required
          value={dial.spreadsheetId}
          onChange={(e) => onChange({ ...dial, spreadsheetId: e.target.value })}
          autoComplete="off"
        />
      </div>
      <div className="space-y-2">
        <p className="text-sm font-medium">Ranges</p>
        {/* UX-1-R5: every range is read at runtime (`SheetsDial.ranges` is a
            plain Vec<String>, all entries used), so each rendered row is
            required — not just the first. Remove drops an unused row. */}
        <p className="text-xs text-muted-foreground">
          Every listed range is read and must be filled in; use Remove for a row you do not need.
        </p>
        {dial.ranges.map((range, index) => (
          <div key={index} className="flex items-center gap-2">
            <Label htmlFor={`sheets-dial-range-${index}`} className="shrink-0 text-xs">
              Range {index + 1}
            </Label>
            <Input
              id={`sheets-dial-range-${index}`}
              aria-required
              className="min-w-0 flex-1"
              value={range}
              onChange={(e) => setRange(index, e.target.value)}
              placeholder="Sheet1!A1:D"
              autoComplete="off"
            />
            {dial.ranges.length > 1 ? (
              <Button type="button" variant="outline" size="sm" onClick={() => removeRange(index)}>
                Remove
              </Button>
            ) : null}
          </div>
        ))}
        <Button type="button" variant="outline" size="sm" onClick={addRange}>
          Add range
        </Button>
      </div>
    </div>
  )
}
