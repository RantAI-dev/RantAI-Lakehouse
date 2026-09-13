"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  formatBytes,
  formatCompactNumber,
  formatNumber,
  formatPercent,
} from "@/lib/format"
import type { Tenant } from "@/services/contracts/identity"

function computeQuota(r: Tenant): string {
  const used = formatCompactNumber(r.usedCompute)
  const quota = formatCompactNumber(r.quotaCompute)
  const utilization =
    r.quotaCompute > 0 ? formatPercent(r.usedCompute / r.quotaCompute) : "—"
  return `${used} / ${quota} (${utilization})`
}

export function getTenantColumns({
  onSelect,
}: {
  onSelect: (tenant: Tenant) => void
}): ColumnDef<Tenant>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Tenant" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <button
              type="button"
              onClick={() => onSelect(r)}
              className="text-left font-medium hover:underline focus:outline-none"
            >
              {r.name}
            </button>
            <p className="font-mono text-xs text-muted-foreground">{r.slug}</p>
          </div>
        )
      },
      meta: {
        label: "Tenant",
        variant: "text",
      },
    },
    {
      accessorKey: "plan",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Plan" />
      ),
      cell: ({ row }) => <span>{row.original.plan}</span>,
      meta: {
        label: "Plan",
        variant: "select",
      },
    },
    {
      accessorKey: "residency",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Residency" />
      ),
      cell: ({ row }) => <span>{row.original.residency}</span>,
      meta: {
        label: "Residency",
        variant: "select",
      },
    },
    {
      accessorKey: "users",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Users" />
      ),
      cell: ({ row }) => <span>{formatNumber(row.original.users)}</span>,
      meta: {
        label: "Users",
        variant: "number",
      },
    },
    {
      accessorKey: "agents",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Agents" />
      ),
      cell: ({ row }) => <span>{formatNumber(row.original.agents)}</span>,
      meta: {
        label: "Agents",
        variant: "number",
      },
    },
    {
      accessorKey: "storageBytes",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Storage" />
      ),
      cell: ({ row }) => <span>{formatBytes(row.original.storageBytes)}</span>,
      meta: {
        label: "Storage",
        variant: "number",
      },
    },
    {
      id: "compute",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Compute quota" />
      ),
      cell: ({ row }) => <span>{computeQuota(row.original)}</span>,
      meta: {
        label: "Compute quota",
        variant: "text",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const item = row.original

        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8 p-0"
                  aria-label="Tenant actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-40">
                <DropdownMenuLabel>Tenant</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onSelect(item)}>
                  View details
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.slug)
                  }}
                >
                  Copy tenant slug
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
      enableSorting: false,
      enableHiding: false,
      size: 40,
    },
  ]
}
