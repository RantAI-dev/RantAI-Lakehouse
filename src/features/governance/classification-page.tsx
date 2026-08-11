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
import { formatPercent } from "@/lib/format"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import { governanceService } from "@/services"
import type { ClassificationRule } from "@/services/contracts/governance"

type ReviewStatus = ClassificationRule["reviewStatus"]

const REVIEW_META: Record<
  ReviewStatus,
  { tone: "success" | "info" | "warning"; label: string }
> = {
  reviewed: { tone: "success", label: "Reviewed" },
  auto: { tone: "info", label: "Auto" },
  "needs-review": { tone: "warning", label: "Needs review" },
}

const REVIEW_OPTIONS = (Object.keys(REVIEW_META) as ReviewStatus[]).map((s) => ({
  value: s,
  label: REVIEW_META[s].label,
}))

const CLASSIFICATION_OPTIONS = (
  Object.keys(CLASSIFICATION_LABEL) as Classification[]
).map((c) => ({ value: c, label: CLASSIFICATION_LABEL[c] }))

const columns: ColumnDef<ClassificationRule>[] = [
  { key: "asset", header: "Asset", render: (r) => r.asset },
  { key: "col", header: "Column", className: "font-mono text-xs", render: (r) => r.column ?? "—" },
  { key: "class", header: "Classification", render: (r) => <ClassificationBadge classification={r.classification} /> },
  { key: "conf", header: "Confidence", render: (r) => formatPercent(r.confidence) },
  { key: "review", header: "Review", render: (r) => (
    <Pill tone={REVIEW_META[r.reviewStatus].tone}>{REVIEW_META[r.reviewStatus].label}</Pill>
  )},
  { key: "mask", header: "Masking", render: (r) => r.maskingRule ?? "—" },
]

export function ClassificationPage() {
  const state = useService((s) => governanceService.listClassifications(s), [])
  const [search, setSearch] = React.useState("")
  const [classification, setClassification] = React.useState("all")
  const [review, setReview] = React.useState("all")

  const filtered = React.useMemo(() => {
    const q = search.trim().toLowerCase()
    return (state.data ?? []).filter((r) => {
      if (classification !== "all" && r.classification !== classification)
        return false
      if (review !== "all" && r.reviewStatus !== review) return false
      if (!q) return true
      return [r.asset, r.column ?? ""].some((v) => v.toLowerCase().includes(q))
    })
  }, [state.data, search, classification, review])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Classification & Masking"
        description="Classification taxonomy, confidence, and column masking rules."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="Search asset or column..."
        />
        <FilterSelect
          value={classification}
          onChange={setClassification}
          options={CLASSIFICATION_OPTIONS}
          allLabel="All classifications"
          ariaLabel="Filter by classification"
        />
        <FilterSelect
          value={review}
          onChange={setReview}
          options={REVIEW_OPTIONS}
          allLabel="All review statuses"
          ariaLabel="Filter by review status"
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
