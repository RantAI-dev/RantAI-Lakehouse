"use client"

import * as React from "react"
import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { ChartColumn, Columns3, Copy, Ellipsis, Filter, Info, ScrollText, Search } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { MetadataList } from "@/components/patterns/metadata-list"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useDataTable } from "@/hooks/use-data-table"
import { cellText as formatCellText } from "@/lib/cell-format"
import { copyTextWithToast } from "@/lib/copy-text"
import { DEFAULT_NUMBER_LOCALE } from "@/lib/formatting-settings"
import { toTsv } from "@/lib/csv"
import { formatBytes, formatCost, formatDuration, formatNumber } from "@/lib/format"
import { queryEngineLabel, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type {
  QueryCell,
  QueryDownloadFormat,
  QueryResult,
} from "@/services/contracts/queries"
import { QueryCellInspector } from "./query-cell-inspector"
import { QueryColumnFilters } from "./query-column-filters"
import { QueryColumnStatistics } from "./query-column-statistics"
import { DOWNLOADS_OFF_REASON, QueryDownloadMenu } from "./query-download-menu"
import { QueryResultBanner } from "./query-result-banner"
import {
  cellText,
  compareNumericCells,
  filterRows,
  formatDecimalText,
  isNumericColumn,
  type QueryRow,
} from "./query-grid-tools"
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

/**
 * How a cell reads (`UX-9` browser review, SHOULD-FIX 2): a number with the
 * deployment's number locale (Settings → Reporting), through the dashboards'
 * own table formatter (`lib/cell-format.ts`), and numeric text — ClickHouse
 * quotes 64-bit integers — with the same separators and every digit kept.
 * The value itself is never changed: Copy as TSV, the inspector and the
 * filters read the cell as the engine sent it.
 */
function renderCell(value: QueryCell, numeric: boolean, locale: string): React.ReactNode {
  if (value === null) return <span className="text-muted-foreground">NULL</span>
  if (typeof value === "number") return formatCellText(value, undefined, locale)
  if (typeof value === "boolean") return String(value)
  return numeric ? formatDecimalText(value, locale) : value
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
  onInspect: (column: string, value: QueryCell) => void,
  locale: string
): ColumnDef<Record<string, QueryCell>>[] {
  return columnNames.map((colName) => {
    // Decided from the data, not from a guess about the column name: a
    // column is numeric when a value arrives as a number or every value
    // reads as one (`isNumericColumn`, the statistics' own rule) — ClickHouse
    // sends a 64-bit sum as text, and before this "3010348" sat left-aligned
    // with no separators (UX-9 browser review). Numeric columns sort by
    // value and sit right-aligned, the way they do everywhere else in the
    // console.
    const numeric = isNumericColumn(rows.map((row) => row[colName] ?? null))
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
        // The shared table has no cell click, so the inspector is opened
        // from a button of its own. It shows on the row's hover or focus
        // only (UX-9 browser review): at rest the value is the whole cell.
        // Its name carries the row as well as the column: the grid holds
        // thousands of rows, and "Inspect city" alone leaves a reader who
        // cannot see the screen with hundreds of buttons that read the same.
        const inspect = (
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={`Inspect ${colName}, row ${row.index + 1}`}
            onClick={() => onInspect(colName, value)}
            className="shrink-0 opacity-0 transition-opacity duration-150 ease-ui group-hover/row:opacity-100 group-focus-within/row:opacity-100 focus-visible:opacity-100"
          >
            <Search className="size-3" aria-hidden />
          </Button>
        )
        return numeric ? (
          // The button sits before a number, so the digits stay flush right.
          <span className="flex w-full items-center justify-end gap-1">
            {inspect}
            <span className="text-[13px] tabular-nums">{renderCell(value, true, locale)}</span>
          </span>
        ) : (
          <span className="flex items-center gap-1">
            <span className="min-w-0 truncate text-[13px]">{renderCell(value, false, locale)}</span>
            {inspect}
          </span>
        )
      },
      enableSorting: true,
      sortingFn: numeric
        ? (a, b, id) => compareNumericCells(a.getValue(id) as QueryCell, b.getValue(id) as QueryCell)
        : "alphanumeric",
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
  /**
   * `UX-9`: "Make a chart" from this result — the statement saved as a SQL
   * source, then the chart builder on it. Absent when the person may not
   * author SQL sources or build a chart.
   */
  readonly onMakeChart?: () => void
  /**
   * The deployment's number locale (Settings → Reporting), for the grid's
   * numbers; `en-US`, the server's default, until it is known.
   */
  readonly numberLocale?: string
}

/** The run's own numbers in one line: what came back and what it cost. */
function metricsLine(result: QueryResult): string {
  const m = result.metrics
  const rows = formatNumber(result.rows.length)
  const parts = [
    // QS-11 (LH-QS-2-01), D12: count the rows actually on screen; the
    // server's rowCount is what the engine sent before the cut.
    result.truncated ? `${rows} rows shown` : `${rows} ${result.rows.length === 1 ? "row" : "rows"}`,
    formatDuration(m.durationMs),
    `${formatBytes(m.scannedBytes)} scanned`,
    formatCost(m.costUnits),
    // QA LH-QS-2-10 (QS-6), decision 8: the engine that really ran, as the
    // engine menu spells it.
    queryEngineLabel(result.engine),
  ]
  // A Trino run has no ClickHouse workload class, and ClickHouse's response
  // carries no cache-hit flag: an absent fact is left out rather than
  // written as a guess ("Miss") or a dash in a sentence.
  if (m.workloadClass) parts.push(WORKLOAD_CLASS_LABEL[m.workloadClass])
  if (m.cacheHit !== null) parts.push(m.cacheHit ? "cache hit" : "cache miss")
  return parts.join(" · ")
}

/**
 * The run's details that do not fit the line: pushdowns, policy
 * obligations and the plan the engine ran. A popover from the info button
 * beside the line, so each number is on the page once.
 */
function RunDetails({ result }: { result: QueryResult }) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="Run details"
          title="Run details"
          className="text-muted-foreground aria-expanded:bg-muted"
        >
          <Info className="size-3.5" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 gap-3">
        <MetadataList
          columns={1}
          items={[
            { label: "Pushdowns", value: <PillList values={result.metrics.pushdowns} /> },
            {
              label: "Policy obligations",
              value: <PillList values={result.metrics.policyObligations} />,
            },
          ]}
        />
        <QueryPlanPanel plan={result.plan} />
      </PopoverContent>
    </Popover>
  )
}

