"use client"

import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  CopyIcon,
  DatabaseIcon,
  EllipsisIcon,
  EyeIcon,
  FileTextIcon,
  GitForkIcon,
  RotateCcwIcon,
  SquareIcon,
} from "lucide-react"
import { toast } from "sonner"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  formatCompactNumber,
  formatDuration,
  formatRelativeTime,
} from "@/lib/format"
import type { PipelineRun } from "@/services/contracts/pipelines"

/**
 * How long a run took, reading "running" while it is still going.
 *
 * Prefers the duration the orchestrator reported; falls back to the gap
 * between the two timestamps when it did not give one.
 */
export function runDuration(run: PipelineRun): string {
  if (!run.endedAt) return "running"
  if (run.durationSeconds != null) return formatDuration(run.durationSeconds * 1000)
  return formatDuration(
    new Date(run.endedAt).getTime() - new Date(run.startedAt).getTime()
  )
}

interface GetPipelineRunColumnsProps {
  readonly onSelect?: (run: PipelineRun) => void
  readonly onCancel?: (run: PipelineRun) => void
  readonly onRetry?: (run: PipelineRun) => void
}

export function getPipelineRunColumns({
  onSelect,
  onCancel,
  onRetry,
}: GetPipelineRunColumnsProps = {}): ColumnDef<PipelineRun>[] {
  return [
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <StatusBadge status={row.original.status} />,
      enableSorting: true,
    },
    {
      id: "startedAt",
      accessorKey: "startedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Started" />
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground whitespace-nowrap">
          {formatRelativeTime(row.original.startedAt)}
        </span>
      ),
      enableSorting: true,
    },
    {
      id: "duration",
      accessorFn: (r) => runDuration(r),
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Duration" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {runDuration(row.original)}
        </span>
      ),
      enableSorting: true,
    },
    {
      id: "processed",
      accessorKey: "processed",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Processed" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatCompactNumber(row.original.processed)}
        </span>
      ),
      enableSorting: true,
    },
    {
      id: "accepted",
      accessorKey: "accepted",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Accepted" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">
          {formatCompactNumber(row.original.accepted)}
        </span>
      ),
      enableSorting: true,
    },
    {
      id: "rejected",
      accessorKey: "rejected",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Rejected" />
      ),
      cell: ({ row }) => (
        <span
          className={
            (row.original.rejected ?? 0) > 0
              ? "font-mono text-xs text-destructive font-medium"
              : "font-mono text-xs text-muted-foreground"
          }
        >
          {formatCompactNumber(row.original.rejected)}
        </span>
      ),
      enableSorting: true,
    },
    {
      id: "retried",
      accessorKey: "retried",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Retried" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {formatCompactNumber(row.original.retried)}
        </span>
      ),
      enableSorting: true,
    },
    {
      id: "error",
      accessorKey: "error",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Error" />
      ),
      cell: ({ row }) =>
        row.original.error ? (
          <span
            className="block max-w-52 truncate text-xs text-destructive"
            title={row.original.error}
          >
            {row.original.error}
          </span>
        ) : (
          <span className="text-muted-foreground">—</span>
        ),
      enableSorting: true,
    },
    {
      id: "actions",
      header: () => null,
      enableSorting: false,
      enableHiding: false,
      cell: ({ row }) => {
        const run = row.original
        return (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Actions for run ${run.id}`}
                  />
                }
              >
                <EllipsisIcon className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {onSelect ? (
                  <DropdownMenuItem onClick={() => onSelect(run)}>
                    <EyeIcon className="size-3.5" data-icon="inline-start" />
                    View details
                  </DropdownMenuItem>
                ) : null}
                {run.status === "running" && onCancel ? (
                  <DropdownMenuItem onClick={() => onCancel(run)}>
                    <SquareIcon className="size-3.5" data-icon="inline-start" />
                    Cancel run
                  </DropdownMenuItem>
                ) : null}
                {(run.status === "failed" || run.status === "cancelled") && onRetry ? (
                  <DropdownMenuItem onClick={() => onRetry(run)}>
                    <RotateCcwIcon className="size-3.5" data-icon="inline-start" />
                    Retry run
                  </DropdownMenuItem>
                ) : null}
                {run.outputAssetId ? (
                  <DropdownMenuItem render={<Link href={`/data/assets/${run.outputAssetId}`} />}>
                    <DatabaseIcon className="size-3.5" data-icon="inline-start" />
                    Output dataset
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuItem render={<Link href={`/lineage?focus=${run.pipelineId}`} />}>
                  <GitForkIcon className="size-3.5" data-icon="inline-start" />
                  Lineage
                </DropdownMenuItem>
                {run.auditEventId ? (
                  <DropdownMenuItem render={<Link href={`/audit?event=${run.auditEventId}`} />}>
                    <FileTextIcon className="size-3.5" data-icon="inline-start" />
                    Audit
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(run.id)
                    toast.success("Run ID copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy run ID
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
    },
  ]
}
