"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { CheckBadge, SeverityBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatRelativeTime } from "@/lib/format"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import type { Severity } from "@/lib/status"
import { governanceService } from "@/services"
import type { QualityRule } from "@/services/contracts/governance"
import {
  SEVERITY_OPTIONS,
  getDataQualityColumns,
} from "./data-quality-columns"
import { EditRuleDialog } from "./quality-rule-edit-dialog"

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

export function DataQualityPage() {
  const state = useService((s) => governanceService.listQuality(s), [])
  const [selected, setSelected] = React.useState<QualityRule | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [asset, setAsset] = React.useState("")
  const [formDimension, setFormDimension] = React.useState("")
  const [threshold, setThreshold] = React.useState("")
  const [severity, setSeverity] = React.useState<Severity>("medium")
  const tableUrlState = useTableUrlState()

  const create = useServiceAction(
    withNotify(
      {
        success: "Quality rule created",
        error: "Failed to create quality rule",
      },
      (
        signal,
        input: Parameters<typeof governanceService.createQualityRule>[0]
      ) => governanceService.createQualityRule(input, signal)
    )
  )

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

  function resetForm() {
    setName("")
    setAsset("")
    setFormDimension("")
    setThreshold("")
    setSeverity("medium")
  }

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
    state.reload()
  }

  async function handleDelete() {
    if (!deleting) return
    // `run` resolves to `null` only on failure, which `withNotify` reports.
    const ok = await removeRule.run(deleting.id)
    if (ok === null) return
    setDeleting(null)
    setSelected(null)
    state.reload()
  }

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      asset: asset.trim(),
      dimension: formDimension.trim(),
      threshold: threshold.trim(),
      severity,
    })
    if (result) {
      setCreateOpen(false)
      resetForm()
      state.reload()
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Data Quality"
        description="Rules, dimensions, thresholds, and remediation signals."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Add Quality Rule
          </Button>
        }
      />

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
            <div className="flex flex-wrap gap-2">
              {selected.evaluable ? (
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
                  <Link href={`/data?q=${encodeURIComponent(selected.asset)}`} />
                }
              >
                Inspect in Data Explorer
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
            state.reload()
          }}
        />
      ) : null}
      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => (open ? undefined : setDeleting(null))}
        title="Delete quality rule"
        description={`Delete ${deleting?.name ?? "this rule"}?`}
        impact="The rule and the results recorded for it are removed."
        confirmLabel="Delete rule"
        confirming={removeRule.status === "pending"}
        destructive
        onConfirm={() => void handleDelete()}
      />
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Add Quality Rule"
        description="Define a quality check with dimension, threshold, and severity."
        canSubmit={Boolean(
          name.trim() && asset.trim() && formDimension.trim() && threshold.trim()
        )}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="qr-name">Name</Label>
          <Input
            id="qr-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="qr-asset">Asset</Label>
          <Input
            id="qr-asset"
            value={asset}
            onChange={(e) => setAsset(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="qr-dim">Dimension</Label>
          <Input
            id="qr-dim"
            value={formDimension}
            onChange={(e) => setFormDimension(e.target.value)}
            placeholder="completeness"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="qr-threshold">Threshold</Label>
          <Input
            id="qr-threshold"
            value={threshold}
            onChange={(e) => setThreshold(e.target.value)}
            placeholder="email not null >= 99%"
          />
          <p className="text-xs text-muted-foreground">
            To be runnable, write it as: <span className="font-mono">rows &gt;= 1000</span>,{" "}
            <span className="font-mono">column not null &gt;= 95%</span>,{" "}
            <span className="font-mono">column unique</span> or{" "}
            <span className="font-mono">column between 0 and 100</span>. Name the asset as
            silver.table, serving.table or bronze.table.
          </p>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="qr-severity">Severity</Label>
          <select
            id="qr-severity"
            className={selectClassName}
            value={severity}
            onChange={(e) => setSeverity(e.target.value as Severity)}
          >
            {SEVERITY_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
      </CreateSheet>
    </div>
  )
}

