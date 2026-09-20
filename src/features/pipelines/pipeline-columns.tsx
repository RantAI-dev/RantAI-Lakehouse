"use client"

import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  Activity,
  ArrowRight,
  Calendar,
  CheckCircle2,
  Clock,
  Database,
  Eye,
  GitBranch,
  Layers,
  MoreHorizontal,
  Play,
  Timer,
  User,
  XCircle,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Badge } from "@/components/ui/badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import type { Pipeline, PipelineKind } from "@/services/contracts/pipelines"

export const PIPELINE_KIND_OPTIONS: { value: PipelineKind; label: string }[] = [
  { value: "batch", label: "Batch" },
  { value: "incremental", label: "Incremental" },
  { value: "document", label: "Document" },
  { value: "vector", label: "Vector" },
]

export const PIPELINE_STATUS_OPTIONS: { value: EntityStatus; label: string }[] =
  Object.entries(ENTITY_STATUS_LABEL).map(([value, label]) => ({
    value: value as EntityStatus,
    label,
  }))

export function getPipelineColumns({
  onView,
  onTrigger,
}: {
  onView?: (pipeline: Pipeline) => void
  onTrigger?: (pipeline: Pipeline) => void
} = {}): ColumnDef<Pipeline>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Pipeline" />
      ),
      cell: ({ row }) => {
        const pipeline = row.original
        return (
          <div className="flex flex-col gap-0.5">
            <Link
              href={`/pipelines/${pipeline.id}`}
              className="font-medium text-foreground hover:underline"
              onClick={(e) => {
                if (onView) {
                  e.preventDefault()
                  onView(pipeline)
                }
              }}
            >
              {pipeline.name}
            </Link>
            <Copyable
              value={pipeline.id}
              className="text-xs text-muted-foreground font-mono truncate max-w-48"
            >
              {pipeline.id}
            </Copyable>
          </div>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Pipeline",
        placeholder: "Filter pipeline name…",
        variant: "text",
        icon: GitBranch,
      },
    },
    {
      id: "kind",
      accessorKey: "kind",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Kind" />
      ),
      cell: ({ row }) => (
        <Badge variant="outline" className="capitalize text-xs font-normal">
          {row.original.kind}
        </Badge>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Kind",
        variant: "select",
        options: PIPELINE_KIND_OPTIONS,
        icon: Layers,
      },
    },
    {
      id: "origin",
      accessorKey: "origin",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Origin" />
      ),
      // What a row can actually do follows from where it came from: only
      // an orchestrator job can be triggered, cancelled or retried.
      cell: ({ row }) => (
        <Tooltip>
          <TooltipTrigger
            render={
              <Badge variant="outline" className="text-xs font-normal">
                {row.original.origin === "orchestrator" ? "Orchestrator" : "Authored"}
              </Badge>
            }
          />
          <TooltipContent>
            {row.original.origin === "orchestrator"
              ? "A job in the orchestrator: it runs, and its history is here."
              : "Authored in the console. No engine is attached yet, so it cannot run."}
          </TooltipContent>
        </Tooltip>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Origin",
        variant: "multiSelect",
        options: [
          { value: "orchestrator", label: "Orchestrator" },
          { value: "authored", label: "Authored" },
        ],
        icon: Layers,
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
      meta: {
        label: "Status",
        variant: "select",
        options: PIPELINE_STATUS_OPTIONS,
        icon: Activity,
      },
    },
    {
      id: "owner",
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      cell: ({ row }) => (
        <span className="text-sm font-medium text-foreground truncate max-w-36 block">
          {row.original.owner}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Owner",
        placeholder: "Filter owner…",
        variant: "text",
        icon: User,
      },
    },
    {
      id: "source",
      accessorKey: "source",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Source" />
      ),
      cell: ({ row }) => (
        <span
          className="font-mono text-xs text-muted-foreground truncate max-w-36 block"
          title={row.original.source}
        >
          {row.original.source}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Source",
        placeholder: "Filter source…",
        variant: "text",
        icon: Database,
      },
    },
    {
      id: "target",
      accessorKey: "target",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Target" />
      ),
      cell: ({ row }) => (
        <span
          className="font-mono text-xs text-muted-foreground truncate max-w-36 block"
          title={row.original.target}
        >
          {row.original.target}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Target",
        placeholder: "Filter target…",
        variant: "text",
        icon: ArrowRight,
      },
    },
    {
      id: "schedule",
      accessorKey: "schedule",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Schedule" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">
          {row.original.schedule}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Schedule",
        placeholder: "Filter schedule…",
        variant: "text",
        icon: Clock,
      },
    },
    {
      id: "lastRunAt",
      accessorKey: "lastRunAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last Run" />
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground whitespace-nowrap">
          {row.original.lastRunAt ? formatRelativeTime(row.original.lastRunAt) : "Never"}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Last Run",
        variant: "date",
        icon: Calendar,
      },
    },
    {
      id: "freshnessLagSeconds",
      accessorKey: "freshnessLagSeconds",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Freshness" />
      ),
      cell: ({ row }) => (
        <FreshnessIndicator lagSeconds={row.original.freshnessLagSeconds} />
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Freshness",
        variant: "number",
        icon: Timer,
      },
    },
    {
      id: "slaOk",
      accessorKey: "slaOk",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="SLA" />
      ),
      cell: ({ row }) =>
        row.original.slaOk ? (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-emerald-600 dark:text-emerald-400">
            <CheckCircle2 className="size-3.5" />
            OK
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 text-xs font-medium text-destructive">
            <XCircle className="size-3.5" />
            Breached
          </span>
        ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "SLA",
        variant: "select",
        options: [
          { value: "true", label: "OK" },
          { value: "false", label: "Breached" },
        ],
        icon: CheckCircle2,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const pipeline = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Pipeline actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuItem asChild>
                  <Link href={`/pipelines/${pipeline.id}`}>
                    <Eye className="size-4" />
                    <span>View details</span>
                  </Link>
                </DropdownMenuItem>
                {onTrigger && pipeline.origin === "orchestrator" ? (
                  <DropdownMenuItem
                    onClick={() => {
                      onTrigger(pipeline)
                    }}
                  >
                    <Play className="size-4" />
                    <span>Trigger run</span>
                  </DropdownMenuItem>
                ) : null}
                {onTrigger && pipeline.origin === "authored" ? (
                  // Shown but disabled, with the reason: hiding it makes
                  // the menu look inconsistent between rows.
                  <DropdownMenuItem disabled>
                    <Play className="size-4" />
                    <span>Trigger run — no engine attached</span>
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(pipeline.id)
                  }}
                >
                  <GitBranch className="size-4" />
                  <span>Copy pipeline ID</span>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
      enableColumnFilter: false,
      enableSorting: false,
      enableHiding: false,
      size: 48,
      meta: {
        fitContent: true,
      },
    },
  ]
}
