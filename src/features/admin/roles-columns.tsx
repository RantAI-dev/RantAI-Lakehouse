"use client"

import type { ColumnDef } from "@tanstack/react-table"
import { MoreHorizontal } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatNumber } from "@/lib/format"
import type { Role } from "@/services/contracts/identity"

/**
 * A Roles-list row: the role plus the table-access summary its column shows
 * (`GOV-7` D8). The summary is worked out by the page, because the roles
 * endpoint carries no read scope and each role's access is read once.
 */
export type RoleRow = Role & {
  /** "All tables", "N schemas, M tables", or "—" when the read failed. */
  tables: string
  /** The read failed: the cell says it could not be read, not "All tables". */
  tablesFailed: boolean
}

/**
 * The Roles list. `onEdit` / `onDelete` are given only to someone with
 * `identity:write` (`WEB-3` S3-D8); without them the menu only copies.
 */
export function getRoleColumns({
  onEdit,
  onDelete,
}: {
  onEdit?: (role: Role) => void
  onDelete?: (role: Role) => void
} = {}): ColumnDef<RoleRow>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Role" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Role",
        variant: "text",
      },
    },
    {
      accessorKey: "members",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Members" />
      ),
      cell: ({ row }) => <span>{formatNumber(row.original.members)}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Members",
        variant: "number",
      },
    },
    {
      accessorKey: "permissions",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Permissions" />
      ),
      cell: ({ row }) => (
        <span className="font-mono text-xs">{row.original.permissions}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Permissions",
        variant: "text",
      },
    },
    {
      id: "tables",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tables" />
      ),
      cell: ({ row }) => {
        const item = row.original
        return (
          <span
            className={item.tablesFailed ? "text-muted-foreground" : undefined}
            title={item.tablesFailed ? "This role's tables could not be read." : undefined}
          >
            {item.tables}
          </span>
        )
      },
      enableSorting: false,
      enableColumnFilter: false,
    },
    {
      accessorKey: "description",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Description" />
      ),
      cell: ({ row }) => (
        <span className="text-muted-foreground">{row.original.description}</span>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Description",
        variant: "text",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const item = row.original

        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8 p-0"
                  aria-label={`Actions for role ${item.name}`}
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-40">
                <DropdownMenuLabel>Role</DropdownMenuLabel>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard?.writeText(item.name)
                  }}
                >
                  Copy role name
                </DropdownMenuItem>
                {onEdit || onDelete ? <DropdownMenuSeparator /> : null}
                {onEdit ? (
                  <DropdownMenuItem onClick={() => onEdit(item)}>Edit role</DropdownMenuItem>
                ) : null}
                {onDelete ? (
                  <DropdownMenuItem variant="destructive" onClick={() => onDelete(item)}>
                    Delete role
                  </DropdownMenuItem>
                ) : null}
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
