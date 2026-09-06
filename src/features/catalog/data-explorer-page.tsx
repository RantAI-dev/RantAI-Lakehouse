"use client"

import { useEffect, useMemo } from "react"
import { useRouter, useSearchParams } from "next/navigation"
import { ExternalLink, Copy, Boxes } from "lucide-react"

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
import { notifyError, notifySuccess } from "@/lib/notify"
import { assetService } from "@/services"
import type { Asset } from "@/services/contracts/assets"
import { toServiceError } from "@/services/errors"
import { dataExplorerColumns } from "./data-explorer-columns"

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

  const { table } = useDataTable({
    data: rows,
    columns: dataExplorerColumns,
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
      // `id` stays available in the column menu and filters; it is just
      // noise beside `name`, which already shows the namespace.
      columnVisibility: { id: false },
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

  async function copyToClipboard(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value)
      notifySuccess(`${label} copied`)
    } catch (err) {
      notifyError(`Failed to copy ${label.toLowerCase()}`, err)
    }
  }

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
          columnCount={dataExplorerColumns.length}
          rowCount={8}
          filterCount={2}
        />
      ) : (
        <DataTable
          table={table}
          groupSummaries={groupSummaries}
          infinite={infiniteState}
          onRowClick={(row) => router.push(`/data/assets/${row.id}`)}
          renderRowContextMenu={(row) => (
            <>
              <ContextMenuItem
                onSelect={() => router.push(`/data/assets/${row.id}`)}
              >
                <ExternalLink />
                Open asset
              </ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem
                onSelect={() => void copyToClipboard(row.id, "Asset ID")}
              >
                <Copy />
                Copy asset ID
              </ContextMenuItem>
              <ContextMenuItem
                onSelect={() => void copyToClipboard(row.namespace, "Namespace")}
              >
                <Boxes />
                Copy namespace
              </ContextMenuItem>
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
