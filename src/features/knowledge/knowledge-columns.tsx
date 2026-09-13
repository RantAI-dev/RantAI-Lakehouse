"use client"

import * as React from "react"
import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import {
  CopyIcon,
  EllipsisIcon,
  EyeIcon,
  LayersIcon,
  SearchIcon,
} from "lucide-react"
import { toast } from "sonner"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import {
  ClassificationBadge,
  Pill,
  StatusBadge,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatCompactNumber, formatRelativeTime } from "@/lib/format"
import type {
  IndexStatus,
  KnowledgeSource,
} from "@/services/contracts/knowledge"

const INDEX_STATUS_TONE: Record<IndexStatus, "success" | "info" | "warning"> = {
  ready: "success",
  indexing: "info",
  degraded: "warning",
}

const INDEX_STATUS_LABEL: Record<IndexStatus, string> = {
  ready: "Ready",
  indexing: "Indexing",
  degraded: "Degraded",
}

export function IndexStatusPill({ status }: { readonly status: IndexStatus }) {
  return (
    <Pill tone={INDEX_STATUS_TONE[status]}>{INDEX_STATUS_LABEL[status]}</Pill>
  )
}

export function getKnowledgeColumns(options: {
  onInspect: (source: KnowledgeSource) => void
}): ColumnDef<KnowledgeSource>[] {
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
        <DataTableColumnHeader column={column} label="Source" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <p className="font-medium">{r.name}</p>
            <p className="text-xs text-muted-foreground">
              {r.kind} · {r.version}
            </p>
          </div>
        )
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
    },
    {
      accessorKey: "indexStatus",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Index" />
      ),
      cell: ({ row }) => <IndexStatusPill status={row.original.indexStatus} />,
    },
    {
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
    },
    {
      accessorKey: "classification",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Class" />
      ),
      cell: ({ row }) => (
        <ClassificationBadge classification={row.original.classification} />
      ),
    },
    {
      accessorKey: "chunkCount",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Chunks" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">
          {formatCompactNumber(row.original.chunkCount)}
        </span>
      ),
    },
    {
      accessorKey: "embeddingModel",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Embedding" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.embeddingModel}</span>
      ),
    },
    {
      accessorKey: "freshnessLagSeconds",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Freshness" />
      ),
      cell: ({ row }) => (
        <FreshnessIndicator lagSeconds={row.original.freshnessLagSeconds} />
      ),
    },
    {
      accessorKey: "dependentAgents",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Agents" />
      ),
      cell: ({ row }) => (
        <span className="tabular-nums">{row.original.dependentAgents}</span>
      ),
    },
    {
      accessorKey: "lastRefresh",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last refresh" />
      ),
      cell: ({ row }) => formatRelativeTime(row.original.lastRefresh),
    },
    {
      id: "actions",
      size: 48,
      enableSorting: false,
      enableHiding: false,
      cell: ({ row }) => {
        const source = row.original
        return (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Actions for ${source.name}`}
                  />
                }
              >
                <EllipsisIcon className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onInspect(source)}>
                  <EyeIcon className="size-3.5" data-icon="inline-start" />
                  Inspect source
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(source.id)
                    toast.success("Source ID copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy Source ID
                </DropdownMenuItem>
                <DropdownMenuItem
                  render={
                    <Link href={`/semantic-search?source=${encodeURIComponent(source.name)}`} />
                  }
                >
                  <SearchIcon className="size-3.5" data-icon="inline-start" />
                  Try in Semantic Search
                </DropdownMenuItem>
                {source.vectorJobId ? (
                  <DropdownMenuItem
                    render={
                      <Link href={`/vector-jobs?job=${encodeURIComponent(source.vectorJobId)}`} />
                    }
                  >
                    <LayersIcon className="size-3.5" data-icon="inline-start" />
                    View Vector Job
                  </DropdownMenuItem>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
    },
  ]
}
