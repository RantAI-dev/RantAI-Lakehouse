"use client"

import type { ColumnDef } from "@tanstack/react-table"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { WorkloadStatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { formatCost, formatDuration, formatRelativeTime } from "@/lib/format"
import {
  ENGINE_CATEGORY_LABEL,
  WORKLOAD_CLASS_LABEL,
} from "@/lib/status"
import type { WorkloadItem } from "@/services/contracts/ops"

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
        <DataTableColumnHeader column={column} title="Principal" />
      ),
      cell: ({ row }) => (
        <span className="font-medium">{row.original.principal}</span>
      ),
      meta: {
        label: "Principal",
        variant: "text",
      },
    },
    {
      accessorKey: "tenant",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Tenant" />
      ),
      cell: ({ row }) => <span>{row.original.tenant}</span>,
      meta: {
        label: "Tenant",
        variant: "text",
      },
    },
    {
      accessorKey: "class",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Class" />
      ),
      cell: ({ row }) => (
        <span>{WORKLOAD_CLASS_LABEL[row.original.class]}</span>
      ),
      meta: {
        label: "Class",
        variant: "select",
      },
    },
    {
      accessorKey: "engine",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Engine" />
      ),
      cell: ({ row }) => (
        <span>{ENGINE_CATEGORY_LABEL[row.original.engine]}</span>
      ),
      meta: {
        label: "Engine",
        variant: "select",
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Status" />
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
      meta: {
        label: "Status",
        variant: "select",
      },
    },
    {
      accessorKey: "startedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Started" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.startedAt)}
        </span>
      ),
      meta: {
        label: "Started",
        variant: "text",
      },
    },
    {
      accessorKey: "elapsedMs",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Elapsed" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatDuration(row.original.elapsedMs)}
        </span>
      ),
      meta: {
        label: "Elapsed",
        variant: "number",
      },
    },
    {
      accessorKey: "estimatedCost",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Est. cost" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatCost(row.original.estimatedCost)}
        </span>
      ),
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
