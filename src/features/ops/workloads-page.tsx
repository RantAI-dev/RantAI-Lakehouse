"use client"

import * as React from "react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import {
  ENGINE_CATEGORY_LABEL,
  WORKLOAD_CLASS_LABEL,
  WORKLOAD_STATUS_LABEL,
  type EngineCategory,
  type WorkloadClass,
  type WorkloadStatus,
} from "@/lib/status"
import { opsService } from "@/services"
import type { WorkloadItem } from "@/services/contracts/ops"
import type { DataTableFilterField } from "@/types/data-table"
import { getWorkloadColumns } from "./workloads-columns"

const filterFields: DataTableFilterField<WorkloadItem>[] = [
  {
    id: "status",
    label: "Status",
    options: (Object.keys(WORKLOAD_STATUS_LABEL) as WorkloadStatus[]).map((s) => ({
      value: s,
      label: WORKLOAD_STATUS_LABEL[s],
    })),
  },
  {
    id: "class",
    label: "Class",
    options: (Object.keys(WORKLOAD_CLASS_LABEL) as WorkloadClass[]).map((c) => ({
      value: c,
      label: WORKLOAD_CLASS_LABEL[c],
    })),
  },
  {
    id: "engine",
    label: "Engine",
    options: (Object.keys(ENGINE_CATEGORY_LABEL) as EngineCategory[]).map((e) => ({
      value: e,
      label: ENGINE_CATEGORY_LABEL[e],
    })),
  },
]

export function WorkloadsPage() {
  const state = useService((s) => opsService.listWorkloads(s), [])
  const [cancellingId, setCancellingId] = React.useState<string | null>(null)

  const cancelAction = useServiceAction((signal, id: string) =>
    opsService.cancelWorkload(id, signal)
  )

  const handleCancel = React.useCallback(
    async (id: string) => {
      setCancellingId(id)
      const result = await cancelAction.run(id)
      setCancellingId(null)
      if (result) state.reload()
    },
    [cancelAction, state]
  )

  const columns = React.useMemo(
    () =>
      getWorkloadColumns({
        onCancel: handleCancel,
        cancellingId,
        isCancelPending: cancelAction.status === "pending",
      }),
    [handleCancel, cancellingId, cancelAction.status]
  )

  const { table } = useDataTable({
    data: state.data ?? [],
    columns,
    pageCount: 1,
    filterFields,
    enableRowSelection: false,
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
    shallow: false,
    clearOnDefault: true,
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Workloads"
        description="Active and queued requests with class, engine category, cost, and fairness signals."
      />
      {cancelAction.status === "error" ? (
        <p className="text-xs text-destructive" role="alert">
          Failed to cancel workload: {cancelAction.error.message}
        </p>
      ) : null}
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              table={table}
              placeholder="Search principal, tenant..."
              className="h-8 w-40 lg:w-64"
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}
    </div>
  )
}
