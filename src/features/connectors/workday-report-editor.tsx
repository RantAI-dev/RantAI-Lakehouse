"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { reportName, reportProblem } from "@/lib/connectors/workday-report"

/**
 * The reports of a Workday connector (`SRC-3` slice 6, decision 5): Workday has no list of reports to ask for,
 * so the person names each custom report by its owner and name. Two plain fields, no discovery. A report is
 * always read in full on every run (Workday cannot filter it by when a row changed), which is said here.
 *
 * It owns no state of the connector: `reports` are the texts in the dial's `objects`, `onAdd` gives the new
 * report's text, `onRemove` takes one away.
 */
export function WorkdayReportEditor({
  reports,
  onAdd,
  onRemove,
  busy = false,
}: {
  reports: readonly string[]
  onAdd: (name: string) => void
  onRemove: (name: string) => void
  busy?: boolean
}) {
  const [owner, setOwner] = React.useState("")
  const [report, setReport] = React.useState("")
  const problem = owner.trim() === "" && report.trim() === "" ? null : reportProblem(owner, report, reports)

  function add() {
    if (reportProblem(owner, report, reports) !== null) return
    onAdd(reportName(owner, report))
    setOwner("")
    setReport("")
  }

  return (
    <div className="space-y-2 rounded-md border border-border p-3" aria-label="Reports">
      <p className="text-sm font-medium">Reports</p>
      <p className="text-xs text-muted-foreground">
        Name each custom report to load: the Workday user who owns it and its name as it appears in the report&apos;s
        web service address. Every run reads the whole report again and replaces the table, because Workday cannot
        say which rows changed.
      </p>
      {reports.length > 0 ? (
        <ul className="space-y-1">
          {reports.map((text) => (
            <li key={text} className="flex items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded bg-muted px-1.5 py-0.5 text-xs">{text}</code>
              <Button type="button" size="xs" variant="ghost" disabled={busy} onClick={() => onRemove(text)}>
                Remove
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="space-y-1">
          <Label htmlFor="workday-report-owner" className="text-xs">
            Report owner
          </Label>
          <Input id="workday-report-owner" value={owner} onChange={(e) => setOwner(e.target.value)} className="h-8 font-mono text-xs" autoComplete="off" />
        </div>
        <div className="space-y-1">
          <Label htmlFor="workday-report-name" className="text-xs">
            Report name
          </Label>
          <Input id="workday-report-name" value={report} onChange={(e) => setReport(e.target.value)} className="h-8 font-mono text-xs" autoComplete="off" />
        </div>
      </div>
      {problem ? <p className="text-xs text-destructive">{problem}</p> : null}
      <Button type="button" size="sm" variant="outline" disabled={busy || reportProblem(owner, report, reports) !== null} onClick={add}>
        Add report
      </Button>
    </div>
  )
}
