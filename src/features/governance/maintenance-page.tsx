"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { IcebergTablesSection } from "@/features/lakehouse/iceberg-tables-section"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { governanceService } from "@/services"
import { getMaintenanceColumns } from "./maintenance-columns"

/**
 * Table Maintenance: the Iceberg tables in the tenant's warehouse (each
 * links to its detail page, where the per-table maintenance policy is
 * set) and the maintenance runs that policy drives. The table list used to
 * be a separate "Tables" page under Data, where it read as a duplicate of
 * Data Explorer; setting a policy and seeing its runs now happen in one
 * place.
 *
 * Runs come from `GET /api/governance/maintenance`, which surfaces
 * `lake.bronze_meta.maintenance_run`, written by `dagster/dispar_orchestrate/
 * maintenance.py`'s `bronze_maintenance_job`. On ClickHouse 26.8 the one
 * in-engine verb that works is `remove_orphan_files` (dry-run, then
 * applied); `expire_snapshots` is refused for tables behind a REST catalog
 * and is recorded as a skipped verb every run; `OPTIMIZE` returns OK but
 * does not bin-pack, so it is not invoked (measured in
 * `docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`). Small-file compaction
 * runs out-of-band via the Trino-as-cron escape hatch (ADR 0009), which has
 * no `bronze_meta.*` record of its own and so is not shown here.
 */
export function MaintenancePage() {
  const state = useService((s) => governanceService.listMaintenanceRuns(s), [])
  const tableUrlState = useTableUrlState()

  const columns = React.useMemo(() => getMaintenanceColumns(), [])

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.tableName,
        (r) => r.skippedVerbs ?? "",
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    state.status,
    state.data,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/governance/maintenance",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Table Maintenance"
        description="Iceberg tables in your warehouse, their maintenance policy, and what each nightly maintenance run did. The run removes orphan files (dry run, then applied); snapshot expiry is refused on this ClickHouse version and shows as a skipped verb; small-file compaction runs through Trino (ADR 0009)."
      />
      <Tabs defaultValue="tables">
        <TabsList>
          <TabsTrigger value="tables">Tables</TabsTrigger>
          <TabsTrigger value="runs">Maintenance runs</TabsTrigger>
        </TabsList>
        <TabsContent value="tables" className="mt-4">
          <IcebergTablesSection />
        </TabsContent>
        <TabsContent value="runs" className="mt-4">
          {state.status === "loading" ? <LoadingSkeleton /> : null}
          {state.status === "error" ? (
            <ErrorState error={state.error} onRetry={state.reload} />
          ) : null}
          {state.status === "success" ? (
            <div className="flex flex-col gap-4">
              <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
                <DataTableSearch placeholder="Search Bronze table, skipped verbs…" />
              </DataTableAdvancedToolbar>
              <DataTable table={table} />
            </div>
          ) : null}
        </TabsContent>
      </Tabs>
    </div>
  )
}
