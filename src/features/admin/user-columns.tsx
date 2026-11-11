"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Activity,
  Building2,
  Clock,
  Copy,
  Eye,
  KeyRound as KeyRoundIcon,
  Mail,
  MoreHorizontal,
  Shield,
  User as UserIcon,
  UserCheck,
  UserX,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import type { User } from "@/services"

export const USER_STATUS_OPTIONS = [
  { value: "active", label: "Active" },
  { value: "inactive", label: "Inactive" },
]

export function UserStatusPill({ status }: { readonly status: User["status"] }) {
  return status === "active" ? (
    <Pill tone="success">Active</Pill>
  ) : (
    <Pill tone="neutral">Inactive</Pill>
  )
}

export function PillList({ values }: { readonly values: readonly string[] }) {
  if (!values || values.length === 0) {
    return <span className="text-muted-foreground">—</span>
  }
  return (
    <div className="flex flex-wrap gap-1">
      {values.map((v) => (
        <Pill key={v} tone="neutral">
          {v}
        </Pill>
      ))}
    </div>
  )
}

export function getUserColumns({
  onSelect,
  canWrite = false,
  onResetPassword,
  onEditRoles,
  onRemoveFromTenant,
  onToggleStatus,
  onEditIdentity,
}: {
  onSelect: (user: User) => void
  /**
   * Whether the caller may reset a password. ADM-3 D2: the reset route
   * needs `identity:write`, so the action is disabled without it rather
   * than offered and refused by the server.
   */
  canWrite?: boolean
  /**
   * ADM-3 D2: hand this user a new temporary password. Omitted when the
   * caller may not, which hides the action entirely.
   */
  onResetPassword?: (user: User) => void
  /**
   * ADM-4 D1: replace this user's local role set. Omitted when the
   * caller may not, which hides the action entirely.
   */
  onEditRoles?: (user: User) => void
  /**
   * ADM-4 D2: take this user out of a tenant. Omitted when the caller
   * may not, which hides the action entirely.
   */
  onRemoveFromTenant?: (user: User) => void
  /**
   * ADM-4 D3: switch this user off, or back on. The label follows the
   * user's current status.
   */
  onToggleStatus?: (user: User) => void
  /** ADM-11 D8: edit a user's username and email. */
  onEditIdentity?: (user: User) => void
}): ColumnDef<User>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="User" />
      ),
      cell: ({ row }) => {
        const user = row.original
        return (
          <div className="flex flex-col gap-0.5">
            <button
              type="button"
              className="text-left font-medium text-foreground hover:underline focus:outline-none"
              onClick={() => onSelect(user)}
            >
              {user.name}
            </button>
            <span className="text-xs text-muted-foreground">{user.email ?? user.username}</span>
          </div>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "User",
        placeholder: "Filter user name…",
        variant: "text",
        icon: UserIcon,
      },
    },
    {
      id: "username",
      accessorKey: "username",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Username" />
      ),
      cell: ({ row }) => (
        <Copyable value={row.original.username} className="max-w-48 truncate">
          <span className="font-mono text-xs text-foreground">
            {row.original.username}
          </span>
        </Copyable>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Username",
        placeholder: "Filter username…",
        variant: "text",
        icon: UserIcon,
      },
    },
    {
      id: "email",
      accessorKey: "email",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Email" />
      ),
      cell: ({ row }) => {
        const email = row.original.email
        if (!email) {
          return <span className="text-muted-foreground">—</span>
        }
        return (
          <Copyable value={email} className="max-w-48 truncate">
            <span className="font-mono text-xs text-foreground">
              {email}
            </span>
          </Copyable>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Email",
        placeholder: "Filter email…",
        variant: "text",
        icon: Mail,
      },
    },
    {
      id: "status",
      accessorKey: "status",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Status" />
      ),
      cell: ({ row }) => <UserStatusPill status={row.original.status} />,
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Status",
        variant: "select",
        options: USER_STATUS_OPTIONS,
        icon: Activity,
      },
    },
    {
      id: "roles",
      accessorKey: "roles",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Roles" />
      ),
      cell: ({ row }) => <PillList values={row.original.roles} />,
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Roles",
        placeholder: "Filter role…",
        variant: "text",
        icon: Shield,
      },
    },
    {
      id: "tenants",
      accessorKey: "tenants",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tenants" />
      ),
      cell: ({ row }) => <PillList values={row.original.tenants} />,
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Tenants",
        placeholder: "Filter tenant…",
        variant: "text",
        icon: Building2,
      },
    },
    {
      id: "lastActivity",
      accessorKey: "lastActivity",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Last Activity" />
      ),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground whitespace-nowrap">
          {formatRelativeTime(row.original.lastActivity)}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Last Activity",
        variant: "date",
        icon: Clock,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const user = row.original
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="User actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuItem
                  onClick={() => {
                    onSelect(user)
                  }}
                >
                  <Eye className="size-4" />
                  <span>View details</span>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                {user.email ? (
                  <DropdownMenuItem
                    onClick={() => {
                      void navigator.clipboard.writeText(user.email ?? "")
                    }}
                  >
                    <Copy className="size-4" />
                    <span>Copy email</span>
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(user.username)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy username</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(user.id)
                  }}
                >
                  <UserIcon className="size-4" />
                  <span>Copy user ID</span>
                </DropdownMenuItem>
                {onEditIdentity ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      disabled={!canWrite}
                      onClick={() => {
                        onEditIdentity(user)
                      }}
                    >
                      <UserIcon className="size-4" />
                      <span>Edit username and email</span>
                    </DropdownMenuItem>
                  </>
                ) : null}
                {onResetPassword ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      disabled={!canWrite}
                      onClick={() => {
                        onResetPassword(user)
                      }}
                    >
                      <KeyRoundIcon className="size-4" />
                      <span>Reset password</span>
                    </DropdownMenuItem>
                  </>
                ) : null}
                {/* ADM-4 D1/D2/D3: the three changes to what a user may
                    do. Each is hidden when the caller may not make it,
                    and disabled when the caller may not WRITE — the same
                    rule the reset action above follows. */}
                {onEditRoles ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      disabled={!canWrite}
                      onClick={() => {
                        onEditRoles(user)
                      }}
                    >
                      <Shield className="size-4" />
                      <span>Edit roles</span>
                    </DropdownMenuItem>
                  </>
                ) : null}
                {onRemoveFromTenant ? (
                  <DropdownMenuItem
                    disabled={!canWrite}
                    onClick={() => {
                      onRemoveFromTenant(user)
                    }}
                  >
                    <Building2 className="size-4" />
                    <span>Remove from tenant</span>
                  </DropdownMenuItem>
                ) : null}
                {onToggleStatus ? (
                  <DropdownMenuItem
                    disabled={!canWrite}
                    onClick={() => {
                      onToggleStatus(user)
                    }}
                  >
                    {user.status === "active" ? (
                      <UserX className="size-4" />
                    ) : (
                      <UserCheck className="size-4" />
                    )}
                    <span>
                      {user.status === "active" ? "Deactivate" : "Reactivate"}
                    </span>
                  </DropdownMenuItem>
                ) : null}
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
