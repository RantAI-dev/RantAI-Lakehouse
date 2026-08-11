"use client"

import { useMemo, useState } from "react"
import { useRouter, useSearchParams } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import {
  FilterSelect,
  FilterToolbar,
  SearchField,
} from "@/components/patterns/filter-toolbar"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import {
  ClassificationBadge,
  HealthBadge,
  TierBadge,
} from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes, formatCompactNumber } from "@/lib/format"
import {
  CLASSIFICATION_LABEL,
  DATA_LAYER_LABEL,
  STORAGE_TIER_LABEL,
  type Classification,
  type DataLayer,
  type StorageTier,
} from "@/lib/status"
import { assetService } from "@/services"
import {
  ASSET_TYPE_LABEL,
  type Asset,
  type AssetType,
} from "@/services/contracts/assets"

const columns: ColumnDef<Asset>[] = [
  {
    key: "name",
    header: "Asset",
    render: (r) => (
      <div>
        <p className="font-medium">{r.name}</p>
        <p className="text-xs text-muted-foreground">
          <span className="font-mono">{r.namespace}</span> · {r.owner}
        </p>
      </div>
    ),
  },
  {
    key: "type",
    header: "Type",
    render: (r) => ASSET_TYPE_LABEL[r.type],
  },
  {
    key: "tier",
    header: "Tier",
    render: (r) => <TierBadge tier={r.tier} />,
  },
  {
    key: "layer",
    header: "Layer",
    render: (r) => DATA_LAYER_LABEL[r.layer],
  },
  {
    key: "class",
    header: "Class",
    render: (r) => <ClassificationBadge classification={r.classification} />,
  },
  {
    key: "fresh",
    header: "Freshness",
    render: (r) => <FreshnessIndicator lagSeconds={r.freshnessLagSeconds} />,
  },
  {
    key: "size",
    header: "Size",
    render: (r) => formatBytes(r.sizeBytes),
  },
  {
    key: "rows",
    header: "Rows",
    render: (r) => formatCompactNumber(r.rows),
  },
  {
    key: "health",
    header: "Health",
    render: (r) => <HealthBadge health={r.health} />,
  },
]

/** Data Explorer — browse assets by tier (primary) and logical layer (secondary). */
export function DataExplorerPage() {
  const router = useRouter()
  const params = useSearchParams()
  const [search, setSearch] = useState(params.get("q") ?? "")
  const tier = (params.get("tier") as StorageTier | "all" | null) ?? "all"
  const layer = (params.get("layer") as DataLayer | "all" | null) ?? "all"
  const type = (params.get("type") as AssetType | "all" | null) ?? "all"
  const classification =
    (params.get("classification") as Classification | "all" | null) ?? "all"

  const filter = useMemo(
    () => ({ search, tier, layer, type, classification }),
    [search, tier, layer, type, classification]
  )

  const state = useService(
    (s) => assetService.listAssets(filter, s),
    [filter.search, filter.tier, filter.layer, filter.type, filter.classification]
  )

  function setParam(key: string, value: string) {
    const next = new URLSearchParams(params.toString())
    if (value === "all" || value === "") next.delete(key)
    else next.set(key, value)
    router.replace(`/data?${next.toString()}`)
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Data Explorer"
        description="Browse tables, views, open tables, streaming views, and vector datasets across hot, warm, cold, and AI tiers."
      />
      <FilterToolbar>
        <SearchField
          value={search}
          onChange={(v) => {
            setSearch(v)
            setParam("q", v)
          }}
          placeholder="Search assets..."
        />
        <FilterSelect
          ariaLabel="Filter by storage tier"
          allLabel="All tiers"
          value={tier}
          onChange={(v) => setParam("tier", v)}
          options={Object.entries(STORAGE_TIER_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by logical layer"
          allLabel="All layers"
          value={layer}
          onChange={(v) => setParam("layer", v)}
          options={Object.entries(DATA_LAYER_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by asset type"
          allLabel="All types"
          value={type}
          onChange={(v) => setParam("type", v)}
          options={Object.entries(ASSET_TYPE_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
        <FilterSelect
          ariaLabel="Filter by classification"
          allLabel="All classifications"
          value={classification}
          onChange={(v) => setParam("classification", v)}
          options={Object.entries(CLASSIFICATION_LABEL).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </FilterToolbar>
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <DataTable
          columns={columns}
          rows={state.data}
          rowKey={(r) => r.id}
          onRowClick={(r) => router.push(`/data/assets/${r.id}`)}
        />
      ) : null}
    </div>
  )
}
