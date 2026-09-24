"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  ArrowDownRight,
  ArrowUpRight,
  Copy,
  GitCommit,
  MoreHorizontal,
  Tag,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { Pill } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import type { LineageEdge, LineageGraph } from "@/services/contracts/governance"

export function getLineageColumns({
  nodes,
}: {
  nodes: LineageGraph["nodes"]
}): ColumnDef<LineageEdge>[] {
  const nodeMap = new Map(nodes.map((n) => [n.id, n.label]))
  const getLabel = (id: string) => nodeMap.get(id) ?? id

  return [
    {
      id: "from",
      accessorFn: (r) => getLabel(r.from),
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="From" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <ArrowUpRight className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-medium text-foreground">
            {getLabel(row.original.from)}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "From",
        placeholder: "Filter source…",
        variant: "text",
        icon: ArrowUpRight,
      },
    },
    {
      id: "to",
      accessorFn: (r) => getLabel(r.to),
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="To" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <ArrowDownRight className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-medium text-foreground">
            {getLabel(row.original.to)}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "To",
        placeholder: "Filter target…",
        variant: "text",
        icon: ArrowDownRight,
      },
    },
    {
      id: "via",
      accessorKey: "kind",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Via" />
      ),
      cell: ({ row }) => <Pill tone="neutral">{row.original.kind}</Pill>,
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Via",
        variant: "text",
        placeholder: "Filter kind…",
        icon: Tag,
      },
    },
    {
      id: "actions",
      header: () => null,
      cell: ({ row }) => {
        const edge = row.original
        const fromLabel = getLabel(edge.from)
        const toLabel = getLabel(edge.to)
        return (
          <div className="flex items-center justify-end">
            <DropdownMenu>
              <DropdownMenuTrigger
                className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus:outline-none"
                aria-label="Connection actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(fromLabel)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy From label</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(toLabel)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy To label</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(edge.id)
                  }}
                >
                  <GitCommit className="size-4" />
                  <span>Copy edge ID</span>
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
