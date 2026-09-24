"use client"

import { useMemo, useState } from "react"
import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { assetService } from "@/services"
import type { Asset, CatalogNamespace } from "@/services/contracts/assets"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DataTableSkeleton } from "@/components/data-table/data-table-skeleton"
import { useDataTable } from "@/hooks/use-data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { filterParam, namespaceAssetsHref } from "@/lib/table-filter-link"
import { formatRelativeTime } from "@/lib/format"
import { TierBadge } from "@/components/patterns/status-badge"
import { getCatalogNamespaceColumns } from "./catalog-namespace-columns"

/** How many assets the namespace drawer previews before linking out. */
const PREVIEW_SIZE = 5

/** Unified catalog namespaces with ownership and residency metadata. */
export function CatalogPage() {
  const state = useService((s) => assetService.listNamespaces(s), [])
  const [selected, setSelected] = useState<CatalogNamespace | null>(null)
  const tableUrlState = useTableUrlState()

  const columns = useMemo(
    () => getCatalogNamespaceColumns({ onSelect: setSelected }),
    []
  )

  // The first few assets of the namespace being inspected. A namespace on
  // its own says very little — "8 assets" is a number, not an answer — so
  // the drawer shows what is actually in there before the user commits to
  // navigating away. Nothing is fetched until a row is opened.
  const selectedName = selected?.name ?? null
  const preview = useService<Asset[]>(
    async (signal) => {
      if (!selectedName) return []
      const page = await assetService.listAssetsPage(
        {
          page: 1,
          pageSize: PREVIEW_SIZE,
          filters: filterParam([
            {
              id: "namespace",
              value: selectedName,
              variant: "text",
              operator: "eq",
            },
          ]),
          joinOperator: "and",
          skipListMeta: true,
        },
        signal
      )
      return page.items
    },
    [selectedName]
  )

  const filteredData = useMemo(() => {
    if (state.status !== "success" || !state.data) return []
    return filterDataClientSide(state.data, {
      search: tableUrlState.search,
      searchFields: [
        (ns) => ns.name,
        (ns) => ns.owner,
        (ns) => ns.description,
        (ns) => ns.sourceEngine,
        (ns) => ns.residency,
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
    persistKey: "/catalog",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Catalog"
        description="Namespaces, ownership, residency, and source-engine metadata for governed discovery."
      />

      {/* The table's own skeleton, not the generic row list: this page
          loads into a six-column table, and a placeholder shaped like
          something else makes the layout jump when the data lands. */}
      {state.status === "loading" ? (
        <DataTableSkeleton columnCount={6} rowCount={8} filterCount={2} />
      ) : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" ? (
        <div className="flex flex-col gap-4">
          <DataTableAdvancedToolbar
            table={table}
            onRefresh={state.reload}
            exportName="Catalog namespaces"
          >
            <DataTableSearch placeholder="Search namespaces…" />
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
            <p className="text-sm text-foreground">{selected.description}</p>
            <MetadataList
              items={[
                { label: "Owner", value: selected.owner },
                { label: "Source engine", value: selected.sourceEngine },
                { label: "Residency", value: selected.residency },
                { label: "Assets", value: selected.assetCount },
              ]}
            />
            <div className="flex flex-col gap-2">
              <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Assets
              </p>
              {preview.status === "loading" ? (
                <LoadingSkeleton rows={3} />
              ) : preview.status === "error" ? (
                <p className="text-sm text-muted-foreground">
                  Could not load this namespace&rsquo;s assets.
                </p>
              ) : preview.data.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No assets in this namespace yet.
                </p>
              ) : (
                <ul className="divide-y divide-border">
                  {preview.data.map((asset) => (
                    <li key={asset.id}>
                      <Link
                        href={`/data/assets/${asset.id}`}
                        className="-mx-2 flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-muted/40"
                      >
                        <span className="min-w-0 flex-1 truncate">
                          {asset.name}
                        </span>
                        <TierBadge tier={asset.tier} />
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {formatRelativeTime(asset.lastUpdated)}
                        </span>
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            {/* Filters Data Explorer's namespace column rather than passing
                the name as a search term: free text also matches names,
                descriptions and owners, so a namespace like `lake.bronze`
                used to return rows from other namespaces that merely
                mention it. */}
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              render={<Link href={namespaceAssetsHref(selected.name)} />}
            >
              {selected.assetCount > PREVIEW_SIZE
                ? `Browse all ${selected.assetCount} assets`
                : "Browse in Data Explorer"}
            </Button>
          </>
        ) : null}
      </DetailDrawer>
    </div>
  )
}
