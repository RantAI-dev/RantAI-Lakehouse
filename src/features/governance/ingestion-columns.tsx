"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Activity,
  Cable,
  Clock,
  Copy,
  Database,
  HardDrive,
  Layers,
  MoreHorizontal,
  Zap,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatBytes, formatRelativeTime } from "@/lib/format"
import type { ReplicationSlot } from "@/services/contracts/governance"

const STATUS_TONE: Record<
  string,
  "success" | "warning" | "destructive" | "neutral"
> = {
  ok: "success",
  warning: "warning",
  critical: "destructive",
}

export function getIngestionColumns(): ColumnDef<ReplicationSlot>[] {
  return [
    {
      id: "connectorId",
      accessorKey: "connectorId",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Connector" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Cable className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-mono text-sm font-medium text-foreground">
            {row.original.connectorId}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Connector",
        placeholder: "Filter connector…",
        variant: "text",
        icon: Cable,
      },
    },
    {
      id: "slotName",
      accessorKey: "slotName",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Replication Slot" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Layers className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-mono text-xs text-muted-foreground">
            {row.original.slotName}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Replication Slot",
        placeholder: "Filter slot…",
        variant: "text",
        icon: Layers,
      },
    },
    {
      id: "active",
      accessorKey: "active",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Active" />
      ),
      cell: ({ row }) => (
        <Pill tone={row.original.active ? "success" : "destructive"}>
          {row.original.active ? "connected" : "disconnected"}
        </Pill>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Active",
        variant: "select",
        options: [
          { value: "true", label: "connected" },
          { value: "false", label: "disconnected" },
        ],
        icon: Zap,
      },
    },
    {
      id: "walRetainedBytes",
      accessorFn: (r) => Number(r.walRetainedBytes) || 0,
      header: ({ column }) => (
        <DataTableColumnHeader
          column={column}
          label="WAL Retained"
          align="right"
        />
      ),
      cell: ({ row }) => (
        <div className="text-right font-mono text-xs text-foreground">
          {formatBytes(Number(row.original.walRetainedBytes) || 0)}
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "WAL Retained",
        variant: "number",
        icon: HardDrive,
      },
    },
    {
      id: "confirmedFlushLagBytes",
      accessorFn: (r) => Number(r.confirmedFlushLagBytes) || 0,
      header: ({ column }) => (
        <DataTableColumnHeader
          column={column}
          label="Flush Lag"
          align="right"
        />
      ),
      cell: ({ row }) => (
        <div className="text-right font-mono text-xs text-foreground">
          {formatBytes(Number(row.original.confirmedFlushLagBytes) || 0)}
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Flush Lag",
        variant: "number",
        icon: Database,
      },
    },
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => (
        <Pill tone={STATUS_TONE[row.original.status] ?? "neutral"}>
          {row.original.status}
        </Pill>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Status",
        variant: "select",
        options: [
          { value: "ok", label: "ok" },
          { value: "warning", label: "warning" },
          { value: "critical", label: "critical" },
        ],
        icon: Activity,
      },
    },
    {
      id: "checkedAt",
      accessorKey: "checkedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Checked" />
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {formatRelativeTime(row.original.checkedAt)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Checked",
        variant: "text",
        icon: Clock,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const item = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Ingestion actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(item.connectorId)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy connector ID</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(item.slotName)
                  }}
                >
                  <Layers className="size-4" />
                  <span>Copy slot name</span>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
      enableColumnFilter: false,
      enableSorting: false,
      enableHiding: false,
      size: 48,
      meta: {
        fitContent: true,
      },
    },
  ]
}
