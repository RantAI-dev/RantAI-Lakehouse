"use client"

import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  Archive,
  ArrowRight,
  Clock,
  Copy,
  Database,
  DollarSign,
  FileCode2,
  HardDrive,
  MoreHorizontal,
  ShieldAlert,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { StatusBadge, TierBadge } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { Button } from "@/components/ui/button"
import { formatRelativeTime } from "@/lib/format"
import type { LifecyclePolicy, TieringOp } from "@/services/contracts/storage"

// ── Lifecycle Policies Columns ──────────────────────────────────────────

export function getLifecyclePolicyColumns(): ColumnDef<LifecyclePolicy>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Policy" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <HardDrive className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-mono text-sm font-medium text-foreground">
            {row.original.name}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Policy",
        placeholder: "Filter policy name…",
        variant: "text",
        icon: HardDrive,
      },
    },
    {
      id: "scope",
      accessorKey: "scope",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Scope" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {row.original.scope}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Scope",
        placeholder: "Filter scope…",
        variant: "text",
        icon: FileCode2,
      },
    },
    {
      id: "rules",
      accessorFn: (r) => `${r.hotDays}d → ${r.warmDays}d → ${r.coldAfterDays}d+`,
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Hot → Warm → Cold" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-1 font-mono text-xs">
          <span>{row.original.hotDays}d</span>
          <span className="text-muted-foreground">→</span>
          <span>{row.original.warmDays}d</span>
          <span className="text-muted-foreground">→</span>
          <span>{row.original.coldAfterDays}d+</span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Thresholds",
        placeholder: "Filter threshold…",
        variant: "text",
        icon: ArrowRight,
      },
    },
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
      enableColumnFilter: true,
      enableSorting: true,
      filterFn: "arrIncludesSome",
      meta: {
        label: "Status",
        variant: "select",
        options: [
          { label: "Ready", value: "ready" },
          { label: "Draft", value: "draft" },
          { label: "Paused", value: "paused" },
        ],
      },
    },
    {
      id: "estimatedSavings",
      accessorKey: "estimatedSavings",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Savings" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-foreground">
          {row.original.estimatedSavings}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Savings",
        variant: "text",
        icon: DollarSign,
      },
    },
    {
      id: "lastAppliedAt",
      accessorKey: "lastAppliedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last Applied" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {formatRelativeTime(row.original.lastAppliedAt)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Last Applied",
        variant: "text",
        icon: Clock,
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
              aria-label="Policy actions"
            >
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.id)}
            >
              <Copy className="size-4 mr-2" />
              Copy ID
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.name)}
            >
              <Archive className="size-4 mr-2" />
              Copy Name
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.scope)}
            >
              <FileCode2 className="size-4 mr-2" />
              Copy Scope
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

// ── Tiering Operations Columns ──────────────────────────────────────────

export function getTieringOpColumns(): ColumnDef<TieringOp>[] {
  return [
    {
      id: "asset",
      accessorKey: "asset",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Asset" />
      ),
      cell: ({ row }) =>
        row.original.assetId ? (
          <Link
            href={`/data/assets/${row.original.assetId}`}
            className="flex items-center gap-2 font-mono text-sm font-medium hover:underline text-foreground"
          >
            <Database className="size-4 shrink-0 text-muted-foreground" />
            <span>{row.original.asset}</span>
          </Link>
        ) : (
          <div className="flex items-center gap-2 font-mono text-sm font-medium text-foreground">
            <Database className="size-4 shrink-0 text-muted-foreground" />
            <span>{row.original.asset}</span>
          </div>
        ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Asset",
        placeholder: "Filter asset…",
        variant: "text",
        icon: Database,
      },
    },
    {
      id: "move",
      accessorFn: (r) => `${r.from} → ${r.to}`,
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Move" />
      ),
      cell: ({ row }) => (
        <span className="inline-flex items-center gap-1">
          <TierBadge tier={row.original.from} />
          <span className="text-muted-foreground">→</span>
          <TierBadge tier={row.original.to} />
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Move",
        placeholder: "Filter tier transition…",
        variant: "text",
        icon: ArrowRight,
      },
    },
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
      enableColumnFilter: true,
      enableSorting: true,
      filterFn: "arrIncludesSome",
      meta: {
        label: "Status",
        variant: "select",
        options: [
          { label: "Completed", value: "completed" },
          { label: "Running", value: "running" },
          { label: "Failed", value: "failed" },
          { label: "Cancelled", value: "cancelled" },
        ],
      },
    },
    {
      id: "at",
      accessorKey: "at",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="When" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
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
      id: "detail",
      accessorKey: "detail",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Detail" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground truncate max-w-sm block">
          {row.original.detail}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Detail",
        placeholder: "Filter detail…",
        variant: "text",
        icon: ShieldAlert,
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
              aria-label="Operation actions"
            >
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.id)}
            >
              <Copy className="size-4 mr-2" />
              Copy ID
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.asset)}
            >
              <Database className="size-4 mr-2" />
              Copy Asset Name
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => navigator.clipboard.writeText(row.original.detail)}
            >
              <ShieldAlert className="size-4 mr-2" />
              Copy Detail
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
