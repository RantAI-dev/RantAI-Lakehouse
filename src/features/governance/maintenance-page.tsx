"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { governanceService } from "@/services"
import { getMaintenanceColumns } from "./maintenance-columns"

/**
 * Bronze Iceberg maintenance runs (P4/P6). Reads `GET
 * /api/governance/maintenance`, which surfaces `lake.bronze_meta.
 * maintenance_run` — written by `dagster/dispar_orchestrate/
 * maintenance.py`'s `bronze_maintenance_job`.
 *
 * Only `expire_snapshots` runs in-engine on this ClickHouse version:
 * `remove_orphan_files` does not exist for Iceberg tables and `OPTIMIZE`
 * fails at runtime with an HTTP 403 against a catalog-registered table
 * (measured in `docs/plans/G3-RESULT.md`). Small-file compaction for Bronze
 * runs out-of-band via the Trino-as-cron escape hatch (ADR 0009) — this
 * page does not surface Trino's run history because it has no `bronze_
 * meta.*` record of its own; only the in-engine `expire_snapshots` chain
 * (dry-run + applied) is tracked here, honestly.
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
        title="Bronze Maintenance"
        description="expire_snapshots dry-run and applied metrics per Bronze Iceberg table. remove_orphan_files and in-engine OPTIMIZE do not work on this ClickHouse version — see the Trino-as-cron escape hatch (ADR 0009) for small-file compaction."
      />
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
    </div>
  )
}

