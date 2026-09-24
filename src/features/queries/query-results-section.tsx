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
import { downloadCsv } from "@/lib/csv"
import { csvFileName, tableCsv } from "@/lib/table-csv"
import { formatBytes, formatCost, formatDuration, formatNumber } from "@/lib/format"
import { ENGINE_CATEGORY_LABEL, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type { QueryCell, QueryResult } from "@/services/contracts/queries"
import { QueryPlanPanel } from "./query-transparency-panel"

/**
 * `values === null` means "not measured" (no pushdown computation and no
 * policy engine exist yet) — a distinct claim from an empty list, which
 * would read as "we checked and there were none."
 */
function PillList({ values }: { values: string[] | null }) {
  if (values === null) return <span className="text-muted-foreground">Not measured</span>
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

/** How a cell reads. Numbers keep their type so they sort and align as numbers. */
function renderCell(value: QueryCell): React.ReactNode {
  if (value === null) return <span className="text-muted-foreground">NULL</span>
  if (typeof value === "number") return formatNumber(value)
  if (typeof value === "boolean") return String(value)
  return value
}

function buildQueryResultColumns(
  columnNames: readonly string[],
  rows: readonly Record<string, QueryCell>[]
): ColumnDef<Record<string, QueryCell>>[] {
  return columnNames.map((colName) => {
    // Decided from the data, not from a guess about the column name: the
    // server sends whatever type ClickHouse reported, so a numeric column
    // is one whose values arrive as numbers. Numeric columns sort
    // numerically and sit right-aligned, the way they do everywhere else
    // in the console — before this every value was a string, so "10" came
    // before "9".
    const numeric = rows.some((row) => typeof row[colName] === "number")
    return {
      id: colName,
      accessorFn: (row) => row[colName] ?? null,
      header: ({ column }) => (
        <DataTableColumnHeader
          column={column}
          label={colName}
          align={numeric ? "right" : undefined}
        />
      ),
      cell: ({ getValue }) => (
        <span
          className={
            numeric
              ? "block text-right font-mono text-xs tabular-nums"
              : "font-mono text-xs"
          }
        >
          {renderCell(getValue() as QueryCell)}
        </span>
      ),
      enableSorting: true,
      sortingFn: numeric ? "basic" : "alphanumeric",
      meta: { label: colName },
    }
  })
}

interface QueryResultsSectionProps {
  readonly result: QueryResult
}

/** Query result rows plus the actual execution metrics beneath them. */
export function QueryResultsSection(props: QueryResultsSectionProps) {
  const { result } = props
  const columns = React.useMemo(
    () => buildQueryResultColumns(result.columns, result.rows),
    [result.columns, result.rows]
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
                csvFileName("query results"),
                tableCsv(
                  result.columns.map((c) => ({ id: c, label: c })),
                  result.rows
                )
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
      {result.truncated ? (
        // Said plainly: a partial answer read as a whole one is worse than
        // no answer.
        <p className="rounded-md bg-amber-500/5 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
          Showing the first {formatNumber(result.rowLimit)} of{" "}
          {formatNumber(result.rowCount)} rows. Add a LIMIT or an aggregate
          to see the whole result.
        </p>
      ) : null}
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
          { label: "Rows", value: formatNumber(result.rowCount) },
          { label: "Duration", value: formatDuration(result.metrics.durationMs) },
          { label: "Scanned", value: formatBytes(result.metrics.scannedBytes) },
          { label: "Cost", value: formatCost(result.metrics.costUnits) },
          { label: "Engine", value: ENGINE_CATEGORY_LABEL[result.metrics.engine] },
          { label: "Workload", value: WORKLOAD_CLASS_LABEL[result.metrics.workloadClass] },
          {
            label: "Cache",
            // ClickHouse's response carries no cache-hit flag — null
            // renders "—" rather than a fabricated "Miss".
            value: result.metrics.cacheHit === null ? "—" : result.metrics.cacheHit ? "Hit" : "Miss",
          },
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
