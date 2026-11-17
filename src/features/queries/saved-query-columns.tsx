"use client"

import * as React from "react"
import Link from "next/link"
import type { ColumnDef } from "@tanstack/react-table"
import {
  CodeIcon,
  CopyIcon,
  CopyPlusIcon,
  EllipsisIcon,
  ExternalLinkIcon,
  EyeIcon,
  FolderInputIcon,
  HistoryIcon,
  PencilIcon,
  Share2Icon,
  Trash2Icon,
} from "lucide-react"
import { toast } from "sonner"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { levelAtLeast } from "@/lib/access-level"
import { formatRelativeTime } from "@/lib/format"
import type { SavedQuery } from "@/services/contracts/queries"

/**
 * What a row's menu does when it is picked.
 *
 * The menu offers what the caller may do with the query, and the level the
 * server sends with the row decides that (`QS-3` C2): a viewer may open,
 * copy and duplicate it and read its history, an editor may also rename
 * it and take it to the trash, and a manager may also move and share it.
 * The server refuses the rest, so the console does not offer it.
 */
export type SavedQueryRowActions = {
  readonly onInspect: (query: SavedQuery) => void
  readonly onDuplicate: (query: SavedQuery) => void
  readonly onRename: (query: SavedQuery) => void
  readonly onMove: (query: SavedQuery) => void
  readonly onHistory: (query: SavedQuery) => void
  readonly onDelete: (query: SavedQuery) => void
  readonly onShare: (query: SavedQuery) => void
}

export function TagPills({ tags }: { readonly tags: readonly string[] }) {
  if (tags.length === 0) return <span className="text-muted-foreground">—</span>
  return (
    <span className="flex flex-wrap gap-1">
      {tags.map((t) => (
        <Pill key={t} tone="neutral">
          {t}
        </Pill>
      ))}
    </span>
  )
}

export function getSavedQueryColumns(
  actions: SavedQueryRowActions
): ColumnDef<SavedQuery>[] {
  const { onInspect, onDuplicate, onRename, onMove, onHistory, onDelete, onShare } = actions

  return [
    {
      id: "select",
      header: ({ table }) => (
        <Checkbox
          checked={
            table.getIsAllPageRowsSelected() ||
            (table.getIsSomePageRowsSelected() && "indeterminate")
          }
          onCheckedChange={(val) => table.toggleAllPageRowsSelected(Boolean(val))}
          aria-label="Select all"
          className="translate-y-0.5"
        />
      ),
      cell: ({ row }) => (
        <Checkbox
          checked={row.getIsSelected()}
          onCheckedChange={(val) => row.toggleSelected(Boolean(val))}
          aria-label="Select row"
          className="translate-y-0.5"
        />
      ),
      size: 40,
      enableSorting: false,
      enableHiding: false,
    },
    {
      accessorKey: "title",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Title" />
      ),
      cell: ({ row }) => <span className="font-medium">{row.original.title}</span>,
      enableColumnFilter: true,
      meta: {
        label: "Title",
        variant: "text",
      },
    },
    {
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      enableColumnFilter: true,
      meta: {
        label: "Owner",
        variant: "text",
      },
    },
    {
      accessorKey: "tags",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tags" />
      ),
      cell: ({ row }) => <TagPills tags={row.original.tags} />,
      enableSorting: false,
    },
    {
      accessorKey: "updatedAt",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Updated" />
      ),
      cell: ({ row }) => formatRelativeTime(row.original.updatedAt),
      enableColumnFilter: true,
      meta: {
        label: "Updated",
        variant: "date",
      },
    },
    {
      id: "actions",
      size: 48,
      enableSorting: false,
      enableHiding: false,
      cell: ({ row }) => {
        const query = row.original
        return (
          <div className="flex justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Actions for ${query.title}`}
                  />
                }
              >
                <EllipsisIcon className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onInspect(query)}>
                  <EyeIcon className="size-3.5" data-icon="inline-start" />
                  Inspect query
                </DropdownMenuItem>
                <DropdownMenuItem
                  render={<Link href={`/query-studio?saved=${encodeURIComponent(query.id)}`} />}
                >
                  <ExternalLinkIcon className="size-3.5" data-icon="inline-start" />
                  Open in Studio
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(query.sql)
                    toast.success("SQL copied to clipboard")
                  }}
                >
                  <CodeIcon className="size-3.5" data-icon="inline-start" />
                  Copy SQL
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={() => {
                    navigator.clipboard.writeText(query.id)
                    toast.success("Query ID copied to clipboard")
                  }}
                >
                  <CopyIcon className="size-3.5" data-icon="inline-start" />
                  Copy Query ID
                </DropdownMenuItem>
                {/* A copy is the caller's own work and changes nothing the
                    original holds, so a viewer may take one too. */}
                <DropdownMenuItem onClick={() => onDuplicate(query)}>
                  <CopyPlusIcon className="size-3.5" data-icon="inline-start" />
                  Duplicate
                </DropdownMenuItem>
                {/* Reading the history is part of reading the query, so it
                    is offered to a viewer too (C2). */}
                <DropdownMenuItem onClick={() => onHistory(query)}>
                  <HistoryIcon className="size-3.5" data-icon="inline-start" />
                  History
                </DropdownMenuItem>
                {/* Renaming and deleting a query need **edit**: the server
                    refuses the change below that level (C2). */}
                {levelAtLeast(query.level, "edit") ? (
                  <DropdownMenuItem onClick={() => onRename(query)}>
                    <PencilIcon className="size-3.5" data-icon="inline-start" />
                    Rename
                  </DropdownMenuItem>
                ) : null}
                {/* A move changes who can see the query, so it needs
                    **manage**, as the server's rule does (`QS-3` review of #382). */}
                {levelAtLeast(query.level, "manage") ? (
                  <DropdownMenuItem onClick={() => onMove(query)}>
                    <FolderInputIcon className="size-3.5" data-icon="inline-start" />
                    Move
                  </DropdownMenuItem>
                ) : null}
                {levelAtLeast(query.level, "edit") ? (
                  <DropdownMenuItem variant="destructive" onClick={() => onDelete(query)}>
                    <Trash2Icon className="size-3.5" data-icon="inline-start" />
                    Delete
                  </DropdownMenuItem>
                ) : null}
                {/* Sharing needs **manage**: both access routes of a saved
                    query ask for it, so the console offers the panel only
                    where the server would answer (C3). */}
                {levelAtLeast(query.level, "manage") ? (
                  <DropdownMenuItem onClick={() => onShare(query)}>
                    <Share2Icon className="size-3.5" data-icon="inline-start" />
                    Share
                  </DropdownMenuItem>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
    },
  ]
}
