"use client"

import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import { Copy, FolderInput, MoreHorizontal, Pencil, Trash2 } from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { formatRelativeTime } from "@/lib/format"
import type { Board } from "@/services/clients/bi-store"

export type BoardRowActions = {
  onRename: (board: Board) => void
  onDuplicate: (board: Board) => void
  onDelete: (board: Board) => void
  /** Absent when the viewer may not file dashboards (`dashboard:write`). */
  onMove?: (board: Board) => void
}

/**
 * The ⋯ menu of one dashboard, shared by the table row and the gallery
 * card so both offer the same actions. Renders nothing for the built-in
 * board: it has no table row, so it cannot be renamed, moved or deleted,
 * and a menu whose every item is dead is worse than none.
 */
export function BoardActionsMenu({
  board: b,
  actions,
}: {
  readonly board: Board
  readonly actions: BoardRowActions
}) {
  if (b.builtin) return null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="size-8 p-0"
            aria-label={`Actions for ${b.name}`}
          />
        }
      >
        <MoreHorizontal className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
        <DropdownMenuItem onClick={() => actions.onRename(b)}>
          <Pencil className="size-4" /> Rename
        </DropdownMenuItem>
        {actions.onMove ? (
          <DropdownMenuItem onClick={() => actions.onMove?.(b)}>
            <FolderInput className="size-4" /> Move to folder…
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem onClick={() => actions.onDuplicate(b)}>
          <Copy className="size-4" /> Duplicate
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onClick={() => actions.onDelete(b)}>
          <Trash2 className="size-4" /> Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * Badge untuk board yang dibagikan keluar. Sengaja ditampilkan di daftar,
 * bukan hanya di dialog Share: siapa pun yang melihat daftar ini berhak tahu
 * board mana yang sudah bisa dibaca tanpa login.
 *
 * Hanya menandai, tidak mengubah. Membagikan sebuah board dilakukan dari
 * kanvasnya, tempat isi yang dibagikan itu terlihat.
 */
export function ShareBadges({ board }: { readonly board: Board }) {
  const isPublic = Boolean(board.publicToken)
  if (!isPublic && !board.embedEnabled) return null
  return (
    <span className="flex flex-wrap gap-1">
      {isPublic ? <Pill tone="warning">Public</Pill> : null}
      {board.embedEnabled ? <Pill tone="neutral">Embed</Pill> : null}
    </span>
  )
}

/**
 * `folderPathOf` turns a board's `folderId` into "Sales / Q1" ("" for no
 * folder). The table shows it as a column, sortable and filterable, since
 * a flat table cannot carry the gallery's folder sections.
 */
export function getDashboardColumns(
  actions: BoardRowActions,
  folderPathOf: (board: Board) => string,
): ColumnDef<Board>[] {
  return [
    {
      accessorKey: "name",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Dashboard" />,
      cell: ({ row }) => {
        const b = row.original
        return (
          <div className="min-w-0">
            <Link
              href={`/dashboards/${b.id}`}
              className="font-medium hover:underline"
            >
              {b.name}
            </Link>
            {b.description ? (
              <p className="truncate text-xs text-muted-foreground">{b.description}</p>
            ) : null}
          </div>
        )
      },
      enableColumnFilter: true,
      meta: { label: "Dashboard", variant: "text" },
    },
    {
      id: "folder",
      accessorFn: (b) => folderPathOf(b),
      header: ({ column }) => <DataTableColumnHeader column={column} label="Folder" />,
      cell: ({ row }) => (
        <span className="text-muted-foreground">{folderPathOf(row.original) || "—"}</span>
      ),
      enableColumnFilter: true,
      meta: { label: "Folder", variant: "text" },
    },
    {
      accessorKey: "chartCount",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Tiles" />,
      cell: ({ row }) => (
        <span className="tabular-nums">{row.original.chartCount ?? 0}</span>
      ),
      enableColumnFilter: true,
      meta: { label: "Tiles", variant: "number" },
    },
    {
      id: "sharing",
      header: () => <span>Sharing</span>,
      cell: ({ row }) => <ShareBadges board={row.original} />,
      enableSorting: false,
    },
    {
      accessorKey: "createdBy",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Owner" />,
      cell: ({ row }) => (
        <span className="text-muted-foreground">{row.original.createdBy || "—"}</span>
      ),
      enableColumnFilter: true,
      meta: { label: "Owner", variant: "text" },
    },
    {
      accessorKey: "updatedAt",
      header: ({ column }) => <DataTableColumnHeader column={column} label="Updated" />,
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {formatRelativeTime(row.original.updatedAt)}
        </span>
      ),
      enableColumnFilter: true,
      meta: { label: "Updated", variant: "date" },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const b = row.original
        if (b.builtin) {
          return (
            <div className="flex items-center justify-end">
              <span className="text-xs text-muted-foreground">Built-in</span>
            </div>
          )
        }
        return (
          <div className="flex items-center justify-end">
            <BoardActionsMenu board={b} actions={actions} />
          </div>
        )
      },
      enableSorting: false,
      enableHiding: false,
    },
  ]
}
