"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatCompactNumber, formatPercent } from "@/lib/format"
import type { UsageSummary } from "@/services/contracts/ops"

type TenantRow = UsageSummary["tenants"][number]

function BudgetUtilization({ row }: { row: TenantRow }) {
  const fraction = row.budgetLimit > 0 ? row.budgetSpent / row.budgetLimit : 0
  if (fraction >= 0.9) {
    return (
      <div className="flex items-center gap-2">
        <Pill tone="destructive">Critical</Pill>
        <span className="text-xs font-medium text-destructive">
          {formatPercent(fraction)}
        </span>
      </div>
    )
  }
  if (fraction >= 0.75) {
    return (
      <div className="flex items-center gap-2">
        <Pill tone="warning">High</Pill>
        <span className="text-xs text-muted-foreground">
          {formatPercent(fraction)}
        </span>
      </div>
    )
  }
  return (
    <div className="flex items-center gap-2">
      <Pill tone="success">Healthy</Pill>
      <span className="text-xs text-muted-foreground">
        {formatPercent(fraction)}
      </span>
    </div>
  )
}

export function getUsageTenantColumns(): ColumnDef<TenantRow>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tenant" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Tenant",
        variant: "text",
      },
    },
    {
      accessorKey: "computeUnits",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Compute" />
      ),
      cell: ({ row }) => (
        <span>{formatCompactNumber(row.original.computeUnits)}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Compute",
        variant: "number",
      },
    },
    {
      accessorKey: "budgetSpent",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Budget" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <span>
            {formatCompactNumber(r.budgetSpent)} / {formatCompactNumber(r.budgetLimit)}
          </span>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Budget",
        variant: "number",
      },
    },
    {
      id: "utilization",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Utilization" />
      ),
      cell: ({ row }) => <BudgetUtilization row={row.original} />,
      meta: {
        label: "Utilization",
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
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.name)
                  }}
                >
                  Copy tenant name
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
