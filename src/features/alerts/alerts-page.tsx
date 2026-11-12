"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { Bell, CircleCheck, CircleX, Plus } from "lucide-react"
import { Button } from "@/components/ui/button"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { readableErrorMessage } from "@/lib/notify-message"
import type { QueryKeys } from "@/types/data-table"
import { getRuleColumns, type Rule } from "@/features/alerts/rules-columns"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useService, useServiceAction } from "@/hooks/use-service"
import { alertRuleService, connectorService, dashboardService } from "@/services"
import type { AlertRule, AlertRunResult } from "@/services/contracts/alerts"
// The rule form is its own page (`WEB-7` D2/D7); the list still writes a rule
// back when it is switched on or off, with the same body builder.
import { alertRuleSaveInput } from "./alert-rule-page"

const RULE_QUERY_KEYS: Partial<QueryKeys> = {
  search: "rule_search",
  page: "rule_page",
  perPage: "rule_limit",
  sort: "rule_sort",
  filters: "rule_filter",
}

function formatRunResult(r: AlertRunResult): string {
  if (r.skipped) {
    return `skipped: ${r.skipped}`
  }
  if (r.type === "alert") {
    const formattedVal = Math.round(r.value ?? 0).toLocaleString()
    if (r.fired) {
      if (r.delivered?.ok) {
        return `fired (value ${formattedVal}) · sent`
      }
      const err = r.delivered?.error ?? "unknown error"
      return `fired (value ${formattedVal}) · send failed: ${err}`
    }
    return `ok (value ${formattedVal}, not breached)`
  }
  if (r.delivered?.ok) {
    return "digest sent"
  }
  const err = r.delivered?.error ?? "unknown error"
  return `send failed: ${err}`
}

function RunResultIcon({ r }: { readonly r: AlertRunResult }) {
  if (r.skipped) {
    return <CircleX className="size-4 text-amber-500" />
  }
  if (!r.fired) {
    return <span className="size-4 text-center text-muted-foreground">–</span>
  }
  if (r.delivered?.ok) {
    return <CircleCheck className="size-4 text-emerald-500" />
  }
  return <CircleX className="size-4 text-destructive" />
}

/** The run-alerts notice (D9), or `null` when the API says reports can arrive or could not say. */
export const RUN_ALERTS_OFF =
  "Run alerts are not being reported: PIPELINE_RUN_TOKEN is not set on the API."

export function RunAlertsNotice({ runEventsConfigured }: { readonly runEventsConfigured: boolean | null }) {
  if (runEventsConfigured !== false) return null
  return (
    <div
      role="status"
      className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
    >
      {RUN_ALERTS_OFF}
    </div>
  )
}

