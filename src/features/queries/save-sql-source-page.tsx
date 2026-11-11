"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { toast } from "sonner"

import { FormPage } from "@/components/patterns/form-page"
import { EmptyState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useEffectiveLimits } from "@/hooks/use-effective-limits"
import { useServiceAction } from "@/hooks/use-service"
import { sqlSummary } from "@/lib/sql-text"
import { dashboardService } from "@/services"
import { clearSaveSourceDraft, readSaveSourceDraft, type SaveSourceDraft } from "./save-draft"

/**
 * `/query-studio/save-source` (`WEB-7` T5, D2/D5): keep the SQL in the
 * editor as a dashboard SQL source, so a chart can be built on it (a join
 * across several marts, or any custom SELECT).
 *
 * The server decides whether the SQL qualifies — one read-only
 * SELECT/WITH over `serving.*` tables only, no SETTINGS/FORMAT — and
 * answers with a fixed message when it does not; that message is shown here
 * as-is. The statement comes over in `sessionStorage` (`save-draft.ts`); a
 * page opened any other way says to open it from Query Studio. The route
 * itself takes `dashboard:sql` (`POST /api/dashboard/sources`), so a person
 * without it does not reach this page at all (`WEB-7` D2).
 */
export function SaveSqlSourcePage() {
  const [draft, setDraft] = React.useState<SaveSourceDraft | null | undefined>(undefined)

  React.useEffect(() => {
    // Read after mount, never during render: see `save-query-page.tsx`.
    setDraft(readSaveSourceDraft())
  }, [])

  if (draft === undefined) return <LoadingSkeleton rows={4} />
  if (draft === null) {
    return (
      <EmptyState
        title="Open this from Query Studio"
        description="A SQL source is the statement the editor holds, so the form starts there. Write a query, then choose Save as SQL source."
        action={
          <Button variant="outline" size="sm" render={<Link href="/query-studio" />}>
            Open Query Studio
          </Button>
        }
      />
    )
  }
  return <SaveSourceForm draft={draft} />
}

function SaveSourceForm({ draft }: { draft: SaveSourceDraft }) {
  const router = useRouter()
  const backHref = "/query-studio"
  // BI-5: the sentence about the limits follows the setting; without it, no number is printed.
  const limits = useEffectiveLimits()
  const [title, setTitle] = React.useState(() => sqlSummary(draft.sql))

  const createSource = useServiceAction((signal, input: { title: string; sql: string }) =>
    dashboardService.createSqlSource(input, signal)
  )

  async function handleSave() {
    const saved = await createSource.run({ title: title.trim(), sql: draft.sql })
    if (!saved) return
    toast.success(`SQL source “${saved.title}” saved`, {
      description: "Pick it under Data source → SQL sources when building a chart.",
    })
    clearSaveSourceDraft()
    router.push(backHref)
  }

  return (
    <FormPage
      title="Save as SQL source"
      description="A SQL source can be picked as the data source of a dashboard chart, next to the Gold marts."
      backHref={backHref}
      backLabel="Back to Query Studio"
      submitLabel="Save source"
      submittingLabel="Saving…"
      canSubmit={title.trim().length > 0 && draft.sql.trim().length > 0}
      submitting={createSource.status === "pending"}
      error={createSource.status === "error" ? createSource.error.message : null}
      onSubmit={() => void handleSave()}
      onCancel={() => router.push(backHref)}
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
        <li>
          {limits
            ? `Charts on it return at most ${limits.chartRows.toLocaleString("en-US")} rows and stop after ${limits.timeoutSeconds.toLocaleString("en-US")} seconds.`
            : "Charts on it stop at the row and time limits set in Admin > Settings."}
        </li>
        <li>Governance masking and row filters still apply to every viewer.</li>
      </ul>
    </FormPage>
  )
}
