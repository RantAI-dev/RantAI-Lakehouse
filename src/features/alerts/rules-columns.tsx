"use client"

import * as React from "react"
import type { ColumnDef } from "@tanstack/react-table"
import {
  Mail,
  MoreHorizontal,
  Play,
  Send,
  Trash2,
  Webhook,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"
import type { AlertRule } from "@/services/contracts/alerts"

/**
 * `getRuleColumns` used to declare its own looser `Rule` type (every field
 * but `id`/`name`/`channel`/`target`/`enabled` optional), which drifted
 * from the real wire contract and let a `freshness` rule (whose `agg`/`op`/
 * `threshold` the backend genuinely omits) type-check against a page
 * expecting the real `AlertRule`. Using the contract type directly keeps
 * the two in sync.
 */
export type Rule = AlertRule

type RuleColumnsOptions = {
  readonly onEdit: (rule: Rule) => void
  readonly onToggle: (rule: Rule) => void
  readonly onRun: (id: string) => void
  readonly onDelete: (id: string) => void
  readonly busy: boolean
  readonly boards: { id: string; name: string }[]
}

export function getRuleColumns(options: RuleColumnsOptions): ColumnDef<Rule>[] {
  const { onEdit, onToggle, onRun, onDelete, busy, boards } = options

  return [
    {
      id: "name",
      accessorKey: "name",
      header: "Rule Name",
      cell: ({ row }) => (
        <button
          type="button"
          className="text-left font-medium hover:underline focus:outline-none"
          onClick={() => onEdit(row.original)}
        >
          {row.original.name}
        </button>
      ),
      enableColumnFilter: true,
      meta: {
        label: "Rule Name",
        variant: "text",
      },
    },
    {
      id: "type",
      accessorKey: "type",
      header: "Type",
      enableColumnFilter: true,
      meta: {
        label: "Type",
        variant: "select",
        options: [
          { label: "Threshold alert", value: "alert" },
          { label: "Dashboard digest", value: "digest" },
        ],
      },
      cell: ({ row }) => {
        const isAlert = row.original.type === "alert"
        return (
          <span
            className={cn(
              "rounded px-1.5 py-0.5 text-xs font-medium",
              isAlert
                ? "bg-amber-500/10 text-amber-600 dark:text-amber-400"
                : "bg-sky-500/10 text-sky-600 dark:text-sky-400"
            )}
          >
            {isAlert ? "Threshold alert" : "Dashboard digest"}
          </span>
        )
      },
    },
    {
      id: "condition",
      header: "Condition / Board",
      cell: ({ row }) => {
        const r = row.original
        if (r.type === "alert") {
          return (
            <span className="font-mono text-xs text-muted-foreground">
              {r.agg}({r.measure}) on {r.mart} {r.op} {r.threshold}
            </span>
          )
        }
        const boardName = boards.find((b) => b.id === r.board)?.name ?? r.board
        return <span className="text-sm text-muted-foreground">{boardName}</span>
      },
    },
    {
      id: "delivery",
      header: "Delivery",
      cell: ({ row }) => {
        const r = row.original
        return (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            {r.channel === "email" ? (
              <Mail className="size-3.5 text-muted-foreground" />
            ) : (
              <Webhook className="size-3.5 text-muted-foreground" />
            )}
            <span className="max-w-50 truncate">{r.target}</span>
          </span>
        )
      },
    },
    {
      id: "enabled",
      accessorKey: "enabled",
      header: "Status",
      cell: ({ row }) => {
        const r = row.original
        return (
          <div className="flex items-center gap-2">
            <Switch
              checked={r.enabled}
              onCheckedChange={() => onToggle(r)}
              aria-label={`Toggle rule ${r.name}`}
            />
            <span className="text-xs text-muted-foreground">
              {r.enabled ? "Active" : "Disabled"}
            </span>
          </div>
        )
      },
      enableColumnFilter: true,
      meta: {
        label: "Status",
        variant: "boolean",
      },
    },
    {
      id: "actions",
      header: () => <span className="sr-only">Actions</span>,
      cell: ({ row }) => {
        const r = row.original

        return (
          <div className="flex items-center justify-end gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onRun(r.id)}
              disabled={busy}
              title="Test run now"
              aria-label="Test run now"
            >
              <Send className="size-4" />
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8 p-0"
                  aria-label="More rule actions"
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuLabel>Rule Actions</DropdownMenuLabel>
                <DropdownMenuItem onClick={() => onEdit(r)}>
                  Edit configuration
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => onRun(r.id)} disabled={busy}>
                  <Play className="mr-2 size-4" />
                  Execute test run
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onClick={() => onDelete(r.id)}
                >
                  <Trash2 className="mr-2 size-4" />
                  Delete rule
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )
      },
      enableSorting: false,
      enableHiding: false,
      size: 96,
    },
  ]
}
