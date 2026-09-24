"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  Activity,
  Building2,
  Clock,
  Copy,
  Eye,
  Mail,
  MoreHorizontal,
  Shield,
  User as UserIcon,
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
import type { User } from "@/services/contracts/identity"

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
}: {
  onSelect: (user: User) => void
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
            <span className="text-xs text-muted-foreground">{user.email}</span>
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
      id: "email",
      accessorKey: "email",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Email" />
      ),
      cell: ({ row }) => (
        <Copyable value={row.original.email} className="max-w-48 truncate">
          <span className="font-mono text-xs text-foreground">
            {row.original.email}
          </span>
        </Copyable>
      ),
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
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(user.email)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy email</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    void navigator.clipboard.writeText(user.id)
                  }}
                >
                  <UserIcon className="size-4" />
                  <span>Copy user ID</span>
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
