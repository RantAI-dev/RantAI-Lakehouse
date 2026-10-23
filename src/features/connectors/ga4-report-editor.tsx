"use client"

import * as React from "react"
import { ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { cursorOf, parseReportText, reportProblem, withTimeDimension, GA4_TIME_DIMENSIONS } from "@/lib/connectors/ga4-report"
import type { DiscoveryObject } from "@/services/contracts/connectors"
import { useDiscoveryList } from "./use-discovery"

/** Rows shown of a long list: the person narrows it with the filter. */
const SHOWN = 100

function Choices({
  title,
  kind,
  all,
  chosen,
  onToggle,
}: {
  title: string
  kind: "dimension" | "metric"
  all: DiscoveryObject[]
  chosen: ReadonlySet<string>
  onToggle: (name: string) => void
}) {
  const [filter, setFilter] = React.useState("")
  const needle = filter.trim().toLowerCase()
  const matching = all.filter((o) => o.name.toLowerCase().includes(needle))
  return (
    <fieldset className="min-w-0 space-y-1">
      <legend className="text-xs font-medium">{title}</legend>
      <Input
        aria-label={`Filter ${kind}s`}
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        placeholder={`Filter ${all.length} ${kind}s`}
        className="h-7 text-xs"
        autoComplete="off"
      />
      <ul className="max-h-40 space-y-0.5 overflow-y-auto">
        {matching.slice(0, SHOWN).map((o) => (
          <li key={o.name}>
            <label className="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 text-xs hover:bg-muted/50">
              <input type="checkbox" checked={chosen.has(o.name)} onChange={() => onToggle(o.name)} />
              <span className="min-w-0 flex-1 truncate font-mono">{o.name}</span>
            </label>
          </li>
        ))}
      </ul>
      {matching.length > SHOWN ? <p className="text-xs text-muted-foreground">Showing {SHOWN} of {matching.length}; filter to narrow.</p> : null}
    </fieldset>
  )
}

/**
 * The report editor of a Google Analytics 4 connector (`SRC-3` slice 3 review BLOCKER 9, spec
 * "table and column choice"): GA4 has no list of tables, so the person defines reports. The
 * dimensions and metrics to choose from are the property's own (`getMetadata`, read by the
 * orchestrator through discovery with `schema: "fields"`), never typed blind. A time dimension
 * is the report's cursor; `date` is added when none is chosen and shown as fixed.
 *
 * It owns no state of the connector: `reports` are the texts in the dial's `objects`, `onAdd`
 * gives the new report's text with its cursor and key, `onRemove` takes one away.
 */
export function Ga4ReportEditor({
  connectorId,
  reports,
  onAdd,
  onRemove,
  busy = false,
  pollMs,
}: {
  connectorId: string
  reports: readonly string[]
  onAdd: (text: string, cursor: string, key: string[]) => void
  onRemove: (text: string) => void
  busy?: boolean
  pollMs?: number
}) {
  const fields = useDiscoveryList(connectorId, pollMs)
  const [label, setLabel] = React.useState("")
  const [dimensions, setDimensions] = React.useState<string[]>([])
  const [metrics, setMetrics] = React.useState<string[]>([])

  const answer = fields.data?.state === "done" ? fields.data : null
  const listed = answer?.objects ?? []
  const dimensionChoices = listed.filter((o) => o.kind === "dimension")
  const metricChoices = listed.filter((o) => o.kind === "metric")
  const taken = reports.flatMap((r) => {
    const parsed = parseReportText(r)
    return parsed ? [parsed.label] : []
  })
  const draft = { label, dimensions, metrics }
  const problem = label.trim() === "" && dimensions.length === 0 && metrics.length === 0 ? null : reportProblem(draft, taken)
  const toggle = (set: React.Dispatch<React.SetStateAction<string[]>>) => (name: string) =>
    set((current) => (current.includes(name) ? current.filter((n) => n !== name) : [...current, name]))
  const timeChosen = dimensions.some((d) => (GA4_TIME_DIMENSIONS as readonly string[]).includes(d))

  function add() {
    if (reportProblem(draft, taken) !== null) return
    const full = withTimeDimension(dimensions)
    onAdd(`${label.trim()} = ${full.join(", ")} | ${metrics.join(", ")}`, cursorOf(full), full)
    setLabel("")
    setDimensions([])
    setMetrics([])
  }

  return (
    <div className="space-y-2 rounded-md border border-border p-3" aria-label="Reports">
      <p className="text-sm font-medium">Reports</p>
      <p className="text-xs text-muted-foreground">
        Google Analytics has no tables: a report is the dimensions and metrics you choose from this property. It loads
        one row per combination of its dimensions, each day, and only the last days are read again on later runs.
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
      <div className="flex items-center gap-2">
        <Button type="button" size="sm" variant="outline" disabled={busy || fields.status === "pending"} onClick={() => void fields.run({ schema: "fields" })}>
          {fields.status === "pending" ? "Reading the property…" : "Load dimensions and metrics"}
        </Button>
      </div>
      {fields.status === "error" ? <ErrorState error={fields.error} /> : null}
      {fields.data?.state === "failed" ? (
        <p className="text-xs text-muted-foreground">Cannot read the property · {fields.data.reason ?? "The property could not be read."}</p>
      ) : null}
      {answer ? (
        <div className="space-y-2">
          <div className="space-y-1">
            <Label htmlFor="ga4-report-label" className="text-xs">
              Report name
            </Label>
            <Input id="ga4-report-label" value={label} onChange={(e) => setLabel(e.target.value)} className="h-8 max-w-xs font-mono text-xs" autoComplete="off" />
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <Choices title="Dimensions" kind="dimension" all={dimensionChoices} chosen={new Set(dimensions)} onToggle={toggle(setDimensions)} />
            <Choices title="Metrics" kind="metric" all={metricChoices} chosen={new Set(metrics)} onToggle={toggle(setMetrics)} />
          </div>
          <p className="text-xs text-muted-foreground" data-testid="ga4-cursor">
            {timeChosen
              ? `The report is cut by ${cursorOf(dimensions)}: that is the date each run continues from.`
              : "The date is always part of a report and is the date each run continues from."}
          </p>
          {problem ? <p className="text-xs text-destructive">{problem}</p> : null}
          <Button type="button" size="sm" variant="outline" disabled={busy || reportProblem(draft, taken) !== null} onClick={add}>
            Add report
          </Button>
        </div>
      ) : null}
    </div>
  )
}
