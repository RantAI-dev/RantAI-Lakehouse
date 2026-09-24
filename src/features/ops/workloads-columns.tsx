"use client"

import type { ColumnDef } from "@tanstack/react-table"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { WorkloadStatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { formatCost, formatDuration, formatRelativeTime } from "@/lib/format"
import {
  ENGINE_CATEGORY_LABEL,
  WORKLOAD_CLASS_LABEL,
  WORKLOAD_STATUS_LABEL,
  type EngineCategory,
  type WorkloadClass,
  type WorkloadStatus,
} from "@/lib/status"
import type { WorkloadItem } from "@/services/contracts/ops"

const STATUS_OPTIONS = (Object.keys(WORKLOAD_STATUS_LABEL) as WorkloadStatus[]).map((s) => ({
  value: s,
  label: WORKLOAD_STATUS_LABEL[s],
}))

const CLASS_OPTIONS = (Object.keys(WORKLOAD_CLASS_LABEL) as WorkloadClass[]).map((c) => ({
  value: c,
  label: WORKLOAD_CLASS_LABEL[c],
}))

const ENGINE_OPTIONS = (Object.keys(ENGINE_CATEGORY_LABEL) as EngineCategory[]).map((e) => ({
  value: e,
  label: ENGINE_CATEGORY_LABEL[e],
}))

export function getWorkloadColumns({
  onCancel,
  cancellingId,
  isCancelPending,
}: {
  onCancel: (id: string) => void
  cancellingId: string | null
  isCancelPending: boolean
}): ColumnDef<WorkloadItem>[] {
  return [
    {
      accessorKey: "principal",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Principal" />
      ),
      cell: ({ row }) => (
        <span className="font-medium">{row.original.principal}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Principal",
        variant: "text",
      },
    },
    {
      accessorKey: "tenant",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tenant" />
      ),
      cell: ({ row }) => <span>{row.original.tenant}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Tenant",
        variant: "text",
      },
    },
    {
      accessorKey: "class",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Class" />
      ),
      cell: ({ row }) => (
        // No workload classifier exists yet; null renders as "—" instead
        // of a guessed class.
        <span>
          {row.original.class === null
            ? "—"
            : WORKLOAD_CLASS_LABEL[row.original.class]}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Class",
        variant: "select",
        options: CLASS_OPTIONS,
      },
    },
    {
      accessorKey: "engine",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Engine" />
      ),
      cell: ({ row }) => (
        <span>{ENGINE_CATEGORY_LABEL[row.original.engine]}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Engine",
        variant: "select",
        options: ENGINE_OPTIONS,
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <WorkloadStatusBadge status={r.status} />
            {r.queueReason ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {r.queueReason}
              </p>
            ) : null}
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Status",
        variant: "select",
        options: STATUS_OPTIONS,
      },
    },
    {
      accessorKey: "startedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Started" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.startedAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Started",
        variant: "date",
      },
    },
    {
      accessorKey: "elapsedMs",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Elapsed" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatDuration(row.original.elapsedMs)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Elapsed",
        variant: "number",
      },
    },
    {
      accessorKey: "estimatedCost",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Est. cost" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatCost(row.original.estimatedCost)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Est. cost",
        variant: "number",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const r = row.original
        if (r.status !== "queued" && r.status !== "running") return null
        const isCancelling = cancellingId === r.id && isCancelPending

        return (
          <div className="flex items-center justify-end">
            <Button
              variant="outline"
              size="sm"
              disabled={isCancelling}
              onClick={() => onCancel(r.id)}
            >
              {isCancelling ? "Cancelling…" : "Cancel"}
            </Button>
          </div>
        )
      },
      enableSorting: false,
      enableHiding: false,
      size: 90,
    },
  ]
}
