"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { ApprovalBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { accessGrantState } from "@/lib/access-requests"
import { formatRelativeTime } from "@/lib/format"
import type { ApprovalItem } from "@/services/contracts/agents"
import { APPROVAL_STATUS_LABEL } from "@/lib/status"

const APPROVAL_STATUS_OPTIONS = Object.entries(APPROVAL_STATUS_LABEL).map(([value, label]) => ({
  value,
  label,
}))

/** `"access:catalog:write"` → `"catalog:write"` — the permission a `kind = "access"` row's `action` names. */
export function accessPermission(action: string): string {
  return action.startsWith("access:") ? action.slice("access:".length) : action
}

/**
 * The row's own known grant expiry — only ever available for a request
 * THIS session just decided (`decidedGrants`, keyed by approval id). A row
 * read back from `GET /api/agents/approvals` never carries it (that route
 * lists `approval_item` rows only; the grant's `expires_at` lives in a
 * separate `access_grant` row nothing joins in — see
 * `@/lib/access-requests`'s `accessGrantState` doc comment). Showing
 * nothing here for an older approved request is the honest choice, never a
 * fabricated date.
 */
export function statusCell(r: ApprovalItem, decidedGrants: Record<string, string>) {
  if (r.kind !== "access") return <ApprovalBadge status={r.status} />
  const expiresAt = decidedGrants[r.id]
  const grantState = accessGrantState(r.status, expiresAt, Date.now())
  if (grantState === "approved-expired") {
    // Deliberately NOT the "Approved" tone — an expired grant must never
    // read as still active.
    return <Pill tone="neutral">Expired</Pill>
  }
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      <ApprovalBadge status={r.status} />
      {grantState === "approved-active" && expiresAt ? (
        <span className="text-xs text-muted-foreground">
          until {formatRelativeTime(expiresAt)}
        </span>
      ) : null}
    </span>
  )
}

interface ApprovalColumnsProps {
  readonly onSelect: (approval: ApprovalItem) => void
  readonly onDecide?: (approval: ApprovalItem, decision: "approved" | "rejected") => void
  /** Grants this session decided, keyed by approval id — see `statusCell`. */
  readonly decidedGrants?: Record<string, string>
  /** `true` when the signed-in user requested this row's access themselves — server-refused, disabled here only as belt-and-suspenders UX. */
  readonly isSelfApproval?: (approval: ApprovalItem) => boolean
}

export function getApprovalColumns({
  onSelect,
  onDecide,
  decidedGrants = {},
  isSelfApproval,
}: ApprovalColumnsProps): ColumnDef<ApprovalItem>[] {
  return [
    {
      accessorKey: "action",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Requested action" />
      ),
      cell: ({ row }) => {
        const r = row.original
        return (
          <div>
            <button
              type="button"
              onClick={() => onSelect(r)}
              className="text-start font-medium hover:underline"
            >
              {r.kind === "access" ? `Access: ${accessPermission(r.action)}` : r.action}
            </button>
            <p className="text-xs text-muted-foreground">
              {r.kind === "access" ? (r.reason || "—") : r.employeeName}
            </p>
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Requested action",
        variant: "text",
      },
    },
    {
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => statusCell(row.original, decidedGrants),
      enableColumnFilter: true,
      meta: {
        label: "Status",
        variant: "select",
        options: APPROVAL_STATUS_OPTIONS,
      },
    },
    {
      accessorKey: "risk",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Risk" />
      ),
      cell: ({ row }) => <span>{row.original.risk || "—"}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Risk",
        variant: "text",
      },
    },
    {
      accessorKey: "requestedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Requested" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.requestedAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Requested",
        variant: "date",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const item = row.original
        const selfApproval = isSelfApproval?.(item) ?? false

        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8 p-0"
                  aria-label="Approval actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuLabel>Approval</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onSelect(item)}>
                  View details
                </DropdownMenuItem>
                {item.status === "pending" && onDecide && (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      disabled={selfApproval}
                      title={
                        selfApproval
                          ? "You requested this access; a different Governance Admin must decide it"
                          : undefined
                      }
                      onClick={() => onDecide(item, "approved")}
                    >
                      Approve
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onClick={() => onDecide(item, "rejected")}
                    >
                      Reject
                    </DropdownMenuItem>
                  </>
                )}
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.action)
                  }}
                >
                  Copy action
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
      enableSorting: false,
      enableHiding: false,
      size: 40,
    },
  ]
}
