"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  CheckCircle2,
  Copy,
  Database,
  Eye,
  KeyRound,
  MoreHorizontal,
  Percent,
  ShieldAlert,
  TableProperties,
} from "lucide-react"

import { Copyable } from "@/components/copyable"
import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { formatPercent } from "@/lib/format"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import type { ClassificationRule } from "@/services/contracts/governance"

export type ReviewStatus = ClassificationRule["reviewStatus"]

export const REVIEW_META: Record<
  ReviewStatus,
  { tone: "success" | "info" | "warning"; label: string }
> = {
  reviewed: { tone: "success", label: "Reviewed" },
  auto: { tone: "info", label: "Auto" },
  "needs-review": { tone: "warning", label: "Needs review" },
}

export const REVIEW_OPTIONS = (Object.keys(REVIEW_META) as ReviewStatus[]).map(
  (s) => ({
    value: s,
    label: REVIEW_META[s].label,
  })
)

export const CLASSIFICATION_OPTIONS = (
  Object.keys(CLASSIFICATION_LABEL) as Classification[]
).map((c) => ({ value: c, label: CLASSIFICATION_LABEL[c] }))

export function getClassificationColumns({
  onSelect,
}: {
  onSelect: (rule: ClassificationRule) => void
}): ColumnDef<ClassificationRule>[] {
  return [
    {
      id: "asset",
      accessorKey: "asset",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Asset" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Database className="size-4 shrink-0 text-muted-foreground" />
          <button
            type="button"
            className="text-left font-mono text-sm font-semibold tracking-tight text-foreground hover:underline focus:outline-none"
            onClick={() => onSelect(row.original)}
          >
            {row.original.asset}
          </button>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Asset",
        placeholder: "Filter asset…",
        variant: "text",
        icon: Database,
      },
    },
    {
      id: "column",
      accessorKey: "column",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Column" />
      ),
      cell: ({ row }) => {
        const col = row.original.column
        if (!col) return <span className="text-muted-foreground">—</span>
        return (
          <Copyable value={col} className="max-w-48 truncate">
            <span className="font-mono text-xs text-foreground">{col}</span>
          </Copyable>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Column",
        placeholder: "Filter column…",
        variant: "text",
        icon: TableProperties,
      },
    },
    {
      id: "classification",
      accessorKey: "classification",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Classification" />
      ),
      cell: ({ row }) => (
        <ClassificationBadge classification={row.original.classification} />
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Classification",
        variant: "select",
        options: CLASSIFICATION_OPTIONS,
        icon: ShieldAlert,
      },
    },
    {
      id: "confidence",
      accessorKey: "confidence",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Confidence" align="right" />
      ),
      cell: ({ row }) => (
        <div className="text-right font-mono text-sm font-medium tabular-nums">
          {formatPercent(row.original.confidence)}
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Confidence",
        variant: "number",
        icon: Percent,
      },
    },
    {
      id: "reviewStatus",
      accessorKey: "reviewStatus",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Review Status" />
      ),
      cell: ({ row }) => {
        const meta = REVIEW_META[row.original.reviewStatus]
        return <Pill tone={meta.tone}>{meta.label}</Pill>
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Review Status",
        variant: "select",
        options: REVIEW_OPTIONS,
        icon: CheckCircle2,
      },
    },
    {
      id: "maskingRule",
      accessorKey: "maskingRule",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Masking" />
      ),
      cell: ({ row }) => {
        const mask = row.original.maskingRule
        if (!mask) return <span className="text-muted-foreground">—</span>
        return (
          <span className="font-mono text-xs rounded bg-muted/60 px-1.5 py-0.5 text-muted-foreground">
            {mask}
          </span>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Masking",
        placeholder: "Filter masking…",
        variant: "text",
        icon: KeyRound,
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
                aria-label="Rule actions"
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
                    void navigator.clipboard.writeText(rule.asset)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy asset</span>
                </DropdownMenuItem>
                {rule.column ? (
                  <DropdownMenuItem
                    onClick={(e) => {
                      e.stopPropagation()
                      void navigator.clipboard.writeText(rule.column!)
                    }}
                  >
                    <TableProperties className="size-4" />
                    <span>Copy column</span>
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
