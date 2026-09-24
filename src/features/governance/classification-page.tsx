"use client"

import * as React from "react"
import Link from "next/link"
import { PlusIcon } from "lucide-react"
import { CreateSheet } from "@/components/patterns/create-sheet"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  ClassificationBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatPercent } from "@/lib/format"
import { useService, useServiceAction } from "@/hooks/use-service"
import { withNotify } from "@/lib/notify"
import type { Classification } from "@/lib/status"
import { governanceService } from "@/services"
import type { ClassificationRule } from "@/services/contracts/governance"
import {
  CLASSIFICATION_OPTIONS,
  REVIEW_META,
  getClassificationColumns,
} from "./classification-columns"

const selectClassName =
  "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

export function ClassificationPage() {
  const state = useService((s) => governanceService.listClassifications(s), [])
  const [selected, setSelected] = React.useState<ClassificationRule | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [asset, setAsset] = React.useState("")
  const [column, setColumn] = React.useState("")
  const [formClassification, setFormClassification] =
    React.useState<Classification>("internal")
  const [maskingRule, setMaskingRule] = React.useState("")
  const tableUrlState = useTableUrlState()

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

  const columns = React.useMemo(
    () => getClassificationColumns({ onSelect: setSelected }),
    []
  )

  const filteredData = React.useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (r) => r.asset,
        (r) => r.column ?? "",
        (r) => r.maskingRule ?? "",
        (r) => r.classification,
        (r) => r.reviewStatus,
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
    persistKey: "/governance/classification",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

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

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch placeholder="Search asset or column…" />
          </DataTableAdvancedToolbar>
          <DataTable table={table} onRowClick={setSelected} />
        </div>
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={
          selected
            ? selected.column
              ? `${selected.asset}.${selected.column}`
              : selected.asset
            : ""
        }
      >
        {selected ? (
          <>
            <div className="flex items-center gap-2">
              <ClassificationBadge classification={selected.classification} />
              <Pill tone={REVIEW_META[selected.reviewStatus].tone}>
                {REVIEW_META[selected.reviewStatus].label}
              </Pill>
            </div>
            <MetadataList
              items={[
                { label: "Asset", value: selected.asset },
                { label: "Column", value: selected.column ?? "—" },
                {
                  label: "Confidence",
                  value: formatPercent(selected.confidence),
                },
                {
                  label: "Masking rule",
                  value: selected.maskingRule ?? "None",
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
        title="Add Rule"
        description="Classify an asset or column and optionally apply masking."
        canSubmit={Boolean(asset.trim())}
        submitting={create.status === "pending"}
        onSubmit={handleCreate}
      >
        <div className="space-y-1.5">
          <Label htmlFor="cr-asset">Asset</Label>
          <Input
            id="cr-asset"
            value={asset}
            onChange={(e) => setAsset(e.target.value)}
          />
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
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
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

