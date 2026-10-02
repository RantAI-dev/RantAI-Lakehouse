"use client"

import * as React from "react"
import { toast } from "sonner"

import { CreateSheet } from "@/components/patterns/create-sheet"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useServiceAction } from "@/hooks/use-service"
import { sqlSummary } from "@/lib/sql-text"
import { dashboardService } from "@/services"

/**
 * Keep the SQL in the editor as a dashboard SQL source, so a chart can be
 * built on it (a join across several marts, or any custom SELECT).
 *
 * The server decides whether the SQL qualifies — one read-only
 * SELECT/WITH over `serving.*` tables only, no SETTINGS/FORMAT — and
 * answers with a fixed message when it does not; that message is shown
 * here as-is. Rendered only for principals holding `dashboard:sql`.
 */
export function SaveSqlSourceSheet({
  open,
  onOpenChange,
  sql,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly sql: string
}) {
  const [title, setTitle] = React.useState("")
  const createAct = useServiceAction((signal, input: { title: string; sql: string }) =>
    dashboardService.createSqlSource(input, signal)
  )

  React.useEffect(() => {
    if (open) setTitle(sqlSummary(sql))
  }, [open, sql])

  return (
    <CreateSheet
      open={open}
      onOpenChange={onOpenChange}
      title="Save as SQL source"
      description="A SQL source can be picked as the data source of a dashboard chart, next to the Gold marts."
      submitLabel="Save source"
      submitting={createAct.status === "pending"}
      canSubmit={title.trim().length > 0 && sql.trim().length > 0}
      error={createAct.error?.message ?? null}
      onSubmit={() => {
        void createAct.run({ title: title.trim(), sql }).then((saved) => {
          if (!saved) return
          toast.success(`SQL source “${saved.title}” saved`, {
            description: "Pick it under Data source → SQL sources when building a chart.",
          })
          onOpenChange(false)
        })
      }}
    >
      <div className="space-y-2">
        <Label htmlFor="sql-source-title">Title</Label>
        <Input
          id="sql-source-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Materials by group and type"
        />
      </div>
      <ul className="list-disc space-y-1 pl-5 text-xs text-muted-foreground">
        <li>One read-only SELECT or WITH query; joins across marts are fine.</li>
        <li>Only Gold tables, written as serving.&lt;table&gt;.</li>
        <li>Charts on it return at most 2,000 rows and stop after 30 seconds.</li>
        <li>Governance masking and row filters still apply to every viewer.</li>
      </ul>
    </CreateSheet>
  )
}
