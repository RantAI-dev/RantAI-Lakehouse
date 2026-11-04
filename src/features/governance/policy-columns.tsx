"use client"

import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  Activity,
  Copy,
  Eye,
  Layers,
  ListChecks,
  MoreHorizontal,
  Shield,
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
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import type { Policy } from "@/services/contracts/governance"
import { accessTabHref, type AccessRule } from "./access-rules"
import { POLICY_NOT_ENFORCEABLE } from "./policy-actions"

export const POLICY_STATUS_OPTIONS = (
  Object.keys(ENTITY_STATUS_LABEL) as EntityStatus[]
).map((s) => ({ value: s, label: ENTITY_STATUS_LABEL[s] }))

/**
 * The Status cell (`SEC-61`): a `ready` policy the engine cannot enforce
 * reads "Not enforced", not its lifecycle status — the engine skips its
 * conditions, so calling it enforced would tell an admin a column is
 * masked when it is not. A draft keeps its draft status, which is already
 * accurate. The column still filters and sorts on `status`; this changes
 * only what the cell says.
 */
export function PolicyStatusPill({
  policy,
}: {
  policy: Pick<Policy, "status" | "enforceable">
}) {
  if (policy.status === "ready" && !policy.enforceable) {
    return (
      <Pill tone="warning" title={POLICY_NOT_ENFORCEABLE}>
        Not enforced
      </Pill>
    )
  }
  return <StatusBadge status={policy.status} />
}

/**
 * The table an access rule governs: a link to its Access tab when the API
 * named the asset (`assetId`, `WEB-3` S3a-D1), plain text otherwise (a
 * table the catalog does not address, or a rule written as text).
 */
export function AccessRuleTable({ rule }: { rule: Pick<AccessRule, "table" | "assetId"> }) {
  const href = accessTabHref(rule)
  if (!href) return <span className="font-mono text-xs text-muted-foreground">{rule.table}</span>
  return (
    <Link
      href={href}
      onClick={(e) => e.stopPropagation()}
      className="font-mono text-xs text-primary underline-offset-2 hover:underline"
    >
      {rule.table}
    </Link>
  )
}

/**
 * The Access rules list (`WEB-3` S3-D4): read-only. Enforce, Stop and
 * Delete stay on the table's Access tab, where the rule is made.
 */
export function getAccessRuleColumns({
  onSelect,
}: {
  onSelect: (rule: AccessRule) => void
}): ColumnDef<AccessRule>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Rule" />
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
        label: "Rule",
        placeholder: "Filter rule name…",
        variant: "text",
        icon: Shield,
      },
    },
    {
      id: "table",
      accessorKey: "table",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Table" />
      ),
      cell: ({ row }) => <AccessRuleTable rule={row.original} />,
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Table",
        placeholder: "Filter table…",
        variant: "text",
        icon: Layers,
      },
    },
    {
      id: "roles",
      accessorKey: "roles",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Roles" />
      ),
      cell: ({ row }) => (
        <span className="line-clamp-1 max-w-44 text-sm text-muted-foreground">
          {row.original.roles}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Roles",
        placeholder: "Filter roles…",
        variant: "text",
        icon: Users,
      },
    },
    {
      id: "does",
      accessorKey: "does",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="What it does" />
      ),
      cell: ({ row }) => (
        <span className="line-clamp-2 max-w-64 text-sm text-muted-foreground">
          {row.original.does}
        </span>
      ),
      enableColumnFilter: false,
      enableSorting: true,
      meta: {
        label: "What it does",
        icon: ListChecks,
      },
    },
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <PolicyStatusPill policy={row.original} />,
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
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const rule = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Access rule actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    onSelect(rule)
                  }}
                >
                  <Eye className="size-4" />
                  <span>View details</span>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(rule.name)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy rule name</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(rule.id)
                  }}
                >
                  <Shield className="size-4" />
                  <span>Copy rule ID</span>
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
