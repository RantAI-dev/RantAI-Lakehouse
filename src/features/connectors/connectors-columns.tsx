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
import type { Connector } from "@/services/contracts/connectors"

type Direction = Connector["direction"]

export const DIRECTION_LABEL: Record<Direction, string> = {
  source: "Source",
  sink: "Sink",
  bidirectional: "Bidirectional",
}

export function getConnectorColumns({
  onSelect,
}: {
  onSelect: (connector: Connector) => void
}): ColumnDef<Connector>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Connector" />
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
      meta: {
        label: "Connector",
        variant: "text",
      },
    },
    {
      accessorKey: "direction",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Direction" />
      ),
      cell: ({ row }) => <span>{DIRECTION_LABEL[row.original.direction]}</span>,
      meta: {
        label: "Direction",
        variant: "select",
      },
    },
    {
      accessorKey: "health",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Health" />
      ),
      cell: ({ row }) => <HealthBadge health={row.original.health} />,
      meta: {
        label: "Health",
        variant: "select",
      },
    },
    {
      accessorKey: "environment",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Environment" />
      ),
      cell: ({ row }) => <span>{row.original.environment}</span>,
      meta: {
        label: "Environment",
        variant: "select",
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
      accessorKey: "lastTestAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Last test" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastTestAt)}
        </span>
      ),
      meta: {
        label: "Last test",
        variant: "text",
      },
    },
    {
      accessorKey: "lastActivityAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Last activity" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastActivityAt)}
        </span>
      ),
      meta: {
        label: "Last activity",
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
