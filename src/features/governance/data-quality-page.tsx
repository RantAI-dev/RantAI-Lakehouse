"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { CheckBadge, SeverityBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useUrlTab } from "@/hooks/use-url-tab"
import { filterDataClientSide } from "@/lib/data-table"
import { formatRelativeTime } from "@/lib/format"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import { governanceService } from "@/services"
import type { QualityRule } from "@/services/contracts/governance"
import { getDataQualityColumns } from "./data-quality-columns"
import { QualityIncidents } from "./quality-incidents"
import { FreshnessTargetsTab } from "./freshness-targets-tab"
import { EditRuleDialog } from "./quality-rule-edit-dialog"
import { RuleRuns } from "./quality-rule-runs"
import { QualitySchedulesTab } from "./quality-schedules-tab"

const QUALITY_TABS = ["rules", "schedules", "freshness"] as const
type QualityTab = (typeof QUALITY_TABS)[number]

/**
 * Data Quality, in three tabs (`WEB-3` S3-D5): the incidents and rules, the
 * tables' check schedules, and the tables' freshness targets. Schedules and
 * targets were set only on each table's page, one at a time; the tabs show
 * them all, and the table page keeps its own controls. The open tab is in
 * `?tab=`; the rules are the bare address.
 */
export function DataQualityPage() {
  const [tab, setTab] = useUrlTab("tab", QUALITY_TABS)
  const state = useService((s) => governanceService.listQuality(s), [])
  const [selected, setSelected] = React.useState<QualityRule | null>(null)
  const tableUrlState = useTableUrlState()

  const runCheck = useServiceAction(
    withNotify(
      { success: "Check ran", error: "The check could not be run" },
      (signal, id: string) => governanceService.runQualityRule(id, signal)
    )
  )
  const { hasPermission } = useAuth()
  const [deleting, setDeleting] = React.useState<QualityRule | null>(null)
  const [editing, setEditing] = React.useState<QualityRule | null>(null)
  const removeRule = useServiceAction(
    withNotify(
      { success: "Quality rule deleted", error: "The rule could not be deleted" },
      (signal, id: string) => governanceService.deleteQualityRule(id, signal)
    )
  )
  const columns = React.useMemo(
    () => getDataQualityColumns({ onSelect: setSelected }),
    []
  )

  // QA `LH-DATA-14-01` F1 (`DATA-54`): the incidents panel is read again
  // after a rule ran, was added, edited or deleted.
  const [incidentsKey, setIncidentsKey] = React.useState(0)

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.name,
        (r) => r.asset,
        (r) => r.dimension,
        (r) => r.threshold,
        (r) => r.severity,
        (r) => r.lastStatus,
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    state.status,
    state.data,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/governance/data-quality",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  async function handleRun(rule: QualityRule) {
    const result = await runCheck.run(rule.id)
    if (!result) return
    // The drawer shows what just ran; the list catches up behind it.
    setSelected({
      ...rule,
      lastStatus: result.status,
      lastValue: result.value,
      lastRunAt: new Date().toISOString(),
    })
    setIncidentsKey((k) => k + 1)
    state.reload()
  }

  async function handleDelete() {
    if (!deleting) return
    // `run` resolves to `null` only on failure, which `withNotify` reports.
    const ok = await removeRule.run(deleting.id)
    if (ok === null) return
    setDeleting(null)
    setSelected(null)
    setIncidentsKey((k) => k + 1)
    state.reload()
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Data Quality"
        description="The checks on each table, when they run, and how fresh each table must be."
        actions={
          // SEC-23 (F4): the API now refuses adding a rule without
          // governance:write, like editing and deleting one. WEB-3 S3-D5:
          // adding a rule belongs to the rules tab. WEB-7 D2: the form is
          // its own page.
          tab === "rules" && hasPermission("governance:write") ? (
            <Button size="sm" render={<Link href="/governance/data-quality/rules/new" />}>
              <PlusIcon data-icon="inline-start" />
              Add Quality Rule
            </Button>
          ) : null
        }
      />

      <Tabs value={tab} onValueChange={(v) => setTab(v as QualityTab)} className="gap-4">
        <TabsList>
          <TabsTrigger value="rules">Incidents and rules</TabsTrigger>
          <TabsTrigger value="schedules">Schedules</TabsTrigger>
          <TabsTrigger value="freshness">Freshness targets</TabsTrigger>
        </TabsList>
        <TabsContent value="rules" className="flex flex-col gap-4">
          {/* DATA-14 (F7): what is wrong now leads; the rules follow. */}
          <QualityIncidents reloadKey={incidentsKey} />

          {state.status === "loading" ? <LoadingSkeleton /> : null}
          {state.status === "error" ? (
            <ErrorState error={state.error} onRetry={state.reload} />
          ) : null}
          {state.status === "success" ? (
            <div className="flex flex-col gap-4">
              <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
                <DataTableSearch placeholder="Search rule, asset, dimension…" />
              </DataTableAdvancedToolbar>
              <DataTable table={table} onRowClick={setSelected} />
            </div>
          ) : null}
        </TabsContent>
        <TabsContent value="schedules">
          <QualitySchedulesTab canWrite={hasPermission("governance:write")} />
        </TabsContent>
        <TabsContent value="freshness">
          <FreshnessTargetsTab canWrite={hasPermission("governance:write")} />
        </TabsContent>
      </Tabs>

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
      >
        {selected ? (
          <>
            <div className="flex items-center gap-2">
              {selected.lastStatus === null ? (
                // A rule nobody has run has no verdict to badge — render
                // that honestly instead of guessing a `CheckStatus`.
                <span className="text-muted-foreground">
                  {selected.evaluable === false ? "Can't be run" : "Not run yet"}
                </span>
              ) : (
                <CheckBadge status={selected.lastStatus} />
              )}
              <SeverityBadge severity={selected.severity} />
              {selected.lastValue ? (
                <span className="text-sm text-muted-foreground">{selected.lastValue}</span>
              ) : null}
            </div>
            {selected.evaluable === false && selected.hint ? (
              <p className="text-sm text-muted-foreground">{selected.hint}</p>
            ) : null}
            <MetadataList
              items={[
                { label: "Asset", value: selected.asset },
                { label: "Dimension", value: selected.dimension },
                { label: "Threshold", value: selected.threshold },
                {
                  label: "Last run",
                  value: formatRelativeTime(selected.lastRunAt),
                },
              ]}
            />
            {selected.evaluable !== undefined ? (
              <section className="flex flex-col gap-2">
                <h3 className="text-sm font-semibold">Run history</h3>
                <RuleRuns
                  key={`${selected.id}-${selected.lastRunAt ?? "never"}`}
                  ruleId={selected.id}
                  threshold={selected.threshold}
                />
              </section>
            ) : null}
            <div className="flex flex-wrap gap-2">
              {/* QA LH-DATA-14-04 (GOV-10, DEC-17): running a rule by hand is
                  a governance action, like editing one — the API takes
                  `governance:write` for it, so the button follows. */}
              {selected.evaluable && hasPermission("governance:write") ? (
                <Button
                  size="sm"
                  onClick={() => void handleRun(selected)}
                  disabled={runCheck.status === "pending"}
                >
                  {runCheck.status === "pending" ? "Running…" : "Run check"}
                </Button>
              ) : null}
              <Button
                variant="outline"
                size="sm"
                render={
                  <Link href={`/catalog?q=${encodeURIComponent(selected.asset)}`} />
                }
              >
                Find in Catalog
              </Button>
              {/* `evaluable` is only on an authored rule; a verdict a job
                  recorded is not a rule this page can change or delete. */}
              {selected.evaluable !== undefined && hasPermission("governance:write") ? (
                <>
                  <Button variant="outline" size="sm" onClick={() => setEditing(selected)}>
                    Edit rule
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setDeleting(selected)}>
                    Delete rule
                  </Button>
                </>
              ) : null}
            </div>
          </>
        ) : null}
      </DetailDrawer>

      {editing ? (
        <EditRuleDialog
          rule={editing}
          onClose={() => setEditing(null)}
          onSaved={(saved) => {
            setEditing(null)
            // The drawer shows the rule as it now reads; the list catches up.
            setSelected(saved)
            setIncidentsKey((k) => k + 1)
            state.reload()
          }}
        />
      ) : null}
      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => (open ? undefined : setDeleting(null))}
        title="Delete quality rule"
        description={`Delete ${deleting?.name ?? "this rule"}?`}
        impact={`The rule and the results recorded for it are removed, and its open incidents are resolved as "rule deleted".`}
        confirmLabel="Delete rule"
        confirming={removeRule.status === "pending"}
        destructive
        onConfirm={() => void handleDelete()}
      />
    </div>
  )
}

