"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
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
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import type { Severity } from "@/lib/status"
import { governanceService } from "@/services"
import type { QualityRule } from "@/services/contracts/governance"
import {
  SEVERITY_OPTIONS,
  getDataQualityColumns,
} from "./data-quality-columns"

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
                // A rule nobody has evaluated has no verdict to badge —
                // render that honestly instead of guessing a `CheckStatus`.
                <span className="text-muted-foreground">Not evaluated</span>
              ) : (
                <CheckBadge status={selected.lastStatus} />
              )}
              <SeverityBadge severity={selected.severity} />
            </div>
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
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              render={
                <Link href={`/data?q=${encodeURIComponent(selected.asset)}`} />
              }
            >
              Inspect in Data Explorer
            </Button>
          </>
        ) : null}
      </DetailDrawer>

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
            placeholder=">= 99%"
          />
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

