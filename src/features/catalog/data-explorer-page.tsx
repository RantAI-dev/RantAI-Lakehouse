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
import { TierBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatBytes } from "@/lib/format"
import {
  DATA_LAYER_LABEL,
  STORAGE_TIER_LABEL,
  type DataLayer,
  type StorageTier,
} from "@/lib/status"
import { cn } from "@/lib/utils"
import { assetService } from "@/services"
import {
  ASSET_TYPE_LABEL,
  type Asset,
  type AssetType,
} from "@/services/contracts/assets"

const LAYERS: (DataLayer | "all")[] = [
  "all",
  "raw",
  "bronze",
  "silver",
  "gold",
  "semantic",
]

const columns: ColumnDef<Asset>[] = [
  {
    key: "name",
    header: "Name",
    render: (r) => (
      <div className="min-w-0 py-0.5">
        <p className="truncate font-medium tracking-tight text-foreground">
          {r.name}
        </p>
        <p className="mt-0.5 truncate text-xs text-muted-foreground">
          <span className="font-mono">{r.namespace}</span>
          <span className="mx-1.5 text-border">·</span>
          {ASSET_TYPE_LABEL[r.type]}
          <span className="mx-1.5 text-border">·</span>
          {DATA_LAYER_LABEL[r.layer]}
        </p>
      </div>
    ),
  },
  {
    key: "tier",
    header: "Tier",
    className: "w-24",
    render: (r) => <TierBadge tier={r.tier} />,
  },
  {
    key: "fresh",
    header: "Freshness",
    className: "w-36",
    render: (r) => <FreshnessIndicator lagSeconds={r.freshnessLagSeconds} />,
  },
  {
    key: "size",
    header: "Size",
    className: "w-24 text-right",
    render: (r) => (
      <span className="tabular-nums text-muted-foreground">
        {formatBytes(r.sizeBytes)}
      </span>
    ),
  },
]

/** Data Explorer — browse assets by data layer (primary) and storage tier (secondary). */
export function DataExplorerPage() {
  const router = useRouter()
  const params = useSearchParams()
  const [search, setSearch] = useState(params.get("q") ?? "")
  const tier = (params.get("tier") as StorageTier | "all" | null) ?? "all"
  const layer = (params.get("layer") as DataLayer | "all" | null) ?? "all"
  const type = (params.get("type") as AssetType | "all" | null) ?? "all"

  const filter = useMemo(
    () => ({
      search,
      tier,
      layer,
      type,
      classification: "all" as const,
    }),
    [search, tier, layer, type]
  )

  const state = useService(
    (s) => assetService.listAssets(filter, s),
    [filter.search, filter.tier, filter.layer, filter.type]
  )

  function setParam(key: string, value: string) {
    const next = new URLSearchParams(params.toString())
    if (value === "all" || value === "") next.delete(key)
    else next.set(key, value)
    // Drop legacy classification filter from the URL when browsing.
    next.delete("classification")
    router.replace(`/data?${next.toString()}`)
  }

  const count = state.status === "success" ? state.data.length : null

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Data Explorer"
        description="Browse governed assets by data layer (Raw → Gold)."
      />

      <div className="flex flex-col gap-3">
        <div
          className="flex flex-wrap gap-1 rounded-lg border border-border bg-muted/40 p-1"
          role="tablist"
          aria-label="Data layer"
        >
          {LAYERS.map((t) => {
            const selected = layer === t
            const label = t === "all" ? "All" : DATA_LAYER_LABEL[t]
            return (
              <button
                key={t}
                type="button"
                role="tab"
                aria-selected={selected}
                onClick={() => setParam("layer", t)}
                className={cn(
                  "rounded-md px-3 py-1.5 text-sm transition-colors",
                  selected
                    ? "bg-background font-medium text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground"
                )}
              >
                {label}
              </button>
            )
          })}
        </div>

        <FilterToolbar className="justify-between gap-3">
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <SearchField
              value={search}
              onChange={(v) => {
                setSearch(v)
                setParam("q", v)
              }}
              placeholder="Search by name or namespace…"
              className="max-w-sm"
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
              className="min-w-28"
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
              className="min-w-28"
            />
          </div>
          {count != null ? (
            <p className="shrink-0 text-xs tabular-nums text-muted-foreground">
              {count} {count === 1 ? "asset" : "assets"}
            </p>
          ) : null}
        </FilterToolbar>
      </div>

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
          emptyMessage="No assets match these filters."
        />
      ) : null}
    </div>
  )
}
