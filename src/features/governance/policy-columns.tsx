"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Activity,
  Clock,
  Copy,
  Eye,
  FileCode,
  Layers,
  Lock,
  MoreHorizontal,
  Shield,
  Tag,
  User,
  Users,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill, StatusBadge } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import type { Policy } from "@/services/contracts/governance"

export const POLICY_STATUS_OPTIONS = (
  Object.keys(ENTITY_STATUS_LABEL) as EntityStatus[]
).map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))

export function getPolicyColumns({
  onSelect,
}: {
  onSelect: (policy: Policy) => void
}): ColumnDef<Policy>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Policy" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Shield className="size-4 shrink-0 text-muted-foreground" />
          <button
            type="button"
            className="text-left font-medium text-foreground hover:underline focus:outline-none"
            onClick={() => onSelect(row.original)}
          >
            {row.original.name}
          </button>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Policy",
        placeholder: "Filter policy name…",
        variant: "text",
        icon: Shield,
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
        options: POLICY_STATUS_OPTIONS,
        icon: Activity,
      },
    },
    {
      id: "kind",
      accessorKey: "kind",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Kind" />
      ),
      cell: ({ row }) => (
        <span className="inline-flex items-center rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
          {row.original.kind}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Kind",
        placeholder: "Filter kind…",
        variant: "text",
        icon: Tag,
      },
    },
    {
      id: "subjects",
      accessorKey: "subjects",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Subjects" />
      ),
      cell: ({ row }) => (
        <span className="line-clamp-1 max-w-44 text-sm text-muted-foreground">
          {row.original.subjects}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Subjects",
        placeholder: "Filter subjects…",
        variant: "text",
        icon: Users,
      },
    },
    {
      id: "resources",
      accessorKey: "resources",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Resources" />
      ),
      cell: ({ row }) => (
        <span className="line-clamp-1 max-w-44 font-mono text-xs text-muted-foreground">
          {row.original.resources}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Resources",
        placeholder: "Filter resources…",
        variant: "text",
        icon: Layers,
      },
    },
    {
      id: "effect",
      accessorKey: "effect",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Effect" />
      ),
      cell: ({ row }) => {
        const effect = row.original.effect?.toLowerCase()
        const isAllow = effect === "allow"
        return (
          <Pill tone={isAllow ? "success" : "danger"}>
            {row.original.effect}
          </Pill>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Effect",
        variant: "select",
        options: [
          { value: "allow", label: "Allow" },
          { value: "deny", label: "Deny" },
        ],
        icon: Lock,
      },
    },
    {
      id: "version",
      accessorKey: "version",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Version" align="right" />
      ),
      cell: ({ row }) => (
        <div className="text-right font-mono text-xs text-muted-foreground">
          v{row.original.version}
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Version",
        variant: "number",
        icon: FileCode,
      },
    },
    {
      id: "owner",
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-foreground">{row.original.owner}</span>
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
      id: "updatedAt",
      accessorKey: "updatedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Updated" />
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {formatRelativeTime(row.original.updatedAt)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Updated",
        variant: "text",
        icon: Clock,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const policy = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Policy actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    onSelect(policy)
                  }}
                >
                  <Eye className="size-4" />
                  <span>View details</span>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(policy.name)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy policy name</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(policy.id)
                  }}
                >
                  <Shield className="size-4" />
                  <span>Copy policy ID</span>
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
