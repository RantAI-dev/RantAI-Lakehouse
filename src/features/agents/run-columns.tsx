"use client"

import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { AgentRunStatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatRelativeTime, formatTokens } from "@/lib/format"
import { fmtMeasured } from "@/lib/measured"
import { AGENT_RUN_STATUS_LABEL } from "@/lib/status"
import type { AgentRun } from "@/services/contracts/agents"

const STATUS_OPTIONS = Object.entries(AGENT_RUN_STATUS_LABEL).map(([value, label]) => ({
  value,
  label,
}))

export function getRunColumns(): ColumnDef<AgentRun>[] {
  return [
    {
      accessorKey: "id",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Run" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <Link
              href={`/agents/runs/${encodeURIComponent(r.id)}`}
              className="font-mono text-xs font-medium hover:underline"
            >
              {r.id}
            </Link>
            <p className="text-xs text-muted-foreground">{r.employeeId}</p>
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Run",
        variant: "text",
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <AgentRunStatusBadge status={row.original.status} />,
      enableColumnFilter: true,
      meta: {
        label: "Status",
        variant: "select",
        options: STATUS_OPTIONS,
      },
    },
    {
      accessorKey: "trigger",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Trigger" />
      ),
      cell: ({ row }) => <span>{row.original.trigger}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Trigger",
        variant: "text",
      },
    },
    {
      accessorKey: "actor",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Actor" />
      ),
      cell: ({ row }) => <span>{row.original.actor}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Actor",
        variant: "text",
      },
    },
    {
      accessorKey: "startedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Started" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.startedAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Started",
        variant: "date",
      },
    },
    {
      accessorKey: "endedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Ended" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {row.original.endedAt
            ? formatRelativeTime(row.original.endedAt)
            : "—"}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Ended",
        variant: "date",
      },
    },
    {
      accessorKey: "budgetConsumed",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tokens" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">
          {fmtMeasured(row.original.budgetConsumed, formatTokens)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Tokens",
        variant: "number",
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
                  aria-label="Run actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Run</DropdownMenuLabel>
                <DropdownMenuItem asChild>
                  <Link href={`/agents/runs/${encodeURIComponent(item.id)}`}>
                    View run details
                  </Link>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.id)
                  }}
                >
                  Copy Run ID
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
