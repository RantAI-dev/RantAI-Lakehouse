"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { CheckBadge, SeverityBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { CHECK_STATUS_LABEL, type CheckStatus } from "@/lib/status"
import { governanceService } from "@/services"
import type { QualityRule } from "@/services/contracts/governance"

const CHECK_OPTIONS = (Object.keys(CHECK_STATUS_LABEL) as CheckStatus[]).map(
  (s) => ({ value: s, label: CHECK_STATUS_LABEL[s] })
)

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

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Data Quality"
        description="Rules, dimensions, thresholds, and remediation signals."
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
    </div>
  )
}
