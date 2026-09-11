"use client"

import { useState } from "react"
import Link from "next/link"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatRelativeTime } from "@/lib/format"
import { lakehouseTableHref } from "@/lib/lakehouse-view"
import { fmtMeasured } from "@/lib/measured"
import { lakehouseService } from "@/services"
import type { LakehouseTableSummary } from "@/services/contracts/lakehouse"

const tableColumns: ColumnDef<LakehouseTableSummary>[] = [
  {
    key: "name",
    header: "Table",
    render: (t) => (
      <Link
        href={lakehouseTableHref(t.namespace, t.name)}
        className="font-mono text-sm text-primary hover:underline"
      >
        {t.name}
      </Link>
    ),
  },
  {
    key: "formatVersion",
    header: "Format",
    render: (t) => fmtMeasured(t.formatVersion),
  },
  {
    key: "currentSnapshotId",
    header: "Current snapshot",
    // Not a `Measured` (it's an id, not a metric) — render the string as
    // is, never through `Number()`, so it keeps its full 64-bit precision.
    render: (t) => t.currentSnapshotId ?? "—",
  },
  {
    key: "lastUpdatedAt",
    header: "Last updated",
    render: (t) => (t.lastUpdatedAt === null ? "—" : formatRelativeTime(t.lastUpdatedAt)),
  },
  { key: "fileCount", header: "Files", render: (t) => fmtMeasured(t.fileCount) },
  { key: "recordCount", header: "Records", render: (t) => fmtMeasured(t.recordCount) },
  {
    key: "totalBytes",
    header: "Size",
    render: (t) => fmtMeasured(t.totalBytes, formatBytes),
  },
]

/**
 * The Lakehouse tables surface: the single configured warehouse, its
 * namespaces (with table counts where the fan-out finished in budget), and
 * the tables grid for a selected namespace (WS2 §4).
 */
export function LakehouseTablesPage() {
  const warehousesState = useService((s) => lakehouseService.listWarehouses(s), [])
  const namespacesState = useService((s) => lakehouseService.listNamespaces(undefined, s), [])
  const [selectedNamespace, setSelectedNamespace] = useState<string | null>(null)
  const tablesState = useService(
    (s) =>
      selectedNamespace === null
        ? Promise.resolve([])
        : lakehouseService.listTables(selectedNamespace, undefined, s),
    [selectedNamespace]
  )
  const warehouse = warehousesState.status === "success" ? warehousesState.data[0] : undefined

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Lakehouse tables"
        description="The Iceberg warehouse, namespaces, and tables served by Lakekeeper."
      />

      <SectionCard size="sm" title="Warehouse">
        {warehousesState.status === "loading" ? <LoadingSkeleton rows={1} /> : null}
        {warehousesState.status === "error" ? (
          <ErrorState error={warehousesState.error} onRetry={warehousesState.reload} />
        ) : null}
        {warehousesState.status === "success" ? (
          warehouse === undefined ? (
            <EmptyState title="No warehouse configured" className="py-4" />
          ) : (
            <div className="flex items-center gap-2 text-sm">
              <span className="font-mono">{warehouse.name}</span>
              <Pill tone={warehouse.reachable ? "success" : "destructive"}>
                {warehouse.reachable ? "Reachable" : "Unreachable"}
              </Pill>
            </div>
          )
        ) : null}
      </SectionCard>

      <SectionCard
        size="sm"
        title="Namespaces"
        description="Select a namespace to list its tables."
      >
        {namespacesState.status === "loading" ? <LoadingSkeleton rows={2} /> : null}
        {namespacesState.status === "error" ? (
          <ErrorState error={namespacesState.error} onRetry={namespacesState.reload} />
        ) : null}
        {namespacesState.status === "success" ? (
          namespacesState.data.length === 0 ? (
            <EmptyState title="No namespaces" className="py-4" />
          ) : (
            <div className="flex flex-wrap gap-2">
              {namespacesState.data.map((ns) => (
                <Button
                  key={ns.name}
                  type="button"
                  size="sm"
                  variant={selectedNamespace === ns.name ? "default" : "outline"}
                  onClick={() => setSelectedNamespace(ns.name)}
                >
                  {ns.name} · {fmtMeasured(ns.tableCount)}
                </Button>
              ))}
            </div>
          )
        ) : null}
      </SectionCard>

      {selectedNamespace === null ? null : (
        <SectionCard size="sm" title={`Tables in ${selectedNamespace}`}>
          {tablesState.status === "loading" ? <LoadingSkeleton /> : null}
          {tablesState.status === "error" ? (
            <ErrorState error={tablesState.error} onRetry={tablesState.reload} />
          ) : null}
          {tablesState.status === "success" ? (
            <DataTable columns={tableColumns} rows={tablesState.data} rowKey={(t) => t.name} />
          ) : null}
        </SectionCard>
      )}
    </div>
  )
}
