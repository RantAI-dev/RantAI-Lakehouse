"use client"

import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { HealthBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import { HEALTH_LABEL, type Health } from "@/lib/status"
import type { Connector } from "@/services/contracts/connectors"

type Direction = Connector["direction"]

export const DIRECTION_LABEL: Record<Direction, string> = {
  source: "Source",
  sink: "Sink",
  bidirectional: "Bidirectional",
}

const DIRECTION_OPTIONS = (Object.keys(DIRECTION_LABEL) as Direction[]).map((d) => ({
  value: d,
  label: DIRECTION_LABEL[d],
}))

const HEALTH_OPTIONS = (Object.keys(HEALTH_LABEL) as Health[]).map((h) => ({
  value: h,
  label: HEALTH_LABEL[h],
}))

export function getConnectorColumns({
  onSelect,
}: {
  onSelect: (connector: Connector) => void
}): ColumnDef<Connector>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Connector" />
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
            <p className="text-xs text-muted-foreground">{r.type}</p>
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Connector",
        variant: "text",
      },
    },
    {
      accessorKey: "direction",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Direction" />
      ),
      cell: ({ row }) => <span>{DIRECTION_LABEL[row.original.direction]}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Direction",
        variant: "select",
        options: DIRECTION_OPTIONS,
      },
    },
    {
      accessorKey: "health",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Health" />
      ),
      cell: ({ row }) => <HealthBadge health={row.original.health} />,
      enableColumnFilter: true,
      meta: {
        label: "Health",
        variant: "select",
        options: HEALTH_OPTIONS,
      },
    },
    {
      accessorKey: "environment",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Environment" />
      ),
      cell: ({ row }) => <span>{row.original.environment}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Environment",
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
      accessorKey: "lastTestAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last test" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastTestAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Last test",
        variant: "date",
      },
    },
    {
      accessorKey: "lastActivityAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last activity" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastActivityAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Last activity",
        variant: "date",
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
                  aria-label="Connector actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Connector</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onSelect(item)}>
                  View details
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem asChild>
                  <Link href={`/pipelines/create?connectorId=${item.id}`}>
                    Create pipeline
                  </Link>
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
