"use client"

import Link from "next/link"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import * as React from "react"
import { useService } from "@/hooks/use-service"
import { formatDate } from "@/lib/format"
import { governanceService } from "@/services"
import type { ColumnLineage, ColumnMapping } from "@/services/contracts/governance"

/** `table.column`, or the part we know; "unknown" when the statement did not let us name it. */
export function columnLabel(end: { table: string | null; column: string | null }): string {
  if (end.table && end.column) return `${end.table}.${end.column}`
  return end.column ?? end.table ?? "unknown"
}

/** Who recorded a mapping, in words a reader can place. */
export function producerLabel(producer: ColumnMapping["producer"]): string {
  if (producer.kind === "pipeline") return `pipeline ${producer.ref}`
  if (producer.kind === "sql_source") return `SQL source ${producer.ref}`
  if (producer.kind === "build") return `build of ${producer.ref}`
  return producer.kind
}

/** The sentence under a column's lineage: how long records are kept and how far back these go. */
export function retentionLine(retention: ColumnLineage["retention"]): string {
  const kept = `Column lineage is kept for ${retention.days} days`
  if (!retention.oldestRecordedAt) return `${kept}; nothing is recorded yet for what you can see.`
  return `${kept}; the oldest record you can see is from ${formatDate(retention.oldestRecordedAt)}.`
}

/**
 * Mappings as a list: `source → target`, how the column is computed, a
 * "partial" pill (the reason is its title and is also written out, so it is
 * not hover-only) and who recorded it.
 */
export function MappingList({ mappings, showHop = false }: { mappings: ColumnMapping[]; showHop?: boolean }) {
  return (
    <ul className="divide-y divide-border text-sm">
      {mappings.map((m) => (
        <li
          key={`${m.producer.kind}-${m.producer.ref}-${m.from.table}-${m.from.column}-${m.to.table}-${m.to.column}`}
          className="flex flex-wrap items-baseline gap-x-2 gap-y-1 py-1.5"
        >
          {showHop ? <span className="text-xs text-muted-foreground">hop {m.hop}</span> : null}
          <span className="font-mono text-xs">{columnLabel(m.from)}</span>
          <span className="text-muted-foreground">→</span>
          <span className="font-mono text-xs">{columnLabel(m.to)}</span>
          <span className="text-xs text-muted-foreground">{m.transform}</span>
          {m.partial ? (
            <Pill tone="warning" title={m.reason ?? undefined}>
              partial
            </Pill>
          ) : null}
          <span className="ml-auto text-xs text-muted-foreground">{producerLabel(m.producer)}</span>
          {m.partial && m.reason ? (
            <span className="basis-full text-xs text-muted-foreground">{m.reason}</span>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

function Side({ title, empty, mappings }: { title: string; empty: string; mappings: ColumnMapping[] }) {
  return (
    <div>
      <h4 className="mb-1 text-sm font-medium">{title}</h4>
      {mappings.length === 0 ? (
        <p className="py-2 text-sm text-muted-foreground">{empty}</p>
      ) : (
        <MappingList mappings={mappings} showHop />
      )}
    </div>
  )
}

function Charts({ down }: { down: ColumnLineage }) {
  if (down.chartsHidden) {
    return <p className="py-2 text-sm text-muted-foreground">Listing dashboards needs dashboard access.</p>
  }
  if (down.chartsUnavailable) {
    return <p className="py-2 text-sm text-muted-foreground">Charts could not be read just now. Try again in a moment.</p>
  }
  const charts = down.charts ?? []
  if (charts.length === 0) {
    return <p className="py-2 text-sm text-muted-foreground">No saved chart reads this column or anything built from it.</p>
  }
  return (
    <ul className="divide-y divide-border text-sm">
      {charts.map((c) => (
        <li key={c.chartId} className="flex flex-wrap items-baseline gap-x-2 py-1.5">
          <span>{c.chartTitle}</span>
          <span className="text-muted-foreground">on</span>
          <Link href={`/dashboards?board=${encodeURIComponent(c.dashboardId)}`} className="text-primary hover:underline">
            {c.dashboardTitle}
          </Link>
        </li>
      ))}
    </ul>
  )
}

function Result({ up, down }: { up: ColumnLineage; down: ColumnLineage }) {
  // Both answers are scoped by the same rules, so the notes are the same;
  // show them once.
  const notes = [...new Set([...(up.coverage ?? []), ...(down.coverage ?? [])])]
  return (
    <div className="flex flex-col gap-4">
      <Side title="Upstream" empty="Nothing is recorded as feeding this column." mappings={up.mappings} />
      <Side title="Downstream" empty="Nothing is recorded as built from this column." mappings={down.mappings} />
      <div>
        <h4 className="mb-1 text-sm font-medium">Charts and dashboards</h4>
        <Charts down={down} />
      </div>
      {notes.map((note) => (
        <p key={note} className="text-xs text-muted-foreground">
          {note}
        </p>
      ))}
      <p className="text-xs text-muted-foreground">{retentionLine(down.retention)}</p>
    </div>
  )
}

/**
 * The column view of the Lineage tab (`DATA-13`): pick a column of the
 * table, see which columns feed it, which are built from it, and which saved
 * charts and dashboards read it. Nothing is requested until a column is
 * picked.
 */
export function ColumnLineageCard({ tableKey, columns }: { tableKey: string; columns: string[] }) {
  const [column, setColumn] = React.useState("")
  const state = useService(
    (signal) =>
      column === ""
        ? Promise.resolve(null)
        : Promise.all([
            governanceService.getColumnLineage(tableKey, column, "up", signal),
            governanceService.getColumnLineage(tableKey, column, "down", signal),
          ]),
    [tableKey, column]
  )
  const pickerId = "column-lineage-picker"
  return (
    <SectionCard
      size="sm"
      title="Column lineage"
      description="Which source columns feed a column, and what is built from it."
    >
      <div className="mb-3 grid max-w-sm gap-1.5">
        <Label htmlFor={pickerId}>Column</Label>
        <select
          id={pickerId}
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={column}
          onChange={(e) => setColumn(e.target.value)}
        >
          <option value="">Pick a column</option>
          {columns.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </div>
      {column === "" ? (
        <EmptyState title="Pick a column" description="Its upstream and downstream columns appear here." className="py-4" />
      ) : state.status === "loading" ? (
        <div className="flex gap-3 py-1" role="status" aria-label="Loading column lineage">
          <Skeleton className="h-10 w-64" />
          <Skeleton className="h-10 w-64" />
        </div>
      ) : state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : state.data === null ? null : (
        <Result up={state.data[0]} down={state.data[1]} />
      )}
    </SectionCard>
  )
}
