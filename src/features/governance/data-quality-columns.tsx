"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Activity,
  AlertTriangle,
  Clock,
  Copy,
  Database,
  Eye,
  Gauge,
  MoreHorizontal,
  ShieldCheck,
  SlidersHorizontal,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { CheckBadge, SeverityBadge } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import {
  CHECK_STATUS_LABEL,
  SEVERITY_LABEL,
  type CheckStatus,
  type Severity,
} from "@/lib/status"
import type { QualityRule } from "@/services/contracts/governance"

export const CHECK_OPTIONS = (
  Object.keys(CHECK_STATUS_LABEL) as CheckStatus[]
).map((s) => ({ value: s, label: CHECK_STATUS_LABEL[s] }))

export const SEVERITY_OPTIONS = (Object.keys(SEVERITY_LABEL) as Severity[]).map(
  (s) => ({ value: s, label: SEVERITY_LABEL[s] })
)

export function getDataQualityColumns({
  onSelect,
}: {
  onSelect: (rule: QualityRule) => void
}): ColumnDef<QualityRule>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Rule" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <ShieldCheck className="size-4 shrink-0 text-muted-foreground" />
          <button
            type="button"
            className="text-left font-medium text-foreground hover:underline focus:outline-none"
            onClick={() => onSelect(row.original)}
          >
            {row.original.name}
          </button>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Rule",
        placeholder: "Filter rule…",
        variant: "text",
        icon: ShieldCheck,
      },
    },
    {
      id: "asset",
      accessorKey: "asset",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Asset" />
      ),
      cell: ({ row }) => (
        <Copyable value={row.original.asset} className="max-w-48 truncate">
          <span className="font-mono text-xs text-foreground">
            {row.original.asset}
          </span>
        </Copyable>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Asset",
        placeholder: "Filter asset…",
        variant: "text",
        icon: Database,
      },
    },
    {
      id: "dimension",
      accessorKey: "dimension",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Dimension" />
      ),
      cell: ({ row }) => (
        <span className="inline-flex items-center rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
          {row.original.dimension}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Dimension",
        placeholder: "Filter dimension…",
        variant: "text",
        icon: Gauge,
      },
    },
    {
      id: "threshold",
      accessorKey: "threshold",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Threshold" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs font-medium text-foreground">
          {row.original.threshold}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Threshold",
        placeholder: "Filter threshold…",
        variant: "text",
        icon: SlidersHorizontal,
      },
    },
    {
      id: "severity",
      accessorKey: "severity",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Severity" />
      ),
      cell: ({ row }) => <SeverityBadge severity={row.original.severity} />,
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Severity",
        variant: "select",
        options: SEVERITY_OPTIONS,
        icon: AlertTriangle,
      },
    },
    {
      id: "lastStatus",
      accessorKey: "lastStatus",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last Status" />
      ),
      cell: ({ row }) => <CheckBadge status={row.original.lastStatus} />,
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Last Status",
        variant: "select",
        options: CHECK_OPTIONS,
        icon: Activity,
      },
    },
    {
      id: "lastRunAt",
      accessorKey: "lastRunAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last Run" />
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {formatRelativeTime(row.original.lastRunAt)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Last Run",
        variant: "text",
        icon: Clock,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const rule = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Quality rule actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    onSelect(rule)
                  }}
                >
                  <Eye className="size-4" />
                  <span>View details</span>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(rule.name)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy rule name</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(rule.asset)
                  }}
                >
                  <Database className="size-4" />
                  <span>Copy asset</span>
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
