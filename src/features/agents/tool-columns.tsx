"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { CopyIcon, EllipsisIcon, EyeIcon } from "lucide-react"
import { toast } from "sonner"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import {
  ApprovalBadge,
  HealthBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatCompactNumber } from "@/lib/format"
import type { AgentTool } from "@/services/contracts/agents"

type ToolColumnOptions = {
  readonly onInspect: (tool: AgentTool) => void
}

export function getToolColumns({
  onInspect,
}: ToolColumnOptions): ColumnDef<AgentTool>[] {
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
        <DataTableColumnHeader column={column} label="Tool" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <div className="flex items-center gap-2">
              <p className="font-mono font-medium">{r.name}</p>
              {r.deprecated ? <Pill tone="neutral">Deprecated</Pill> : null}
            </div>
            <p className="text-xs text-muted-foreground">
              v{r.version} · {r.publisher}
            </p>
          </div>
        )
      },
    },
    {
      accessorKey: "permission",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Permission" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {row.original.permission}
        </span>
      ),
    },
    {
      accessorKey: "health",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Health" />
      ),
      cell: ({ row }) => <HealthBadge health={row.original.health} />,
      filterFn: (row, id, value) => {
        return Array.isArray(value) && value.includes(row.getValue(id))
      },
    },
    {
      accessorKey: "approvalStatus",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Approval" />
      ),
      cell: ({ row }) => <ApprovalBadge status={row.original.approvalStatus} />,
      filterFn: (row, id, value) => {
        return Array.isArray(value) && value.includes(row.getValue(id))
      },
    },
    {
      accessorKey: "rateLimit",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Rate Limit" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.rateLimit}
        </span>
      ),
    },
    {
      accessorKey: "usage30d",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Usage 30d" />
      ),
      cell: ({ row }) => (
        <span className="text-sm tabular-nums">
          {formatCompactNumber(row.original.usage30d)}
        </span>
      ),
    },
    {
      id: "actions",
      size: 48,
      enableSorting: false,
      enableHiding: false,
      cell: ({ row }) => {
        const tool = row.original
        return (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Actions for ${tool.name}`}
                  />
                }
              >
                <EllipsisIcon className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onInspect(tool)}>
                  <EyeIcon className="size-3.5" data-icon="inline-start" />
                  Inspect tool
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(tool.permission)
                    toast.success("Permission copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy permission
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(tool.id)
                    toast.success("Tool ID copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy Tool ID
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
    },
  ]
}
