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
import {
  ClassificationBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
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

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

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
  const [createOpen, setCreateOpen] = React.useState(false)
  const [asset, setAsset] = React.useState("")
  const [column, setColumn] = React.useState("")
  const [formClassification, setFormClassification] =
    React.useState<Classification>("internal")
  const [maskingRule, setMaskingRule] = React.useState("")
  const create = useServiceAction(
    withNotify(
      {
        success: "Classification rule created",
        error: "Failed to create classification rule",
      },
      (
        signal,
        input: Parameters<typeof governanceService.createClassificationRule>[0]
      ) => governanceService.createClassificationRule(input, signal)
    )
  )

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

  function resetForm() {
    setAsset("")
    setColumn("")
    setFormClassification("internal")
    setMaskingRule("")
  }

  async function handleCreate() {
    const result = await create.run({
      asset: asset.trim(),
      column: column.trim() || undefined,
      classification: formClassification,
      maskingRule: maskingRule.trim() || undefined,
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
        title="Classification & Masking"
        description="Classification taxonomy, confidence, and column masking rules."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <PlusIcon data-icon="inline-start" />
            Add Rule
          </Button>
        }
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
      <CreateSheet
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) resetForm()
        }}
        title="Add Rule"
        description="Classify an asset or column and optionally apply masking."
        canSubmit={Boolean(asset.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="cr-asset">Asset</Label>
          <Input id="cr-asset" value={asset} onChange={(e) => setAsset(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cr-column">Column (optional)</Label>
          <Input
            id="cr-column"
            value={column}
            onChange={(e) => setColumn(e.target.value)}
            placeholder="email"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cr-class">Classification</Label>
          <select
            id="cr-class"
            className={selectClassName}
            value={formClassification}
            onChange={(e) =>
              setFormClassification(e.target.value as Classification)
            }
          >
            {CLASSIFICATION_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cr-mask">Masking rule (optional)</Label>
          <Input
            id="cr-mask"
            value={maskingRule}
            onChange={(e) => setMaskingRule(e.target.value)}
            placeholder="hash_last4"
          />
        </div>
      </CreateSheet>
    </div>
  )
}
