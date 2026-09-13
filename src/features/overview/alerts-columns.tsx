"use client"

import * as React from "react"
import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  AlertTriangle,
  ArrowUpDown,
  CheckCircle2,
  ExternalLink,
  MoreHorizontal,
  ShieldAlert,
  User,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  AlertStatusBadge,
  SeverityBadge,
} from "@/components/patterns/status-badge"
import { formatRelativeTime } from "@/lib/format"
import {
  ALERT_STATUS_LABEL,
  SEVERITY_LABEL,
  type AlertStatus,
  type Severity,
} from "@/lib/status"
import type { AlertItem } from "@/services/contracts/overview"

type AlertColumnOptions = {
  readonly onOpenDetail: (alert: AlertItem) => void
  readonly onAcknowledge?: (alert: AlertItem) => void
  readonly onResolve?: (alert: AlertItem) => void
}

export function getAlertColumns(options: AlertColumnOptions): ColumnDef<AlertItem>[] {
  const { onOpenDetail, onAcknowledge, onResolve } = options

  return [
    {
      id: "severity",
      accessorKey: "severity",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>Severity</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) => <SeverityBadge severity={row.original.severity} />,
      meta: {
        label: "Severity",
        variant: "select",
        options: (Object.entries(SEVERITY_LABEL) as [Severity, string][]).map(
          ([value, label]) => ({
            label,
            value,
            icon: AlertTriangle,
          })
        ),
      },
    },
    {
      id: "title",
      accessorKey: "title",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>Alert</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) => {
        const item = row.original
        return (
          <button
            type="button"
            className="text-left font-medium hover:underline focus:outline-none"
            onClick={() => onOpenDetail(item)}
          >
            {item.title}
          </button>
        )
      },
      meta: {
        label: "Alert Title",
        variant: "text",
      },
    },
    {
      id: "source",
      accessorKey: "source",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>Source</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">{row.original.source}</span>
      ),
      meta: {
        label: "Source",
        variant: "text",
      },
    },
    {
      id: "affected",
      accessorKey: "affected",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>Affected</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.affected}</span>
      ),
      meta: {
        label: "Affected Target",
        variant: "text",
      },
    },
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>Status</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) => <AlertStatusBadge status={row.original.status} />,
      meta: {
        label: "Status",
        variant: "select",
        options: (Object.entries(ALERT_STATUS_LABEL) as [AlertStatus, string][]).map(
          ([value, label]) => ({
            label,
            value,
            icon: ShieldAlert,
          })
        ),
      },
    },
    {
      id: "assignee",
      accessorKey: "assignee",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>Assignee</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) =>
        row.original.assignee ? (
          <span className="inline-flex items-center gap-1.5 text-xs">
            <User className="size-3 text-muted-foreground" />
            {row.original.assignee}
          </span>
        ) : (
          <span className="text-xs text-muted-foreground">Unassigned</span>
        ),
      meta: {
        label: "Assignee",
        variant: "text",
      },
    },
    {
      id: "at",
      accessorKey: "at",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3 h-8 data-[state=open]:bg-accent"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          <span>When</span>
          <ArrowUpDown className="ml-2 size-3.5" />
        </Button>
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {formatRelativeTime(row.original.at)}
        </span>
      ),
      meta: {
        label: "When",
        variant: "text",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const item = row.original

        return (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-8 p-0"
                aria-label="Open alert actions menu"
              >
                <MoreHorizontal className="size-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuLabel>Alert Actions</DropdownMenuLabel>
              <DropdownMenuItem onClick={() => onOpenDetail(item)}>
                View details &amp; timeline
              </DropdownMenuItem>
              {item.status === "open" && onAcknowledge ? (
                <DropdownMenuItem onClick={() => onAcknowledge(item)}>
                  <CheckCircle2 className="mr-2 size-4 text-amber-500" />
                  Acknowledge incident
                </DropdownMenuItem>
              ) : null}
              {item.status !== "resolved" && onResolve ? (
                <DropdownMenuItem onClick={() => onOpenDetail(item)}>
                  <CheckCircle2 className="mr-2 size-4 text-emerald-500" />
                  Resolve with note…
                </DropdownMenuItem>
              ) : null}
              {item.href ? (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem asChild>
                    <Link href={item.href} className="flex items-center">
                      <ExternalLink className="mr-2 size-4" />
                      Investigate target resource
                    </Link>
                  </DropdownMenuItem>
                </>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        )
      },
      enableSorting: false,
      enableHiding: false,
      size: 48,
    },
  ]
}
