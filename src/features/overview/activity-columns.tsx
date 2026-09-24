"use client"

import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  Clock,
  Copy,
  FileText,
  MoreHorizontal,
  ShieldCheck,
  Tag,
  User,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { Button } from "@/components/ui/button"
import { formatRelativeTime } from "@/lib/format"
import { ACTOR_KIND_LABEL } from "@/lib/status"
import type { ActivityCategory, ActivityItem } from "@/services/contracts/overview"

export const CATEGORY_LABEL: Record<ActivityCategory, string> = {
  pipeline: "Pipeline",
  query: "Query",
  schema: "Schema",
  policy: "Policy",
  connector: "Connector",
  agent: "Agent",
  approval: "Approval",
  incident: "Incident",
}

export function getActivityColumns(): ColumnDef<ActivityItem>[] {
  return [
    {
      id: "at",
      accessorKey: "at",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="When" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground whitespace-nowrap">
          {formatRelativeTime(row.original.at)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "When",
        variant: "text",
        icon: Clock,
      },
    },
    {
      id: "actor",
      accessorKey: "actor",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Actor" />
      ),
      cell: ({ row }) => (
        <div className="flex flex-col">
          <span className="text-sm font-medium text-foreground">
            {row.original.actor}
          </span>
          <span className="text-xs text-muted-foreground">
            {ACTOR_KIND_LABEL[row.original.actorKind]}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Actor",
        placeholder: "Filter actor…",
        variant: "text",
        icon: User,
      },
    },
    {
      id: "actorKind",
      accessorKey: "actorKind",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Actor Kind" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {ACTOR_KIND_LABEL[row.original.actorKind]}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      filterFn: "arrIncludesSome",
      meta: {
        label: "Actor Kind",
        variant: "select",
        options: [
          { label: "User", value: "user" },
          { label: "Agent", value: "agent" },
          { label: "Service", value: "service" },
          { label: "System", value: "system" },
        ],
      },
    },
    {
      id: "action",
      accessorKey: "action",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Action" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-foreground">
          {row.original.action}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Action",
        placeholder: "Filter action…",
        variant: "text",
        icon: FileText,
      },
    },
    {
      id: "target",
      accessorKey: "target",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Target" />
      ),
      cell: ({ row }) =>
        row.original.targetHref ? (
          <Link
            href={row.original.targetHref}
            className="font-mono text-xs text-primary hover:underline"
          >
            {row.original.target}
          </Link>
        ) : (
          <span className="font-mono text-xs text-muted-foreground">
            {row.original.target}
          </span>
        ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Target",
        placeholder: "Filter target…",
        variant: "text",
      },
    },
    {
      id: "category",
      accessorKey: "category",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Category" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {CATEGORY_LABEL[row.original.category]}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      filterFn: "arrIncludesSome",
      meta: {
        label: "Category",
        variant: "select",
        options: Object.entries(CATEGORY_LABEL).map(([value, label]) => ({
          value,
          label,
        })),
        icon: Tag,
      },
    },
    {
      id: "actions",
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-8 p-0"
              aria-label="Activity actions"
            >
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {row.original.auditEventId ? (
              <DropdownMenuItem asChild>
                <Link href={`/audit?event=${row.original.auditEventId}`}>
                  <ShieldCheck className="size-4 mr-2" />
                  View in Audit
                </Link>
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.id)}
            >
              <Copy className="size-4 mr-2" />
              Copy Activity ID
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.action)}
            >
              <FileText className="size-4 mr-2" />
              Copy Action
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
      size: 40,
      enableResizing: false,
      enableHiding: false,
    },
  ]
}
