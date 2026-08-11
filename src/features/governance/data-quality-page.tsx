"use client"

import * as React from "react"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { CheckBadge, SeverityBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import {
  CHECK_STATUS_LABEL,
  SEVERITY_LABEL,
  type CheckStatus,
  type Severity,
} from "@/lib/status"
import { governanceService } from "@/services"
import type { QualityRule } from "@/services/contracts/governance"

const CHECK_OPTIONS = (Object.keys(CHECK_STATUS_LABEL) as CheckStatus[]).map(
  (s) => ({ value: s, label: CHECK_STATUS_LABEL[s] })
)

const SEVERITY_OPTIONS = (Object.keys(SEVERITY_LABEL) as Severity[]).map(
  (s) => ({ value: s, label: SEVERITY_LABEL[s] })
)

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

const columns: ColumnDef<QualityRule>[] = [
  { key: "name", header: "Rule", render: (r) => r.name },
  { key: "asset", header: "Asset", render: (r) => r.asset },
  { key: "dim", header: "Dimension", render: (r) => r.dimension },
  { key: "thr", header: "Threshold", render: (r) => r.threshold },
  { key: "sev", header: "Severity", render: (r) => <SeverityBadge severity={r.severity} /> },
  { key: "status", header: "Last status", render: (r) => <CheckBadge status={r.lastStatus} /> },
  { key: "last", header: "Last run", render: (r) => formatRelativeTime(r.lastRunAt) },
]

export function DataQualityPage() {
  const state = useService((s) => governanceService.listQuality(s), [])
  const [search, setSearch] = React.useState("")
  const [dimension, setDimension] = React.useState("all")
  const [lastStatus, setLastStatus] = React.useState("all")
  const [createOpen, setCreateOpen] = React.useState(false)
  const [name, setName] = React.useState("")
  const [asset, setAsset] = React.useState("")
  const [formDimension, setFormDimension] = React.useState("")
  const [threshold, setThreshold] = React.useState("")
  const [severity, setSeverity] = React.useState<Severity>("medium")
  const create = useServiceAction(
    (signal, input: Parameters<typeof governanceService.createQualityRule>[0]) =>
      governanceService.createQualityRule(input, signal)
  )

  const dimensionOptions = React.useMemo(() => {
    const present = new Set(state.data?.map((r) => r.dimension) ?? [])
    return [...present].map((d) => ({ value: d, label: d }))
  }, [state.data])

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (dimension !== "all" && r.dimension !== dimension) return false
      if (lastStatus !== "all" && r.lastStatus !== lastStatus) return false
      if (!q) return true
      return [r.name, r.asset].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, dimension, lastStatus])

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
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search rule or asset..."
        />
        <FilterSelect
          value={dimension}
          onChange={setDimension}
          options={dimensionOptions}
          allLabel="All dimensions"
          ariaLabel="Filter by dimension"
        />
        <FilterSelect
          value={lastStatus}
          onChange={setLastStatus}
          options={CHECK_OPTIONS}
          allLabel="All statuses"
          ariaLabel="Filter by last status"
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <DataTable columns={columns} rows={filtered} rowKey={(r) => r.id} />
      ) : null}
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
          <Input id="qr-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="qr-asset">Asset</Label>
          <Input id="qr-asset" value={asset} onChange={(e) => setAsset(e.target.value)} />
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
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </div>
      </CreateSheet>
    </div>
  )
}
