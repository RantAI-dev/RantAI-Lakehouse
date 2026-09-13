"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { ApprovalBadge } from "@/components/patterns/status-badge"
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
import type { ApprovalItem } from "@/services/contracts/agents"

interface ApprovalColumnsProps {
  readonly onSelect: (approval: ApprovalItem) => void
  readonly onDecide?: (approval: ApprovalItem, decision: "approved" | "rejected") => void
}

export function getApprovalColumns({
  onSelect,
  onDecide,
}: ApprovalColumnsProps): ColumnDef<ApprovalItem>[] {
  return [
    {
      accessorKey: "action",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Requested action" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <button
              type="button"
              onClick={() => onSelect(r)}
              className="text-start font-medium hover:underline"
            >
              {r.action}
            </button>
            <p className="text-xs text-muted-foreground">{r.employeeName}</p>
          </div>
        )
      },
      meta: {
        label: "Requested action",
        variant: "text",
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <ApprovalBadge status={row.original.status} />,
      meta: {
        label: "Status",
        variant: "select",
      },
    },
    {
      accessorKey: "risk",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Risk" />
      ),
      cell: ({ row }) => <span>{row.original.risk}</span>,
      meta: {
        label: "Risk",
        variant: "text",
      },
    },
    {
      accessorKey: "requestedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Requested" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.requestedAt)}
        </span>
      ),
      meta: {
        label: "Requested",
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
                  aria-label="Approval actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Approval</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onSelect(item)}>
                  View details
                </DropdownMenuItem>
                {item.status === "pending" && onDecide && (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      onClick={() => onDecide(item, "approved")}
                    >
                      Approve
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => onDecide(item, "rejected")}
                    >
                      Reject
                    </DropdownMenuItem>
                  </>
                )}
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.action)
                  }}
                >
                  Copy action
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
