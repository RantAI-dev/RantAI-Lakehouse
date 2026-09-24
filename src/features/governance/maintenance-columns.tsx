"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  AlertTriangle,
  Clock,
  Copy,
  Database,
  FileCheck,
  FileSearch,
  MoreHorizontal,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import type { MaintenanceRun } from "@/services/contracts/governance"

export function getMaintenanceColumns(): ColumnDef<MaintenanceRun>[] {
  return [
    {
      id: "tableName",
      accessorKey: "tableName",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Bronze Table" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Database className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-mono text-sm font-medium text-foreground">
            {row.original.tableName}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Bronze Table",
        placeholder: "Filter table…",
        variant: "text",
        icon: Database,
      },
    },
    {
      id: "runAt",
      accessorKey: "runAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Run" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {formatRelativeTime(row.original.runAt)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Run",
        variant: "text",
        icon: Clock,
      },
    },
    {
      id: "dryRun",
      accessorFn: (r) =>
        `${r.dryRun.deletedDataFiles} data / ${r.dryRun.deletedManifestFiles} manifest`,
      header: ({ column }) => (
        <DataTableColumnHeader
          column={column}
          label="Dry run (would delete)"
        />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {row.original.dryRun.deletedDataFiles} data /{" "}
          {row.original.dryRun.deletedManifestFiles} manifest
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Dry Run",
        placeholder: "Filter dry run…",
        variant: "text",
        icon: FileSearch,
      },
    },
    {
      id: "applied",
      accessorFn: (r) =>
        `${r.applied.deletedDataFiles} data / ${r.applied.deletedManifestFiles} manifest`,
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Applied" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-foreground">
          {row.original.applied.deletedDataFiles} data /{" "}
          {row.original.applied.deletedManifestFiles} manifest
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Applied",
        placeholder: "Filter applied…",
        variant: "text",
        icon: FileCheck,
      },
    },
    {
      id: "skippedVerbs",
      accessorKey: "skippedVerbs",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Skipped Verbs" />
      ),
      cell: ({ row }) =>
        row.original.skippedVerbs ? (
          <Pill tone="warning" title={row.original.skippedVerbs}>
            {row.original.skippedVerbs}
          </Pill>
        ) : (
          <span className="text-xs text-muted-foreground">none</span>
        ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Skipped Verbs",
        placeholder: "Filter verbs…",
        variant: "text",
        icon: AlertTriangle,
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
                aria-label="Maintenance actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(item.tableName)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy table name</span>
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
