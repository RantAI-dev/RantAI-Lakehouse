"use client"

import * as React from "react"
import Link from "next/link"
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
import { displayColumn } from "@/lib/chart-joins"
import { formatDateTime } from "@/lib/format"
import { deliverySummary } from "@/features/overview/alert-delivery"
import { LOAD_ALERT_KINDS, QUALITY_ALERT_KIND, type AlertRule } from "@/services/contracts/alerts"

/**
 * `getRuleColumns` used to declare its own looser `Rule` type (every field
 * but `id`/`name`/`channel`/`target`/`enabled` optional), which drifted
 * from the real wire contract and let a `freshness` rule (whose `agg`/`op`/
 * `threshold` the backend genuinely omits) type-check against a page
 * expecting the real `AlertRule`. Using the contract type directly keeps
 * the two in sync.
 */
export type Rule = AlertRule

/** Plain labels for the rule kinds the console knows; any other kind shows as sent. */
export const RULE_KIND_LABEL: Record<string, string> = {
  alert: "Threshold alert",
  digest: "Dashboard digest",
  freshness: "Dataset freshness",
  pipeline_behind_target: "Pipeline behind target lag",
  connector_failure: "Connector run failed",
  connector_repeated_failure: "Connector failed 3 runs in a row",
  connector_disabled: "Connector disabled",
  connector_schema_change: "Connector schema changed",
  connector_success: "Connector run succeeded",
  connector_lag: "Change capture behind its limit",
  upload_failure: "Upload failed",
  quality_incident: "Quality incident opened",
}

export function ruleKindLabel(type: string): string {
  return RULE_KIND_LABEL[type] ?? type
}

/**
 * What the Delivery cell says: the address when the API returned it, else
 * "Webhook" or "E-mail" from the channel (`DEC-16`, QA `LH-SRC-7-04`,
 * `SRC-42`). A hidden rule holds exactly one target, so no count is shown.
 */
export function deliveryLabel(rule: AlertRule): string {
  if (rule.targetHidden === true) return rule.channel === "email" ? "E-mail" : "Webhook"
  return rule.target ?? ""
}

/**
 * The second line of the Delivery cell for a "run succeeded" rule (`SRC-40` D7,
 * QA `TC-1695`): when it last sent, or when it last tried and that the message
 * did not arrive, with `deliverySummary`'s words — the same the incident list
 * uses. `null` when no delivery was recorded, so a rule that never sent shows
 * no line at all.
 */
export function lastDeliveryLabel(rule: Pick<AlertRule, "lastDelivery">): string | null {
  const delivery = rule.lastDelivery
  if (!delivery) return null
  if (delivery.delivered) return `Last sent ${formatDateTime(delivery.at)}`
  const summary = deliverySummary({ delivered: false, deliveryReason: delivery.reason })
  return `Last attempt ${formatDateTime(delivery.at)} · ${summary.text}`
}

/**
 * Whether a rule is a chart alert (`BI-19` part C): the "Alert me..." dialog
 * saves `type: "alert"` with a `chart` id and a `condition`, and no mart.
 * BI-19 owner QA 2026-10-28, QA-9: the list used to read it as a column rule.
 */
export function isChartAlert(rule: Pick<AlertRule, "type" | "chart">): boolean {
  return rule.type === "alert" && !!rule.chart
}

/** The type badge text of a rule: "Chart alert" for a chart alert, the kind's label otherwise. */
export function ruleTypeLabel(rule: Pick<AlertRule, "type" | "chart">): string {
  return isChartAlert(rule) ? "Chart alert" : ruleKindLabel(rule.type)
}

/**
 * What a chart alert watches, in words: the chart and its dashboard when the
 * list knows them, "Dashboard chart" when it does not (a chart the caller
 * cannot see, or an older answer). `BI-39` D8: the two names are read live, so
 * the text follows a rename and never states a chart the reader lost.
 */
export function chartAlertScope(
  rule: Pick<AlertRule, "chartTitle" | "dashboardTitle">
): string {
  return rule.chartTitle && rule.dashboardTitle
    ? `${rule.chartTitle} on ${rule.dashboardTitle}`
    : "Dashboard chart"
}

/**
 * The condition of a chart alert in words (`QA-9`): the scope, then what the
 * number does.
 */
export function chartAlertCondition(
  rule: Pick<AlertRule, "chartTitle" | "dashboardTitle" | "series" | "condition" | "threshold">
): string {
  const series = rule.series ? displayColumn(rule.series) : "the chart"
  const what =
    rule.condition === "above"
      ? `${series} goes above ${rule.threshold}`
      : rule.condition === "below"
        ? `${series} goes below ${rule.threshold}`
        : rule.condition === "any_rows"
          ? "returns any rows"
          : String(rule.condition ?? "")
  return `${chartAlertScope(rule)} · ${what}`.trim()
}

