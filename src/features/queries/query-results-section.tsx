"use client"

import * as React from "react"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { MetadataList } from "@/components/patterns/metadata-list"
import { Pill } from "@/components/patterns/status-badge"
import { SectionCard } from "@/components/patterns/section-card"
import { formatBytes, formatCost, formatDuration } from "@/lib/format"
import { ENGINE_CATEGORY_LABEL, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type { QueryResult } from "@/services/contracts/queries"

type ResultRow = { key: string; cells: Record<string, string> }

function PillList({ values }: { values: string[] }) {
  if (values.length === 0) return <span className="text-muted-foreground">None</span>
  return (
    <span className="flex flex-wrap gap-1">
      {values.map((v) => (
        <Pill key={v} tone="neutral">
          {v}
        </Pill>
      ))}
    </span>
  )
}

/** Query result rows plus the actual execution metrics beneath them. */
export function QueryResultsSection({ result }: { result: QueryResult }) {
  const columns = React.useMemo<ColumnDef<ResultRow>[]>(
    () =>
      result.columns.map((c) => ({
        key: c,
        header: c,
        render: (row) => (
          <span className="font-mono text-xs">{row.cells[c]}</span>
        ),
      })),
    [result.columns]
  )
  const rows = React.useMemo<ResultRow[]>(
    () => result.rows.map((cells, i) => ({ key: String(i), cells })),
    [result.rows]
  )

  return (
    <SectionCard title="Results" contentClassName="space-y-4">
      <DataTable
        columns={columns}
        rows={rows}
        rowKey={(r) => r.key}
        emptyMessage="The query returned no rows."
      />
      <MetadataList
        columns={3}
        items={[
          { label: "Duration", value: formatDuration(result.metrics.durationMs) },
          { label: "Scanned", value: formatBytes(result.metrics.scannedBytes) },
          { label: "Cost", value: formatCost(result.metrics.costUnits) },
          { label: "Engine", value: ENGINE_CATEGORY_LABEL[result.metrics.engine] },
          { label: "Workload", value: WORKLOAD_CLASS_LABEL[result.metrics.workloadClass] },
          { label: "Cache", value: result.metrics.cacheHit ? "Hit" : "Miss" },
          { label: "Pushdowns", value: <PillList values={result.metrics.pushdowns} /> },
          { label: "Policy obligations", value: <PillList values={result.metrics.policyObligations} /> },
        ]}
      />
    </SectionCard>
  )
}
