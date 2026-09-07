"use client"

import * as React from "react"
import { useCallback, useEffect, useMemo } from "react"
import { useRouter, useSearchParams } from "next/navigation"

import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DataTableSkeleton } from "@/components/data-table/data-table-skeleton"
import {
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/components/ui/context-menu"
import { ErrorState } from "@/components/patterns/page-states"
import { useDataTable } from "@/hooks/use-data-table"
import { useInfiniteTableQuery } from "@/hooks/use-infinite-table-query"
import {
  toInfiniteQueryParams,
  useTableUrlState,
} from "@/hooks/use-table-url-state"
import { useWindowWidth } from "@/hooks/use-window-width"
import { assetService } from "@/services"
import type { Asset } from "@/services/contracts/assets"
import { toServiceError } from "@/services/errors"
import { getAssetActions } from "./data-explorer-actions"
import { getDataExplorerColumns } from "./data-explorer-columns"

/**
 * Legacy `?q/layer/tier/type` params, mapped onto the names the advanced
 * table uses. Links to this page predate the table rewrite (the sidebar and
 * several dashboard cards still build them), so they are translated once on
 * arrival instead of being silently dropped.
 */
function legacyParamsToTableState(
  params: URLSearchParams
): URLSearchParams | null {
  const search = params.get("q")
  const facets = (["layer", "tier", "type"] as const).flatMap((id) => {
    const value = params.get(id)
    return value && value !== "all"
      ? [{ id, value: [value], variant: "multiSelect", operator: "inArray" }]
      : []
  })
  if (!search && facets.length === 0) return null

  const next = new URLSearchParams(params)
  for (const key of ["q", "layer", "tier", "type", "classification"]) {
    next.delete(key)
  }
  if (search) next.set("search", search)
  if (facets.length > 0) next.set("filters", JSON.stringify(facets))
  return next
}

/** Data Explorer — browse governed assets by layer, tier, type, and freshness. */
export function DataExplorerPage() {
  const router = useRouter()
  const searchParams = useSearchParams()
  const tableUrlState = useTableUrlState()

  // Rewrite legacy links before the table reads the URL. `replace` keeps the
  // old address out of history, so Back still leaves the page.
  useEffect(() => {
    const migrated = legacyParamsToTableState(
      new URLSearchParams(searchParams.toString())
    )
    if (migrated) router.replace(`/data?${migrated.toString()}`)
  }, [router, searchParams])

  // Only the query-shaping state belongs in the key. Column order, widths
  // and visibility also live in the URL but must not refetch when changed.
  const assetsQueryKey = useMemo(
    () => [
      "catalog-assets",
      tableUrlState.search,
      tableUrlState.sort,
      tableUrlState.filters,
      tableUrlState.joinOperator,
      tableUrlState.groupBy,
    ],
    [
      tableUrlState.search,
      tableUrlState.sort,
      tableUrlState.filters,
      tableUrlState.joinOperator,
      tableUrlState.groupBy,
    ]
  )

  const {
    rows,
    totalItems,
    groupSummaries,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isLoading,
    error,
    refetch,
  } = useInfiniteTableQuery<Asset>({
    queryKey: assetsQueryKey,
    queryFn: (page, signal) =>
      assetService.listAssetsPage(
        toInfiniteQueryParams(tableUrlState, page),
        signal
      ),
  })

  const openAsset = useCallback(
    (asset: Asset) => router.push(`/data/assets/${asset.id}`),
    [router]
  )

  // All eight columns need ~875px, and the content area is much narrower
  // than the window once the sidebar is open — at a 900px window it is
  // only ~604px. Rather than let the page scroll sideways (which drags
  // the sidebar and navbar with it and clips the last columns outright),
  // leave out the columns that earn their width least as space runs out.
  //
  // Order of sacrifice, least useful first: Size (nice to know) →
  // Freshness (also shown on the detail page) → Type (largely implied by
  // Layer). Name, Namespace, Tier and the actions menu always stay.
  //
  // These are removed from the column list rather than toggled through
  // `columnVisibility`, because that state is persisted per user by
  // `useTableLayout` — driving it from the window size would overwrite
  // someone's saved choices every time they resized.
  const width = useWindowWidth()
  const hiddenByWidth = useMemo(() => {
    // Before the first measurement, assume there is room: showing the full
    // table and then trimming reads better than the reverse.
    if (width === null) return []
    const hidden: string[] = []
    if (width < 1280) hidden.push("sizeBytes")
    if (width < 1100) hidden.push("freshnessLagSeconds")
    if (width < 950) hidden.push("type")
    // Below this the sidebar collapses to a sheet, so the content area is
    // the full window — but it is still too tight for five columns. Layer
    // goes last because Tier carries the more operational signal.
    if (width < 860) hidden.push("layer")
    return hidden
  }, [width])

  // Memoised: a new array each render would make TanStack Table rebuild
  // its column model, losing in-flight state like a column being dragged.
  const columns = useMemo(() => {
    const all = getDataExplorerColumns({ onOpen: openAsset })
    if (hiddenByWidth.length === 0) return all
    return all.filter((column) => !hiddenByWidth.includes(column.id ?? ""))
  }, [openAsset, hiddenByWidth])

  const { table } = useDataTable({
    data: rows,
    columns,
    // -1 because the server never counts the pages for an infinite table;
    // `hasNextPage` is what ends the scroll.
    pageCount: -1,
    rowCount: totalItems,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    // The URL is the source of truth for the query, so navigation has to
    // reach the server component too — `shallow: false` is what makes a
    // filter change actually refetch.
    shallow: false,
    getRowId: (row) => row.id,
    initialState: {
      // `id` stays available in the column menu, filters and the row
      // context menu; it is just noise as a column, since Name and
      // Namespace together already identify the asset.
      columnVisibility: { id: false },
      // Keeps the ⋮ button reachable while the rest of the table scrolls
      // sideways — the actions are useless if you have to scroll to them.
      columnPinning: { right: ["actions"] },
    },
  })

  const infiniteState = useMemo(
    () => ({
      onLoadMore: () => {
        void fetchNextPage()
      },
      hasNextPage: Boolean(hasNextPage),
      isFetchingNextPage,
      totalItems,
      loadedCount: rows.length,
    }),
    [fetchNextPage, hasNextPage, isFetchingNextPage, rows.length, totalItems]
  )

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Data Explorer"
        description="Browse governed assets by data layer (Raw → Gold)."
      />

      {error ? (
        // React Query widens whatever was thrown to `Error`; `ErrorState`
        // branches on `ServiceError.code` (a 403 renders as a permission
        // notice, not a generic failure), so it is narrowed back here.
        <ErrorState
          error={toServiceError(error)}
          onRetry={() => void refetch()}
        />
      ) : isLoading ? (
        <DataTableSkeleton
          columnCount={columns.length}
          rowCount={8}
          filterCount={2}
        />
      ) : (
        <DataTable
          table={table}
          groupSummaries={groupSummaries}
          infinite={infiniteState}
          onRowClick={openAsset}
          // Same list the ⋮ button renders, from `getAssetActions` — the
          // two menus cannot drift apart because only the markup differs.
          // Right-click stays as the shortcut for people who know it;
          // the pinned column is what makes the actions discoverable.
          renderRowContextMenu={(row) => (
            <>
              {getAssetActions(row, { onOpen: openAsset }).map((action) => (
                <React.Fragment key={action.id}>
                  {action.separatorBefore ? <ContextMenuSeparator /> : null}
                  <ContextMenuItem onSelect={action.onSelect}>
                    <action.icon />
                    {action.label}
                  </ContextMenuItem>
                </React.Fragment>
              ))}
            </>
          )}
        >
          <DataTableAdvancedToolbar table={table}>
            <DataTableSearch placeholder="Search assets by name, namespace, or owner…" />
          </DataTableAdvancedToolbar>
        </DataTable>
      )}
    </div>
  )
}