/** Where the chart's name in the condition links to, when the list knows it. */
export function chartAlertHref(
  rule: Pick<AlertRule, "chartTitle" | "dashboardId">
): string | null {
  return rule.chartTitle && rule.dashboardId
    ? `/dashboards/${encodeURIComponent(rule.dashboardId)}`
    : null
}

/** Whether `type` is one of the `SRC-7` kinds for connector and upload loads. */
export function isLoadAlertKind(type: string): boolean {
  return (LOAD_ALERT_KINDS as readonly string[]).includes(type)
}

/** Whether `type` is the quality incident kind (`DATA-14`). */
export function isQualityAlertKind(type: string): boolean {
  return type === QUALITY_ALERT_KIND
}

/**
 * What a rule of the `SRC-7` kinds is scoped to, for the rules table: "Uploads"
 * for an upload rule, "All connectors" for `*`, the connector's name when the
 * list of connectors has it, otherwise its id as stored (a connector the viewer
 * cannot list is not given a made-up name). `null` for every other kind.
 */
export function ruleScope(
  rule: Pick<AlertRule, "type" | "connector"> & { asset?: string; pipeline?: string },
  connectors: readonly { id: string; name: string }[]
): string | null {
  // A quality incident rule is scoped to a table, or to every table.
  if (isQualityAlertKind(rule.type)) return rule.asset === "*" ? "All tables" : (rule.asset ?? null)
  if (rule.type === "pipeline_behind_target") {
    return !rule.pipeline || rule.pipeline === "*" ? "All pipelines" : rule.pipeline
  }
  if (!isLoadAlertKind(rule.type)) return null
  if (rule.type === "upload_failure") return "Uploads"
  if (!rule.connector) return null
  if (rule.connector === "*") return "All connectors"
  return connectors.find((c) => c.id === rule.connector)?.name ?? rule.connector
}

type RuleColumnsOptions = {
  readonly onEdit: (rule: Rule) => void
  readonly onToggle: (rule: Rule) => void
  readonly onRun: (id: string) => void
  readonly onDelete: (id: string) => void
  readonly busy: boolean
  readonly boards: { id: string; name: string }[]
  readonly connectors: { id: string; name: string }[]
}

export function getRuleColumns(options: RuleColumnsOptions): ColumnDef<Rule>[] {
  const { onEdit, onToggle, onRun, onDelete, busy, boards, connectors } = options

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
        options: Object.entries(RULE_KIND_LABEL).map(([value, label]) => ({ label, value })),
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
            {ruleTypeLabel(row.original)}
          </span>
        )
      },
    },
    {
      id: "condition",
      header: "Condition / Board",
      cell: ({ row }) => {
        const r = row.original
        if (isChartAlert(r)) {
          const text = chartAlertCondition(r)
          const href = chartAlertHref(r)
          if (!href || !r.chartTitle) {
            return <span className="text-sm text-muted-foreground">{text}</span>
          }
          // The title is the start of `chartAlertCondition`'s text, so the
          // linked name and the rest read as one sentence.
          return (
            <span className="text-sm text-muted-foreground">
              <Link href={href} className="text-primary hover:underline">
                {r.chartTitle}
              </Link>
              {text.slice(r.chartTitle.length)}
            </span>
          )
        }
        if (r.type === "alert" && r.metric) {
          return (
            <span className="font-mono text-xs text-muted-foreground">
              metric {r.metric} {r.op} {r.threshold}
            </span>
          )
        }
        // A load rule has no mart, measure or board: its scope is the next column.
        if (isLoadAlertKind(r.type) || isQualityAlertKind(r.type)) {
          return <span className="text-sm text-muted-foreground">—</span>
        }
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
      id: "scope",
      header: "Scope",
      cell: ({ row }) => (
        <span className="text-sm text-muted-foreground">
          {ruleScope(row.original, connectors) ?? "—"}
        </span>
      ),
    },
    {
      id: "delivery",
      header: "Delivery",
      cell: ({ row }) => {
        const r = row.original
        const last = lastDeliveryLabel(r)
        return (
          <div className="flex flex-col gap-0.5">
            <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
              {r.channel === "email" ? (
                <Mail className="size-3.5 text-muted-foreground" />
              ) : (
                <Webhook className="size-3.5 text-muted-foreground" />
              )}
              <span className="max-w-50 truncate">{deliveryLabel(r)}</span>
            </span>
            {last === null ? null : (
              <span
                className={cn(
                  "block text-xs",
                  r.lastDelivery?.delivered === false
                    ? "text-destructive"
                    : "text-muted-foreground"
                )}
              >
                {last}
              </span>
            )}
          </div>
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
              // DEC-16 (QA LH-SRC-7-04, SRC-42): the API refuses the change for
              // this caller, so the switch says so instead of offering it.
              disabled={r.targetHidden === true}
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
        // DEC-16 (QA LH-SRC-7-04, SRC-42): testing, editing and deleting are
        // all refused by the API for a rule this caller may not manage.
        if (r.targetHidden === true) return null

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
