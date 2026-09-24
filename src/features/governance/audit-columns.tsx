"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Activity,
  Building2,
  Clock,
  Copy,
  DollarSign,
  Eye,
  FileText,
  MoreHorizontal,
  Scale,
  Shield,
  User as UserIcon,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { OutcomeBadge } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatCost, formatDateTime, formatRelativeTime } from "@/lib/format"
import { notifySuccess } from "@/lib/notify"
import {
  ACTOR_KIND_LABEL,
  AUDIT_OUTCOME_LABEL,
  type ActorKind,
  type AuditOutcome,
} from "@/lib/status"
import type { AuditEvent } from "@/services/contracts/governance"

export const ACTOR_KIND_OPTIONS = (Object.keys(ACTOR_KIND_LABEL) as ActorKind[]).map(
  (k) => ({ value: k, label: ACTOR_KIND_LABEL[k] })
)

export const OUTCOME_OPTIONS = (Object.keys(AUDIT_OUTCOME_LABEL) as AuditOutcome[]).map(
  (o) => ({ value: o, label: AUDIT_OUTCOME_LABEL[o] })
)

export function getAuditColumns({
  onSelect,
}: {
  readonly onSelect: (event: AuditEvent) => void
}): ColumnDef<AuditEvent>[] {
  return [
    {
      id: "select",
      header: ({ table }) => (
        <div className="flex items-center justify-center">
          <input
            type="checkbox"
            className="size-4 rounded border-border text-primary focus:ring-primary"
            checked={
              table.getIsAllPageRowsSelected() ||
              (table.getIsSomePageRowsSelected() && "indeterminate") ===
                "indeterminate"
            }
            onChange={(e) => table.toggleAllPageRowsSelected(e.target.checked)}
            aria-label="Select all audit events"
          />
        </div>
      ),
      cell: ({ row }) => (
        <div className="flex items-center justify-center">
          <input
            type="checkbox"
            className="size-4 rounded border-border text-primary focus:ring-primary"
            checked={row.getIsSelected()}
            onChange={(e) => row.toggleSelected(e.target.checked)}
            aria-label={`Select audit event ${row.original.id}`}
          />
        </div>
      ),
      enableSorting: false,
      enableHiding: false,
      meta: {
        fitContent: true,
      },
    },
    {
      id: "at",
      accessorKey: "at",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="When" />
      ),
      cell: ({ row }) => (
        <div className="flex flex-col gap-0.5 whitespace-nowrap">
          <span className="text-sm font-medium text-foreground">
            {formatRelativeTime(row.original.at)}
          </span>
          <span className="text-xs text-muted-foreground">
            {formatDateTime(row.original.at)}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "When",
        placeholder: "Filter timestamp…",
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
      cell: ({ row }) => {
        const item = row.original
        return (
          <div className="flex flex-col gap-0.5">
            <span className="text-sm font-medium text-foreground">
              {item.actor}
            </span>
            <span className="text-xs text-muted-foreground">
              {ACTOR_KIND_LABEL[item.actorKind]}
              {item.delegatedActor ? ` · on behalf of ${item.delegatedActor}` : ""}
            </span>
          </div>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Actor",
        placeholder: "Filter actor…",
        variant: "text",
        icon: UserIcon,
      },
    },
    {
      id: "tenant",
      accessorKey: "tenant",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tenant" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-foreground">
          {row.original.tenant}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Tenant",
        placeholder: "Filter tenant…",
        variant: "text",
        icon: Building2,
      },
    },
    {
      id: "action",
      accessorKey: "action",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Action" />
      ),
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-medium text-foreground hover:underline focus:outline-none"
          onClick={() => onSelect(row.original)}
        >
          {row.original.action}
        </button>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Action",
        placeholder: "Filter action…",
        variant: "text",
        icon: Activity,
      },
    },
    {
      id: "resource",
      accessorKey: "resource",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Resource" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-foreground max-w-56 truncate inline-block">
          {row.original.resource}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Resource",
        placeholder: "Filter resource…",
        variant: "text",
        icon: FileText,
      },
    },
    {
      id: "outcome",
      accessorKey: "outcome",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Outcome" />
      ),
      cell: ({ row }) => <OutcomeBadge outcome={row.original.outcome} />,
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Outcome",
        variant: "multiSelect",
        options: OUTCOME_OPTIONS,
        icon: Shield,
      },
    },
    {
      id: "policyDecision",
      accessorKey: "policyDecision",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Policy" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.policyDecision}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Policy",
        placeholder: "Filter policy…",
        variant: "text",
        icon: Scale,
      },
    },
    {
      id: "cost",
      accessorFn: (r) => r.actualCost ?? r.estimatedCost ?? 0,
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Cost" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs text-foreground">
          {row.original.actualCost != null
            ? formatCost(row.original.actualCost)
            : "—"}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Cost",
        variant: "number",
        icon: DollarSign,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const item = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Audit event actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    onSelect(item)
                  }}
                >
                  <Eye className="size-4" />
                  <span>View details</span>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(item.id)
                    notifySuccess("Audit event ID copied")
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy Event ID</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(item.action)
                    notifySuccess("Action copied")
                  }}
                >
                  <Activity className="size-4" />
                  <span>Copy action</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(item.resource)
                    notifySuccess("Resource copied")
                  }}
                >
                  <FileText className="size-4" />
                  <span>Copy resource</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(item.actor)
                    notifySuccess("Actor copied")
                  }}
                >
                  <UserIcon className="size-4" />
                  <span>Copy actor</span>
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
