"use client"

import * as React from "react"
import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import {
  BookOpenIcon,
  CopyIcon,
  DatabaseIcon,
  EllipsisIcon,
  EyeIcon,
  SearchIcon,
} from "lucide-react"
import { toast } from "sonner"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import type { VectorJob } from "@/services/contracts/knowledge"
import { ENTITY_STATUS_LABEL } from "@/lib/status"

const ENTITY_STATUS_OPTIONS = Object.entries(ENTITY_STATUS_LABEL).map(([value, label]) => ({
  value,
  label,
}))

export function getVectorJobColumns(options: {
  readonly onInspect: (job: VectorJob) => void
}): ColumnDef<VectorJob>[] {
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
        <DataTableColumnHeader column={column} label="Job" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Job",
        variant: "text",
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
      enableColumnFilter: true,
      meta: {
        label: "Status",
        variant: "select",
        options: ENTITY_STATUS_OPTIONS,
      },
    },
    {
      accessorKey: "source",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Source" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {row.original.source}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Source",
        variant: "text",
      },
    },
    {
      accessorKey: "embeddingModel",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Model" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.embeddingModel}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Model",
        variant: "text",
      },
    },
    {
      accessorKey: "indexType",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Index" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.indexType}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Index",
        variant: "text",
      },
    },
    {
      accessorKey: "lastRunAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last run" />
      ),
      cell: ({ row }) => formatRelativeTime(row.original.lastRunAt),
      enableColumnFilter: true,
      meta: {
        label: "Last run",
        variant: "date",
      },
    },
    {
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      enableColumnFilter: true,
      meta: {
        label: "Owner",
        variant: "text",
      },
    },
    {
      id: "actions",
      size: 48,
      enableSorting: false,
      enableHiding: false,
      cell: ({ row }) => {
        const job = row.original
        return (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Actions for ${job.name}`}
                  />
                }
              >
                <EllipsisIcon className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onInspect(job)}>
                  <EyeIcon className="size-3.5" data-icon="inline-start" />
                  Inspect job
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(job.id)
                    toast.success("Job ID copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy Job ID
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(job.source)
                    toast.success("Source URI copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy source URI
                </DropdownMenuItem>
                {job.sourceId ? (
                  <DropdownMenuItem
                    render={
                      <Link href={`/knowledge?source=${encodeURIComponent(job.sourceId)}`} />
                    }
                  >
                    <BookOpenIcon className="size-3.5" data-icon="inline-start" />
                    Knowledge source
                  </DropdownMenuItem>
                ) : null}
                {job.outputAssetId ? (
                  <DropdownMenuItem
                    render={
                      <Link href={`/data/assets/${job.outputAssetId}`} />}
                  >
                    <DatabaseIcon className="size-3.5" data-icon="inline-start" />
                    Output asset
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuItem
                  render={
                    <Link href={`/semantic-search?source=${encodeURIComponent(job.source)}`} />
                  }
                >
                  <SearchIcon className="size-3.5" data-icon="inline-start" />
                  Try in Semantic Search
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
    },
  ]
}
