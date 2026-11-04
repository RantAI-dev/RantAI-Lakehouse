"use client"

import * as React from "react"
import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { Copy, Search } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { MetadataList } from "@/components/patterns/metadata-list"
import { Pill } from "@/components/patterns/status-badge"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { copyText } from "@/lib/copy-text"
import { toTsv } from "@/lib/csv"
import { formatBytes, formatCost, formatDuration, formatNumber } from "@/lib/format"
import { notifyFailure, notifySuccess } from "@/lib/notify"
import { ENGINE_CATEGORY_LABEL, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type {
  QueryCell,
  QueryDownloadFormat,
  QueryResult,
} from "@/services/contracts/queries"
import { QueryCellInspector } from "./query-cell-inspector"
import { QueryColumnFilters } from "./query-column-filters"
import { QueryColumnStatistics } from "./query-column-statistics"
import { QueryDownloadMenu } from "./query-download-menu"
import { QueryResultBanner } from "./query-result-banner"
import { cellText, filterRows, type QueryRow } from "./query-grid-tools"
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

/**
 * The rows as the tab-separated text the clipboard gets: the result's
 * columns in the result's order, and a cell that holds nothing written as
 * an empty cell rather than as the words "null".
 */
function toTsvText(columns: readonly string[], rows: readonly QueryRow[]): string {
  return toTsv(
    [...columns],
    rows.map((row) =>
      Object.fromEntries(
        columns.map((column) => {
          const cell = row[column]
          if (cell === undefined || cell === null) return [column, null]
          return [column, typeof cell === "string" ? cell : cellText(cell)]
        })
      )
    )
  )
}

function buildQueryResultColumns(
  columnNames: readonly string[],
  rows: readonly Record<string, QueryCell>[],
  onInspect: (column: string, value: QueryCell) => void
): ColumnDef<Record<string, QueryCell>>[] {
  return columnNames.map((colName) => {
    // Decided from the data, not from a guess about the column name: the
    // server sends whatever type ClickHouse reported it as, so a numeric column
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
      cell: ({ row, getValue }) => {
        const value = getValue() as QueryCell
        return (
          <span
            className={
              numeric
                ? "flex w-full items-center justify-end gap-1"
                : "flex items-center gap-1"
            }
          >
            <span
              className={
                numeric
                  ? "font-mono text-xs tabular-nums"
                  : "font-mono text-xs"
              }
            >
              {renderCell(value)}
            </span>
            {/* The shared table has no cell click, so the inspector is opened
                from a button of its own. It stays quiet: the value is the
                thing the reader came for. Its name carries the row as well
                as the column: the grid holds thousands of rows, and "Inspect
                city" alone leaves a reader who cannot see the screen with
                hundreds of buttons that read the same. */}
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              aria-label={`Inspect ${colName}, row ${row.index + 1}`}
              onClick={() => onInspect(colName, value)}
            >
              <Search className="size-3" aria-hidden />
            </Button>
          </span>
        )
      },
      enableSorting: true,
      sortingFn: numeric ? "basic" : "alphanumeric",
      meta: { label: colName },
    }
  })
}

interface QueryResultsSectionProps {
  readonly result: QueryResult
  /** The id the run of this result had, or null when there is none. */
  readonly runId: string | null
  /** Asks the server for the result as a file, in the chosen format. */
  readonly onDownload: (format: QueryDownloadFormat) => void
  /** The file kind being downloaded, or null when none is. */
  readonly downloadBusy: QueryDownloadFormat | null
}

/** Query result rows plus the actual execution metrics beneath them. */
export function QueryResultsSection(props: QueryResultsSectionProps) {
  const { result } = props
  const [filters, setFilters] = React.useState<Record<string, string>>({})
  const [inspected, setInspected] = React.useState<{
    column: string
    value: QueryCell
  } | null>(null)

  // The rows the grid shows: the ones the browser got, less the ones the
  // filters leave out. The statistics and the copy read the same rows, so
  // every number on the page describes one set of rows.
  const rows = React.useMemo(
    () => filterRows(result.rows, filters),
    [result.rows, filters]
  )

  const onInspect = React.useCallback((column: string, value: QueryCell) => {
    setInspected({ column, value })
  }, [])

  const columns = React.useMemo(
    () => buildQueryResultColumns(result.columns, result.rows, onInspect),
    [result.columns, result.rows, onInspect]
  )

  const { table } = useDataTable({
    data: rows,
    columns,
    pageCount: 1,
    getRowId: (_row, index) => String(index),
  })

  async function copyAsTsv() {
    const copied = await copyText(toTsvText(result.columns, rows))
    if (copied) {
      notifySuccess(
        "Copied the rows as TSV",
        `${formatNumber(rows.length)} rows copied to the clipboard.`
      )
    } else {
      notifyFailure(
        "Could not copy the rows",
        "The browser refused the copy."
      )
    }
  }

  return (
    <SectionCard
      title="Results"
      contentClassName="space-y-4"
      action={
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={rows.length === 0}
            onClick={() => void copyAsTsv()}
          >
            <Copy className="size-4" aria-hidden />
            Copy as TSV
          </Button>
          <QueryDownloadMenu
            resultId={props.runId === null ? null : result.id}
            engine={result.engine}
            busy={props.downloadBusy}
            onDownload={props.onDownload}
          />
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
        // no answer. The sentence names the limit that cut the result, not a
        // total the console never received.
        <QueryResultBanner
          rowCount={result.rows.length}
          rowLimit={result.rowLimit}
          cutBy={result.cutBy}
        />
      ) : null}
      <QueryColumnFilters
        columns={result.columns}
        filters={filters}
        onFiltersChange={setFilters}
      />
      <QueryColumnStatistics columns={result.columns} rows={rows} />
      {rows.length === 0 ? (
        <p className="py-4 text-center text-sm text-muted-foreground">
          {result.rows.length === 0
            ? "The query returned no rows."
            : "No row matches the filters."}
        </p>
      ) : (
        <div className="rounded-md border">
          <DataTable
            table={table}
            infinite={{
              onLoadMore: () => {},
              hasNextPage: false,
              isFetchingNextPage: false,
              totalItems: rows.length,
              loadedCount: rows.length,
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
      {inspected ? (
        <QueryCellInspector
          column={inspected.column}
          value={inspected.value}
          open
          onOpenChange={(open) => {
            if (!open) setInspected(null)
          }}
        />
      ) : null}
    </SectionCard>
  )
}
