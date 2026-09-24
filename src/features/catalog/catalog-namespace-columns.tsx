"use client"

import type { ColumnDef } from "@tanstack/react-table"
import Link from "next/link"
import {
  Boxes,
  Cpu,
  Database,
  Eye,
  FileText,
  Globe,
  MoreHorizontal,
  Search,
  User,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Badge } from "@/components/ui/badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { namespaceAssetsHref } from "@/lib/table-filter-link"
import type { CatalogNamespace } from "@/services/contracts/assets"

export function getCatalogNamespaceColumns({
  onSelect,
}: {
  onSelect: (namespace: CatalogNamespace) => void
}): ColumnDef<CatalogNamespace>[] {
  return [
    {
      id: "name",
      accessorKey: "name",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Namespace" />
      ),
      cell: ({ row }) => (
        <Copyable value={row.original.name} className="max-w-56 truncate">
          <span className="font-mono text-sm font-semibold tracking-tight text-foreground">
            {row.original.name}
          </span>
        </Copyable>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Namespace",
        placeholder: "Filter namespace…",
        variant: "text",
        icon: Boxes,
      },
    },
    {
      id: "description",
      accessorKey: "description",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Description" />
      ),
      cell: ({ row }) => (
        <span
          className="line-clamp-2 max-w-[20rem] text-sm text-muted-foreground"
          title={row.original.description}
        >
          {row.original.description || "—"}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Description",
        placeholder: "Filter description…",
        variant: "text",
        icon: FileText,
      },
    },
    {
      id: "assetCount",
      accessorKey: "assetCount",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Assets" align="right" />
      ),
      cell: ({ row }) => (
        <div className="text-right font-mono text-sm font-medium tabular-nums">
          {row.original.assetCount.toLocaleString()}
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Assets",
        variant: "number",
        icon: Database,
      },
    },
    {
      id: "owner",
      accessorKey: "owner",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Owner" />
      ),
      cell: ({ row }) => (
        <span className="text-sm font-medium text-foreground">
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
      id: "sourceEngine",
      accessorKey: "sourceEngine",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Source Engine" />
      ),
      cell: ({ row }) => (
        <Badge variant="outline" className="font-mono text-xs font-normal">
          {row.original.sourceEngine}
        </Badge>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Source Engine",
        placeholder: "Filter engine…",
        variant: "text",
        icon: Cpu,
      },
    },
    {
      id: "residency",
      accessorKey: "residency",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Residency" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.residency}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Residency",
        placeholder: "Filter residency…",
        variant: "text",
        icon: Globe,
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
                aria-label="Namespace actions"
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
                <DropdownMenuItem asChild>
                  <Link
                    href={namespaceAssetsHref(item.name)}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <Search className="size-4" />
                    <span>Browse assets</span>
                  </Link>
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(item.name)
                  }}
                >
                  <Boxes className="size-4" />
                  <span>Copy namespace</span>
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
