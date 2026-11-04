"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { copyText } from "@/lib/copy-text"
import { notifyFailure, notifySuccess } from "@/lib/notify"
import type { QueryCell } from "@/services/contracts/queries"
import { cellText } from "./query-grid-tools"

/**
 * The whole value of one cell.
 *
 * A cell in the grid is one line of a table, so a long text or a JSON
 * document is cut short there. The inspector shows all of it — pretty-printed
 * when the value is JSON — and puts the raw value on the clipboard.
 */
export function QueryCellInspector({
  column,
  value,
  open,
  onOpenChange,
}: {
  /** The column the cell belongs to, so the reader knows what they are looking at. */
  readonly column: string
  readonly value: QueryCell
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
}) {
  const text = cellText(value)
  const pretty = prettyJson(text)

  async function copy() {
    // The raw value, not the pretty-printed one: the clipboard is for the
    // value the engine sent.
    const copied = await copyText(text)
    if (copied) {
      notifySuccess("Value copied", "The cell's value is on the clipboard.")
    } else {
      notifyFailure(
        "Could not copy the value",
        "The browser refused the copy."
      )
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{column}</DialogTitle>
          <DialogDescription>The whole value of the cell.</DialogDescription>
        </DialogHeader>
        <pre className="max-h-80 overflow-auto rounded-md bg-muted/50 p-3 text-xs break-words whitespace-pre-wrap">
          {pretty ?? text}
        </pre>
        <DialogFooter>
          <Button type="button" size="sm" onClick={() => void copy()}>
            Copy value
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * The value as JSON, pretty-printed, when the value is JSON.
 *
 * Only an object or an array counts: a cell that reads "10" or "Jakarta" is
 * text, and showing it as a quoted string would be a lie about its type.
 */
function prettyJson(text: string): string | null {
  if (text === "") return null
  try {
    const parsed: unknown = JSON.parse(text)
    if (parsed === null || typeof parsed !== "object") return null
    return JSON.stringify(parsed, null, 2)
  } catch {
    return null
  }
}
