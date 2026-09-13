"use client"

import * as React from "react"
import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { Download } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { MetadataList } from "@/components/patterns/metadata-list"
import { Pill } from "@/components/patterns/status-badge"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { downloadCsv, toCsv } from "@/lib/csv"
import { formatBytes, formatCost, formatDuration } from "@/lib/format"
import { ENGINE_CATEGORY_LABEL, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type { QueryResult } from "@/services/contracts/queries"
import { QueryPlanPanel } from "./query-transparency-panel"

interface PillListProps {
  readonly values: readonly string[]
}

function PillList(props: PillListProps) {
  const { values } = props
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

function buildQueryResultColumns(
  columnNames: readonly string[]
): ColumnDef<Record<string, string>>[] {
  return columnNames.map((colName) => ({
    id: colName,
    accessorFn: (row) => row[colName] ?? "",
    header: ({ column }) => (
      <DataTableColumnHeader column={column} label={colName} />
    ),
    cell: ({ getValue }) => {
      const val = getValue()
      return (
        <span className="font-mono text-xs">
          {typeof val === "string" ? val : JSON.stringify(val ?? "")}
        </span>
      )
    },
    enableSorting: true,
  }))
}

interface QueryResultsSectionProps {
  readonly result: QueryResult
}

/** Query result rows plus the actual execution metrics beneath them. */
export function QueryResultsSection(props: QueryResultsSectionProps) {
  const { result } = props
  const columns = React.useMemo(
    () => buildQueryResultColumns(result.columns),
    [result.columns]
  )

  const { table } = useDataTable({
    data: result.rows,
    columns,
    pageCount: 1,
    getRowId: (_row, index) => String(index),
  })

  return (
    <SectionCard
      title="Results"
      contentClassName="space-y-4"
      action={
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={result.rows.length === 0}
            onClick={() =>
              downloadCsv(
                `query-results-${new Date().toISOString().slice(0, 10)}.csv`,
                toCsv(result.columns, result.rows)
              )
            }
          >
            <Download className="size-4" aria-hidden />
            Export CSV
          </Button>
          {result.auditEventId ? (
            <Button
              size="sm"
              variant="outline"
              render={<Link href={`/audit?event=${result.auditEventId}`} />}
            >
              View in Audit
            </Button>
          ) : null}
        </div>
      }
    >
      {result.rows.length === 0 ? (
        <p className="py-4 text-center text-sm text-muted-foreground">
          The query returned no rows.
        </p>
      ) : (
        <div className="rounded-md border">
          <DataTable
            table={table}
            infinite={{
              onLoadMore: () => {},
              hasNextPage: false,
              isFetchingNextPage: false,
              totalItems: result.rows.length,
              loadedCount: result.rows.length,
            }}
          />
        </div>
      )}
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
          {
            label: "Policy obligations",
            value: <PillList values={result.metrics.policyObligations} />,
          },
        ]}
      />
      <QueryPlanPanel plan={result.plan} />
    </SectionCard>
  )
}