/**
 * A run's result (`UX-9`, layout A): one header row — the run's numbers,
 * then Filter rows, Copy as TSV, Download and ⋯ (Make a chart, Column
 * statistics, View in Audit) — and the rows in the console's one data
 * table under it.
 */
export function QueryResultsSection(props: QueryResultsSectionProps) {
  const { result } = props
  const [filters, setFilters] = React.useState<Record<string, string>>({})
  const [filtering, setFiltering] = React.useState(false)
  const [statistics, setStatistics] = React.useState(false)
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
    () =>
      buildQueryResultColumns(
        result.columns,
        result.rows,
        onInspect,
        props.numberLocale ?? DEFAULT_NUMBER_LOCALE
      ),
    [result.columns, result.rows, onInspect, props.numberLocale]
  )

  const { table } = useDataTable({
    data: rows,
    columns,
    pageCount: 1,
    getRowId: (_row, index) => String(index),
  })

  async function copyAsTsv() {
    // The one toast pair every copy site shares (`UX-6` review R1).
    await copyTextWithToast(toTsvText(result.columns, rows), "Copied the rows as TSV")
  }

  // QA LH-QS-2-06 (QS-9), decision 10: the copy is an export of the rows like
  // the file is, so a role whose downloads are switched off gets neither.
  // An absent flag is an older server, which counts as allowed.
  const canDownload = result.canDownload !== false
  const hasFilters = Object.values(filters).some((text) => text !== "")

  return (
    <section aria-label="Results" className="space-y-3">
      <div className="flex min-h-8 flex-wrap items-center gap-x-1 gap-y-1">
        <p className="min-w-0 flex-1 truncate text-xs text-muted-foreground tabular-nums">
          <span className="font-medium text-foreground">{metricsLine(result)}</span>
        </p>
        <RunDetails result={result} />
        <div className="ml-auto flex items-center gap-0.5">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Filter rows"
            title={filtering && hasFilters ? "Clear the filters and hide them" : "Filter rows"}
            aria-pressed={filtering}
            className="aria-pressed:bg-muted"
            onClick={() => {
              // Hiding the fields clears them: a filter nobody can see
              // would leave rows missing with no word about why.
              if (filtering) setFilters({})
              setFiltering(!filtering)
            }}
          >
            <Filter aria-hidden />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Copy as TSV"
            disabled={rows.length === 0 || !canDownload}
            title={canDownload ? "Copy as TSV" : DOWNLOADS_OFF_REASON}
            onClick={() => void copyAsTsv()}
          >
            <Copy aria-hidden />
          </Button>
          <QueryDownloadMenu
            resultId={props.runId === null ? null : result.id}
            engine={result.engine}
            busy={props.downloadBusy}
            canDownload={canDownload}
            onDownload={props.onDownload}
          />
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label="More result actions"
                  className="aria-expanded:bg-muted"
                />
              }
            >
              <Ellipsis aria-hidden />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-52">
              {props.onMakeChart ? (
                <DropdownMenuItem onClick={props.onMakeChart}>
                  <ChartColumn aria-hidden />
                  Make a chart
                </DropdownMenuItem>
              ) : null}
              <DropdownMenuItem onClick={() => setStatistics((open) => !open)}>
                <Columns3 aria-hidden />
                {statistics ? "Hide column statistics" : "Column statistics"}
              </DropdownMenuItem>
              {result.auditEventId ? (
                <DropdownMenuItem render={<Link href={`/audit?event=${result.auditEventId}`} />}>
                  <ScrollText aria-hidden />
                  View in Audit
                </DropdownMenuItem>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      {!canDownload ? (
        <p className="text-xs text-muted-foreground">{DOWNLOADS_OFF_REASON}</p>
      ) : null}
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
      {filtering ? (
        <QueryColumnFilters
          columns={result.columns}
          filters={filters}
          onFiltersChange={setFilters}
        />
      ) : null}
      {statistics ? <QueryColumnStatistics columns={result.columns} rows={rows} /> : null}
      {rows.length === 0 ? (
        <p className="py-6 text-center text-sm text-muted-foreground">
          {result.rows.length === 0
            ? "The query returned no rows."
            : "No row matches the filters."}
        </p>
      ) : (
        <div className="overflow-hidden rounded-lg border border-border">
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
    </section>
  )
}
