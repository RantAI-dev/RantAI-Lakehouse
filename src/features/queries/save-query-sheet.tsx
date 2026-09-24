"use client"

import * as React from "react"

import { CreateSheet } from "@/components/patterns/create-sheet"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { sqlSummary } from "@/lib/sql-text"
import type { SaveQueryInput } from "@/services/contracts/queries"

/**
 * Name the SQL in the editor and keep it.
 *
 * Saved queries were a read-only list before this: the console could show
 * them and open them, but nothing in the UI could create one.
 */
export function SaveQuerySheet({
  open,
  onOpenChange,
  sql,
  saving,
  error,
  onSave,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly sql: string
  readonly saving: boolean
  readonly error?: string | null
  readonly onSave: (input: SaveQueryInput) => Promise<unknown>
}) {
  const [title, setTitle] = React.useState("")
  const [tags, setTags] = React.useState("")

  // Opening the sheet proposes a name taken from the query itself, so the
  // common case is "type nothing, press Save".
  React.useEffect(() => {
    if (open) {
      setTitle(sqlSummary(sql))
      setTags("")
    }
  }, [open, sql])

  return (
    <CreateSheet
      open={open}
      onOpenChange={onOpenChange}
      title="Save query"
      description="Saved queries are shared with the team, with you recorded as the author."
      submitLabel="Save"
      submitting={saving}
      canSubmit={title.trim().length > 0 && sql.trim().length > 0}
      error={error}
      onSubmit={() => {
        void onSave({
          title: title.trim(),
          sql,
          tags: tags
            .split(",")
            .map((t) => t.trim())
            .filter(Boolean),
        }).then((saved) => {
          if (saved) onOpenChange(false)
        })
      }}
    >
      <div className="space-y-2">
        <Label htmlFor="saved-query-title">Title</Label>
        <Input
          id="saved-query-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Revenue by region"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="saved-query-tags">Tags</Label>
        <Input
          id="saved-query-tags"
          value={tags}
          onChange={(e) => setTags(e.target.value)}
          placeholder="finance, gold"
        />
        <p className="text-xs text-muted-foreground">Separated by commas.</p>
      </div>
    </CreateSheet>
  )
}