export function AlertsPage() {
  const router = useRouter();
  const state = useService((s) => alertRuleService.listRules(s), []);
  // D9: says whether run reports can arrive at all. If it cannot be read, no
  // notice is shown (the rules list reports its own failure).
  const runStatus = useService((s) => alertRuleService.getStatus(s), []);
  const connectorList = useService((s) => connectorService.listConnectors(s), []);
  const connectors = React.useMemo(
    () => (connectorList.status === "success" ? connectorList.data.map((c) => ({ id: c.id, name: c.name })) : []),
    [connectorList]
  );

  // The board names the rules list shows. `WEB-7` D7: through the dashboard
  // service, not a `fetch` of this page's own.
  const boardList = useService((s) => dashboardService.listBoards(s), []);
  const boards = React.useMemo(
    () =>
      boardList.status === "success"
        ? boardList.data.filter((b) => b.id !== "default").map((b) => ({ id: b.id, name: b.name }))
        : [],
    [boardList]
  );

  const removeAction = useServiceAction((signal, id: string) => alertRuleService.deleteRule(id, signal));
  const toggleAction = useServiceAction((signal, rule: AlertRule) =>
    // A load rule goes back with only the fields that apply to it, as a save does.
    alertRuleService.updateRule({ ...alertRuleSaveInput(rule), id: rule.id, enabled: !rule.enabled }, signal)
  );
  // `BI-40` D5: only one rule is tested at a time; "Run all now" is gone,
  // because the whole tick is the scheduler's (`POST /api/alerts/run`).
  const runAction = useServiceAction((signal, id: string) => alertRuleService.testRule(id, signal));

  async function remove(id: string) {
    const result = await removeAction.run(id);
    if (result !== null) state.reload();
  }
  async function toggle(r: AlertRule) {
    const result = await toggleAction.run(r);
    if (result) state.reload();
  }

  async function run(id: string) {
    await runAction.run(id);
  }

  const mutationError = removeAction.error ?? toggleAction.error ?? runAction.error;
  const rules = state.status === "success" ? state.data : [];
  const runResults = runAction.status === "success" ? runAction.data.results : null;

  const columns = React.useMemo(
    () =>
      getRuleColumns({
        // `WEB-7` D2/D7: the rule form is its own page.
        onEdit: (r) => router.push(`/alerts/rules/${encodeURIComponent(r.id)}/edit`),
        onToggle: (r) => void toggle(r),
        onRun: (id) => void run(id),
        onDelete: (id) => void remove(id),
        busy: toggleAction.status === "pending" || runAction.status === "pending",
        boards,
        connectors,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [toggleAction.status, runAction.status, boards, connectors]
  )

  const tableUrlState = useTableUrlState(RULE_QUERY_KEYS)
  const filteredRules = React.useMemo(
    () =>
      filterDataClientSide(
        // `getRuleColumns`/`Rule` predate the real `AlertRule` contract (it
        // has no "freshness" kind or `severity`); a real rule is a
        // structural superset of `Rule`, so this is a widening cast, not a
        // fabrication — every field `Rule` reads is genuinely present.
        rules as unknown as Rule[],
        {
          search: tableUrlState.search,
          searchFields: [(r) => r.name, (r) => r.mart, (r) => r.measure, (r) => r.target ?? ""],
          filters: tableUrlState.filters,
          joinOperator: tableUrlState.joinOperator,
        }
      ),
    [rules, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredRules,
    columns,
    queryKeys: RULE_QUERY_KEYS,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/alerts/rules",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="flex items-center gap-2 text-xl font-semibold">
            <Bell className="size-5" /> Alert Rules &amp; Digests
          </h2>
          <p className="text-sm text-muted-foreground">
            Threshold alerts on Gold metrics + scheduled dashboard digests. Delivered via
            webhook or email.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" onClick={() => router.push("/alerts/rules/new")}>
            <Plus className="size-4" /> New rule
          </Button>
        </div>
      </div>

      <RunAlertsNotice
        runEventsConfigured={runStatus.status === "success" ? runStatus.data.runEventsConfigured : null}
      />

      {mutationError ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {readableErrorMessage(mutationError.message)}
        </div>
      ) : null}

      {runResults ? (
        <div className="rounded-lg border border-border bg-card/50 p-3 text-sm">
          <p className="mb-2 font-medium">Run results ({runResults.length})</p>
          <ul className="space-y-1">
            {runResults.map((r) => (
              <li key={r.id} className="flex items-center gap-2">
                <RunResultIcon r={r} />
                <span className="font-medium">{r.name}</span>
                <span className="text-muted-foreground">{formatRunResult(r)}</span>
              </li>
            ))}
            {runResults.length === 0 ? <li className="text-muted-foreground">No enabled rules.</li> : null}
          </ul>
        </div>
      ) : null}

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch placeholder="Search rules..." />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}

      <p className="text-xs text-muted-foreground">
        Scheduling: hit{" "}
        <code className="rounded bg-muted px-1 font-mono">POST /api/alerts/run</code>{" "}
        periodically from cron (e.g. every 15 min). Email needs{" "}
        <code className="rounded bg-muted px-1 font-mono">
          SMTP_HOST/PORT/USER/PASS/FROM
        </code>{" "}
        env; webhook works out of the box.
      </p>

    </div>
  )
}

export { AlertsPage as AlertRulesPage }
