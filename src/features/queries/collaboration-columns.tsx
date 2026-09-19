"use client"

import * as React from "react"
import type { ColumnDef } from "@tanstack/react-table"
import { CopyIcon, EllipsisIcon, EyeIcon } from "lucide-react"
import { toast } from "sonner"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatNumber, formatRelativeTime } from "@/lib/format"
import type { CollaborationProject } from "@/services/contracts/queries"

export function getCollaborationColumns(options: {
  readonly onInspect: (project: CollaborationProject) => void
}): ColumnDef<CollaborationProject>[] {
  const { onInspect } = options

  return [
    {
      id: "select",
      header: ({ table }) => (
        <Checkbox
          checked={
            table.getIsAllPageRowsSelected() ||
            (table.getIsSomePageRowsSelected() && "indeterminate")
          }
          onCheckedChange={(val) => table.toggleAllPageRowsSelected(Boolean(val))}
          aria-label="Select all"
          className="translate-y-0.5"
        />
      ),
      cell: ({ row }) => (
        <Checkbox
          checked={row.getIsSelected()}
          onCheckedChange={(val) => row.toggleSelected(Boolean(val))}
          aria-label="Select row"
          className="translate-y-0.5"
        />
      ),
      size: 40,
      enableSorting: false,
      enableHiding: false,
    },
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Project" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Project",
        variant: "text",
      },
    },
    {
      accessorKey: "description",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Description" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.description}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Description",
        variant: "text",
      },
    },
    {
      accessorKey: "members",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Members" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">
          {formatNumber(row.original.members)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Members",
        variant: "number",
      },
    },
    {
      accessorKey: "updatedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Updated" />
      ),
      cell: ({ row }) => formatRelativeTime(row.original.updatedAt),
      enableColumnFilter: true,
      meta: {
        label: "Updated",
        variant: "date",
      },
    },
    {
      id: "actions",
      size: 48,
      enableSorting: false,
      enableHiding: false,
      cell: ({ row }) => {
        const project = row.original
        return (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Actions for ${project.name}`}
                  />
                }
              >
                <EllipsisIcon className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onInspect(project)}>
                  <EyeIcon className="size-3.5" data-icon="inline-start" />
                  Inspect project
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(project.name)
                    toast.success("Project name copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy project name
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(project.id)
                    toast.success("Project ID copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy Project ID
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
    },
  ]
}
