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

const ROTATION_STATUS_OPTIONS = [
  { value: "current", label: "Current" },
  { value: "due", label: "Rotation due" },
  { value: "expired", label: "Expired" },
]

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
        <DataTableColumnHeader column={column} label="Identity" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Identity",
        variant: "text",
      },
    },
    {
      id: "scopes",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Scopes" />
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
      accessorKey: "rotationStatus",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Rotation" />
      ),
      cell: ({ row }) => <RotationPill status={row.original.rotationStatus} />,
      enableColumnFilter: true,
      meta: {
        label: "Rotation",
        variant: "select",
        options: ROTATION_STATUS_OPTIONS,
      },
    },
    {
      accessorKey: "expiresAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Expires" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.expiresAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Expires",
        variant: "date",
      },
    },
    {
      accessorKey: "lastUsedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last used" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastUsedAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Last used",
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
