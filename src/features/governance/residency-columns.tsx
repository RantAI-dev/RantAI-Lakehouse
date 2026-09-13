"use client"

import type { ColumnDef } from "@tanstack/react-table"
import {
  AlertTriangle,
  ArrowRightCircle,
  Building2,
  Copy,
  Globe,
  MoreHorizontal,
  Share2,
  Shield,
} from "lucide-react"

import { DataTableColumnHeader } from "@/components/data-table/data-table-column-header"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import type { ResidencyRule } from "@/services/contracts/governance"

export const CLASSIFICATION_OPTIONS = (
  Object.keys(CLASSIFICATION_LABEL) as Classification[]
).map((c) => ({ value: c, label: CLASSIFICATION_LABEL[c] }))

export function getResidencyColumns(): ColumnDef<ResidencyRule>[] {
  return [
    {
      id: "tenant",
      accessorKey: "tenant",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Tenant" />
      ),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <Building2 className="size-4 shrink-0 text-muted-foreground" />
          <span className="font-mono text-sm font-medium text-foreground">
            {row.original.tenant}
          </span>
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Tenant",
        placeholder: "Filter tenant…",
        variant: "text",
        icon: Building2,
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
        icon: Shield,
      },
    },
    {
      id: "approvedSites",
      accessorFn: (r) => r.approvedSites.join(", "),
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Approved Sites" />
      ),
      cell: ({ row }) => (
        <div className="flex flex-wrap gap-1">
          {row.original.approvedSites.map((s) => (
            <Pill key={s} tone="neutral">
              {s}
            </Pill>
          ))}
        </div>
      ),
      enableColumnFilter: true,
      enableSorting: false,
      meta: {
        label: "Approved Sites",
        placeholder: "Filter sites…",
        variant: "text",
        icon: Globe,
      },
    },
    {
      id: "crossSiteAllowed",
      accessorKey: "crossSiteAllowed",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Cross-site" />
      ),
      cell: ({ row }) =>
        row.original.crossSiteAllowed ? (
          <Pill tone="neutral">Cross-site allowed</Pill>
        ) : (
          <Pill tone="warning">Single site</Pill>
        ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Cross-site",
        variant: "select",
        options: [
          { value: "true", label: "Cross-site allowed" },
          { value: "false", label: "Single site" },
        ],
        icon: Share2,
      },
    },
    {
      id: "allowedOutput",
      accessorKey: "allowedOutput",
      header: ({ column }) => (
        <DataTableColumnHeader column={column} label="Allowed Output" />
      ),
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {row.original.allowedOutput}
        </span>
      ),
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Allowed Output",
        placeholder: "Filter allowed output…",
        variant: "text",
        icon: ArrowRightCircle,
      },
    },
    {
      id: "violations7d",
      accessorKey: "violations7d",
      header: ({ column }) => (
        <DataTableColumnHeader
          column={column}
          label="Violations 7d"
          align="right"
        />
      ),
      cell: ({ row }) => {
        const v = row.original.violations7d
        return (
          <div className="text-right font-mono text-sm">
            {v > 0 ? (
              <span className="font-semibold text-destructive">{v}</span>
            ) : (
              <span className="text-muted-foreground">{v}</span>
            )}
          </div>
        )
      },
      enableColumnFilter: true,
      enableSorting: true,
      meta: {
        label: "Violations 7d",
        variant: "number",
        icon: AlertTriangle,
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
                aria-label="Residency actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(item.tenant)
                  }}
                >
                  <Copy className="size-4" />
                  <span>Copy tenant</span>
                </DropdownMenuItem>
                <DropdownMenuItem
                  onClick={(e) => {
                    e.stopPropagation()
                    void navigator.clipboard.writeText(item.id)
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
