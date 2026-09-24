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
import { getIngestionColumns } from "./ingestion-columns"

/**
 * CDC ingestion health (P5/P6): one row per Postgres logical-replication
 * slot backing a Debezium Server connector into Bronze. Reads `GET
 * /api/governance/replication`, which surfaces `lake.bronze_meta.
 * replication_slot` — written every 15 minutes by `dagster/
 * dispar_orchestrate/replication_metrics.py`'s `replication_slot_check_job`
 * (R5: a lagging or disconnected slot pins WAL and can fill the source
 * database's disk; a disconnected slot is flagged "critical" regardless of
 * byte thresholds, because it still pins WAL indefinitely).
 *
 * This is deliberately not called "Streaming" — there is no Kafka/Flink/
 * streaming engine in this stack. CDC is a batch-of-one-per-transaction
 * replication pipe into Bronze Iceberg, which is what this page monitors.
 */
export function IngestionPage() {
  const state = useService((s) => governanceService.listReplicationSlots(s), [])
  const tableUrlState = useTableUrlState()

  const columns = React.useMemo(() => getIngestionColumns(), [])

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.connectorId,
        (r) => r.slotName,
        (r) => r.status,
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
    persistKey: "/governance/ingestion",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Ingestion (CDC)"
        description="Postgres logical-replication slot health per CDC connector — WAL retention, flush lag, and connection state. Not a streaming engine: this is Debezium Server capturing row-level changes into Bronze Iceberg."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search connector, slot, status…" />
          </DataTableAdvancedToolbar>
          <DataTable table={table} />
        </div>
      ) : null}
    </div>
  )
}

