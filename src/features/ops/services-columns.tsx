"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatPercent } from "@/lib/format"
import type { PlatformService } from "@/services/contracts/ops"

function DependencyPills({ dependencies }: { readonly dependencies: readonly string[] }) {
  if (dependencies.length === 0) return <span>—</span>
  return (
    <div className="flex flex-wrap gap-1">
      {dependencies.map((dep) => (
        <Pill key={dep} tone="neutral">
          {dep}
        </Pill>
      ))}
    </div>
  )
}

export function getServiceColumns({
  onSelect,
}: {
  onSelect: (service: PlatformService) => void
}): ColumnDef<PlatformService>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Service" />
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
            <p className="text-xs text-muted-foreground">
              v{r.version} · {r.site}
            </p>
          </div>
        )
      },
      meta: {
        label: "Service",
        variant: "text",
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
      accessorKey: "replicas",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Replicas" />
      ),
      cell: ({ row }) => <span>{row.original.replicas}</span>,
      meta: {
        label: "Replicas",
        variant: "number",
      },
    },
    {
      accessorKey: "errorRate",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Error rate" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatPercent(row.original.errorRate)}
        </span>
      ),
      meta: {
        label: "Error rate",
        variant: "number",
      },
    },
    {
      accessorKey: "latencyMs",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Latency" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.latencyMs} ms</span>
      ),
      meta: {
        label: "Latency",
        variant: "number",
      },
    },
    {
      id: "dependencies",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} title="Dependencies" />
      ),
      cell: ({ row }) => (
        <DependencyPills dependencies={row.original.dependencies} />
      ),
      enableSorting: false,
      meta: {
        label: "Dependencies",
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
                  aria-label="Service actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Service Actions</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onSelect(item)}>
                  View details
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
