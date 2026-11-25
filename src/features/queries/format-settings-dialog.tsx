"use client"

import * as React from "react"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { KeywordCase } from "@/lib/sql-format"
import type { useFormatSettings } from "./use-format-settings"

/** What the keyword case is called in the settings control. */
const KEYWORD_CASE_LABELS: Record<KeywordCase, string> = {
  upper: "Upper case",
  lower: "Lower case",
  preserve: "As typed",
}

/** What the indent is called in the settings control. */
const INDENT_LABELS: Record<"0" | "2" | "4", string> = {
  "0": "No indent",
  "2": "2 spaces",
  "4": "4 spaces",
}

/** The indent the control picked, or none when it picked something else. */
function indentOf(value: string | null): 0 | 2 | 4 {
  if (value === "2") return 2
  if (value === "4") return 4
  return 0
}

/**
 * "Format settings…" from the ⋯ menu (`UX-9`; it was a popover on the old
 * toolbar): how Format writes keywords and indents. The settings are the
 * page's (`useFormatSettings`), so Format reads what this dialog sets.
 */
export function FormatSettingsDialog({
  open,
  onOpenChange,
  format,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly format: ReturnType<typeof useFormatSettings>
}) {
  const { settings, setSettings } = format
  const id = React.useId()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>Format settings</DialogTitle>
          <DialogDescription>How Format writes the SQL in the editor.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor={`${id}-case`}>Keyword case</Label>
            <Select
              value={settings.keywordCase}
              items={KEYWORD_CASE_LABELS}
              onValueChange={(v) =>
                setSettings({ ...settings, keywordCase: (v ?? "upper") as KeywordCase })
              }
            >
              <SelectTrigger id={`${id}-case`} className="w-full" aria-label="Keyword case">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="upper">Upper case</SelectItem>
                <SelectItem value="lower">Lower case</SelectItem>
                <SelectItem value="preserve">As typed</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor={`${id}-indent`}>Indent</Label>
            <Select
              value={String(settings.indent)}
              items={INDENT_LABELS}
              onValueChange={(v) => setSettings({ ...settings, indent: indentOf(v) })}
            >
              <SelectTrigger id={`${id}-indent`} className="w-full" aria-label="Indent">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="0">No indent</SelectItem>
                <SelectItem value="2">2 spaces</SelectItem>
                <SelectItem value="4">4 spaces</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
