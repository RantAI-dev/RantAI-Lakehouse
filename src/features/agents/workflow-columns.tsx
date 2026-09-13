"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill, StatusBadge } from "@/components/patterns/status-badge"
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
import type { AgentWorkflow } from "@/services/contracts/agents"

interface WorkflowColumnsProps {
  readonly onSelect: (workflow: AgentWorkflow) => void
}

function ApprovalGatePill({ required }: { readonly required: boolean }) {
  return required ? (
    <Pill tone="warning">Approval gate</Pill>
  ) : (
    <Pill tone="neutral">Autonomous</Pill>
  )
}

export function getWorkflowColumns({
  onSelect,
}: WorkflowColumnsProps): ColumnDef<AgentWorkflow>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Workflow" />
      ),
      cell: ({ row }) => {
        const item = row.original
        return (
          <button
            type="button"
            onClick={() => onSelect(item)}
            className="text-start font-medium hover:underline"
          >
            {item.name}
          </button>
        )
      },
      meta: {
        label: "Workflow",
        variant: "text",
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
      meta: {
        label: "Status",
        variant: "select",
      },
    },
    {
      accessorKey: "trigger",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Trigger" />
      ),
      cell: ({ row }) => <span>{row.original.trigger}</span>,
      meta: {
        label: "Trigger",
        variant: "text",
      },
    },
    {
      accessorKey: "steps",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Steps" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">{row.original.steps}</span>
      ),
      meta: {
        label: "Steps",
        variant: "number",
      },
    },
    {
      id: "approval",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Approval" />
      ),
      cell: ({ row }) => (
        <ApprovalGatePill required={row.original.approvalRequired} />
      ),
      meta: {
        label: "Approval",
        variant: "select",
      },
    },
    {
      accessorKey: "lastRunAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last run" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.lastRunAt)}
        </span>
      ),
      meta: {
        label: "Last run",
        variant: "text",
      },
    },
    {
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      cell: ({ row }) => <span>{row.original.owner}</span>,
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
                  aria-label="Workflow actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Workflow</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onSelect(item)}>
                  View details
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.name)
                  }}
                >
                  Copy workflow name
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
