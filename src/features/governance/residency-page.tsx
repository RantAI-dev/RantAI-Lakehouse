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
import {
  ClassificationBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import { governanceService } from "@/services"
import type { ResidencyRule } from "@/services/contracts/governance"

const CLASSIFICATION_OPTIONS = (
  Object.keys(CLASSIFICATION_LABEL) as Classification[]
).map((c) => ({ value: c, label: CLASSIFICATION_LABEL[c] }))

const columns: ColumnDef<ResidencyRule>[] = [
  { key: "tenant", header: "Tenant", render: (r) => r.tenant },
  { key: "class", header: "Classification", render: (r) => <ClassificationBadge classification={r.classification} /> },
  { key: "sites", header: "Approved sites", render: (r) => (
    <div className="flex flex-wrap gap-1">
      {r.approvedSites.map((s) => (
        <Pill key={s} tone="neutral">{s}</Pill>
      ))}
    </div>
  )},
  { key: "cross", header: "Cross-site", render: (r) =>
    r.crossSiteAllowed ? (
      <Pill tone="neutral">Cross-site allowed</Pill>
    ) : (
      <Pill tone="warning">Single site</Pill>
    ),
  },
  { key: "out", header: "Allowed output", render: (r) => r.allowedOutput },
  { key: "viol", header: "Violations 7d", render: (r) =>
    r.violations7d > 0 ? (
      <span className="font-medium text-destructive">{r.violations7d}</span>
    ) : (
      r.violations7d
    ),
  },
]

export function ResidencyPage() {
  const state = useService((s) => governanceService.listResidency(s), [])
  const [search, setSearch] = React.useState("")
  const [classification, setClassification] = React.useState("all")

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (classification !== "all" && r.classification !== classification)
        return false
      if (!q) return true
      return r.tenant.toLowerCase().includes(q)
    })
  }, [state.data, search, classification])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Residency"
        description="Approved sites, classification rules, and boundary-crossing constraints."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search tenant..."
        />
        <FilterSelect
          value={classification}
          onChange={setClassification}
          options={CLASSIFICATION_OPTIONS}
          allLabel="All classifications"
          ariaLabel="Filter by classification"
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
