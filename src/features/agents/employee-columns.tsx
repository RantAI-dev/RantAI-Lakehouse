"use client"

import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { AutonomyBadge, StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatCost, formatPercent } from "@/lib/format"
import type { DigitalEmployee } from "@/services/contracts/agents"
import { AUTONOMY_LABEL, ENTITY_STATUS_LABEL } from "@/lib/status"

const AUTONOMY_OPTIONS = Object.entries(AUTONOMY_LABEL).map(([value, label]) => ({
  value,
  label,
}))

const ENTITY_STATUS_OPTIONS = Object.entries(ENTITY_STATUS_LABEL).map(([value, label]) => ({
  value,
  label,
}))

export function getEmployeeColumns(): ColumnDef<DigitalEmployee>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Employee" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <Link
              href={`/agents/employees/${r.id}`}
              className="font-medium hover:underline"
            >
              {r.name}
            </Link>
            <p className="text-xs text-muted-foreground">{r.purpose}</p>
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Employee",
        variant: "text",
      },
    },
    {
      accessorKey: "autonomy",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Autonomy" />
      ),
      cell: ({ row }) => <AutonomyBadge level={row.original.autonomy} />,
      enableColumnFilter: true,
      meta: {
        label: "Autonomy",
        variant: "select",
        options: AUTONOMY_OPTIONS,
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
      enableColumnFilter: true,
      meta: {
        label: "Status",
        variant: "select",
        options: ENTITY_STATUS_OPTIONS,
      },
    },
    {
      id: "budget",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Budget" />
      ),
      cell: ({ row }) => {
        const r = row.original
        const used = r.budgetSpent + r.budgetReserved
        return (
          <span className="tabular-nums">
            {formatCost(used)} / {formatCost(r.budgetLimit)}{" "}
            <span className="text-xs text-muted-foreground">
              ({formatPercent(r.budgetLimit > 0 ? used / r.budgetLimit : 0)})
            </span>
          </span>
        )
      },
      meta: {
        label: "Budget",
        variant: "number",
      },
    },
    {
      accessorKey: "successRate",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Success" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">
          {formatPercent(row.original.successRate)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Success",
        variant: "number",
      },
    },
    {
      accessorKey: "approvalRate",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Approval rate" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">
          {formatPercent(row.original.approvalRate)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Approval rate",
        variant: "number",
      },
    },
    {
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      cell: ({ row }) => <span>{row.original.owner}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Owner",
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
                  aria-label="Employee actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Employee</DropdownMenuLabel>
                <DropdownMenuItem asChild>
                  <Link href={`/agents/employees/${item.id}`}>
                    View profile
                  </Link>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.name)
                  }}
                >
                  Copy employee name
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
