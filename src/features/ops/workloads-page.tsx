"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { WorkloadStatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatCost, formatDuration, formatRelativeTime } from "@/lib/format"
import {
  ENGINE_CATEGORY_LABEL,
  WORKLOAD_CLASS_LABEL,
  WORKLOAD_STATUS_LABEL,
  type WorkloadClass,
  type WorkloadStatus,
} from "@/lib/status"
import { opsService } from "@/services"
import type { WorkloadItem } from "@/services/contracts/ops"

const STATUS_OPTIONS = (
  Object.keys(WORKLOAD_STATUS_LABEL) as WorkloadStatus[]
).map((s) => ({ value: s, label: WORKLOAD_STATUS_LABEL[s] }))

const CLASS_OPTIONS = (
  Object.keys(WORKLOAD_CLASS_LABEL) as WorkloadClass[]
).map((c) => ({ value: c, label: WORKLOAD_CLASS_LABEL[c] }))

export function WorkloadsPage() {
  const state = useService((s) => opsService.listWorkloads(s), [])
  const [search, setSearch] = React.useState("")
  const [status, setStatus] = React.useState("all")
  const [workloadClass, setWorkloadClass] = React.useState("all")
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

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((w) => {
      if (status !== "all" && w.status !== status) return false
      if (workloadClass !== "all" && w.class !== workloadClass) return false
      if (!q) return true
      return [w.principal, w.tenant].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, status, workloadClass])

  const columns = React.useMemo<ColumnDef<WorkloadItem>[]>(
    () => [
      { key: "principal", header: "Principal", render: (r) => r.principal },
      { key: "tenant", header: "Tenant", render: (r) => r.tenant },
      {
        key: "class",
        header: "Class",
        render: (r) => WORKLOAD_CLASS_LABEL[r.class],
      },
      {
        key: "engine",
        header: "Engine",
        render: (r) => ENGINE_CATEGORY_LABEL[r.engine],
      },
      {
        key: "status",
        header: "Status",
        render: (r) => (
          <div>
            <WorkloadStatusBadge status={r.status} />
            {r.queueReason ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {r.queueReason}
              </p>
            ) : null}
          </div>
        ),
      },
      {
        key: "started",
        header: "Started",
        render: (r) => formatRelativeTime(r.startedAt),
      },
      {
        key: "elapsed",
        header: "Elapsed",
        render: (r) => formatDuration(r.elapsedMs),
      },
      {
        key: "cost",
        header: "Est. cost",
        render: (r) => formatCost(r.estimatedCost),
      },
      {
        key: "actions",
        header: "Actions",
        render: (r) => {
          if (r.status !== "queued" && r.status !== "running") return null
          const isCancelling =
            cancellingId === r.id && cancelAction.status === "pending"
          return (
            <Button
              variant="outline"
              size="sm"
              disabled={isCancelling}
              onClick={() => handleCancel(r.id)}
            >
              {isCancelling ? "Cancelling…" : "Cancel"}
            </Button>
          )
        },
      },
    ],
    [cancellingId, cancelAction.status, handleCancel]
  )

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Workloads"
        description="Active and queued requests with class, engine category, cost, and fairness signals."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search principal, tenant..."
        />
        <FilterSelect
          value={status}
          onChange={setStatus}
          options={STATUS_OPTIONS}
          allLabel="All statuses"
          ariaLabel="Filter by status"
        />
        <FilterSelect
          value={workloadClass}
          onChange={setWorkloadClass}
          options={CLASS_OPTIONS}
          allLabel="All classes"
          ariaLabel="Filter by workload class"
        />
      </FilterToolbar>
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
        <DataTable columns={columns} rows={filtered} rowKey={(r) => r.id} />
      ) : null}
    </div>
  )
}
