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
import { formatRelativeTime } from "@/lib/format"
import type { ServiceIdentity } from "@/services/contracts/identity"

function RotationPill({
  status,
}: {
  readonly status: ServiceIdentity["rotationStatus"]
}) {
  switch (status) {
    case "current":
      return <Pill tone="success">Current</Pill>
    case "due":
      return <Pill tone="warning">Rotation due</Pill>
    case "expired":
      return <Pill tone="destructive">Expired</Pill>
  }
}

export function getServiceIdentityColumns(): ColumnDef<ServiceIdentity>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Identity" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
      meta: {
        label: "Identity",
        variant: "text",
      },
    },
    {
      id: "scopes",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Scopes" />
      ),
      cell: ({ row }) => (
        <div className="flex flex-wrap gap-1">
          {row.original.scopes.map((scope) => (
            <Pill key={scope} tone="neutral" className="font-mono">
              {scope}
            </Pill>
          ))}
        </div>
      ),
      enableSorting: false,
      meta: {
        label: "Scopes",
        variant: "text",
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
      accessorKey: "rotationStatus",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Rotation" />
      ),
      cell: ({ row }) => <RotationPill status={row.original.rotationStatus} />,
      meta: {
        label: "Rotation",
        variant: "select",
      },
    },
    {
      accessorKey: "expiresAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Expires" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.expiresAt)}
        </span>
      ),
      meta: {
        label: "Expires",
        variant: "text",
      },
    },
    {
      accessorKey: "lastUsedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Last used" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastUsedAt)}
        </span>
      ),
      meta: {
        label: "Last used",
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
                  aria-label="Service identity actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Identity</DropdownMenuLabel>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.name)
                  }}
                >
                  Copy identity name
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
