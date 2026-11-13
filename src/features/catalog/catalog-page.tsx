"use client"

import * as React from "react"
import { useCallback, useEffect, useMemo } from "react"
import Link from "next/link"
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
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { useInfiniteTableQuery } from "@/hooks/use-infinite-table-query"
import {
  toInfiniteQueryParams,
  useTableUrlState,
} from "@/hooks/use-table-url-state"
import { useService } from "@/hooks/use-service"
import { useWindowWidth } from "@/hooks/use-window-width"
import { assetService } from "@/services"
import type { Asset } from "@/services/contracts/assets"
import { toServiceError } from "@/services/errors"
import {
  activeFilterValues,
  filterParam,
  removeFilter,
  setFilter,
  toggleFilterValue,
} from "@/lib/table-filter-link"
import { DATA_LAYER_LABEL, type DataLayer } from "@/lib/status"
import { cn } from "@/lib/utils"
import { NamespaceFilterRow, NamespaceSummary, QuickFilterRow } from "./catalog-namespaces"
import { getAssetActions } from "./data-explorer-actions"
import { DroppedTablesNote } from "./dropped-tables"
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
      ? [
          {
            id,
            value: [value],
            variant: "multiSelect" as const,
            operator: "inArray" as const,
          },
        ]
      : []
  })
  if (!search && facets.length === 0) return null

  const next = new URLSearchParams(params)
  for (const key of ["q", "layer", "tier", "type", "classification"]) {
    next.delete(key)
  }
  if (search) next.set("search", search)
  // `filterParam`, not raw JSON: it fills in the `filterId` the filter
  // schema requires, without which the chips never appear.
  if (facets.length > 0) next.set("filters", filterParam(facets))
  return next
}

/**
 * The layers worth a one-click filter. `semantic` is left out on purpose:
 * it is rare, and every layer remains available through the toolbar's own
 * Layer filter — these are shortcuts, not the whole set.
 */
const QUICK_LAYERS: DataLayer[] = ["raw", "bronze", "silver", "gold"]

/** Column labels, for telling the user which ones the window is too narrow for. */
const WIDTH_HIDDEN_LABEL: Record<string, string> = {
  tags: "Tags",
  owner: "Owner",
  sizeBytes: "Size",
  freshnessLagSeconds: "Freshness",
  type: "Type",
  layer: "Layer",
}

/**
 * The Catalog (`WEB-3` S2-D1): every governed asset in one list, with the
 * search, filters and row actions the Data Explorer had and the namespace
 * grouping the old Catalog page had.
 *
 * The two pages paged through the same `assetService.listAssetsPage`; the
 * old Catalog only added a namespace table whose drawer previewed five
 * assets. The namespaces are now a quick filter above the list, and the
 * namespace a user filters to is described above the rows (owner,
 * residency, source engine), so nothing the drawer showed is lost and there
 * is one place to find a table. `/data` redirects here.
 */
export function CatalogPage() {
  const router = useRouter()
  const searchParams = useSearchParams()
  const tableUrlState = useTableUrlState()
  const namespaces = useService((s) => assetService.listNamespaces(s), [])

  // Rewrite legacy links before the table reads the URL. `replace` keeps the
  // old address out of history, so Back still leaves the page.
  useEffect(() => {
    const migrated = legacyParamsToTableState(
      new URLSearchParams(searchParams.toString())
    )
    if (migrated) router.replace(`/catalog?${migrated.toString()}`)
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
    supported,
    unsupportedReason,
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
  // Order of sacrifice, least useful first: Tags and Owner (DATA-11; both
  // are on the detail page) → Size (nice to know) → Freshness (also shown on the detail page) → Type (largely implied by
  // Layer). Name, Namespace, Tier and the actions menu always stay.
  //
  // The columns are hidden, not removed (DATA-24): they stay in the column
  // list, so their filters keep working in the toolbar's filter menu at
  // every width. They used to be dropped from the column list, which took
  // their filters with them — the Owner and Tags filters vanished on a
  // laptop.
  const width = useWindowWidth()
  const hiddenByWidth = useMemo(() => {
    // Before the first measurement, assume there is room: showing the full
    // table and then trimming reads better than the reverse.
    if (width === null) return []
    const hidden: string[] = []
    // Tags and Owner (DATA-11) are the widest new columns and go first.
    //
    // DATA-11 review SHOULD-FIX 7: derived like the figures above, from the
    // window minus the open sidebar (~296px of content lost, 900 -> 604).
    // Without Tags and Owner the table needs at most ~960px (Name at its
    // `xl` cap of 22rem plus the ~605px the other columns took), so it
    // already fills a 1280px window (984px of content). Owner is capped at
    // 10rem (~190px with padding) and Tags at 14rem (~255px), and each is
    // shown only when the content area has ~55px to spare beyond the table
    // plus the column: Owner from 1500px (1204px of content for ~1150px),
    // Tags from 1760px (1464px for ~1405px). At 1280, 1440 and 1536px the
    // table therefore fits with the sidebar open, and Tags goes before Owner.
    // These are estimates from the column caps, not measurements, and they
    // follow the window width, not the table's container.
    if (width < 1760) hidden.push("tags")
    if (width < 1500) hidden.push("owner")
    if (width < 1280) hidden.push("sizeBytes")
    if (width < 1100) hidden.push("freshnessLagSeconds")
    if (width < 950) hidden.push("type")
    // Below this the sidebar collapses to a sheet, so the content area is
    // the full window — but it is still too tight for five columns. Layer
    // goes last because Tier carries the more operational signal.
    if (width < 860) hidden.push("layer")
    return hidden
  }, [width])

  const hiddenLabels = hiddenByWidth.map((id) => WIDTH_HIDDEN_LABEL[id] ?? id)

  // The quick filters each own one filter (`layer`, `namespace`) and
  // nothing else: they rewrite just that entry and carry the rest of the
  // URL — search, sort, other filters, column layout — through untouched.
  const activeLayers = useMemo(
    () => activeFilterValues(tableUrlState.filters, "layer"),
    [tableUrlState.filters]
  )
  const filtersHref = useCallback(
    (filters: string) => {
      const next = new URLSearchParams(searchParams.toString())
      if (filters) next.set("filters", filters)
      else next.delete("filters")
      const query = next.toString()
      return query ? `/catalog?${query}` : "/catalog"
    },
    [searchParams]
  )

  // A namespace counts as picked only when the filter names exactly one;
  // two values typed into the toolbar are the user's own filter, and no
  // chip claims it.
  const activeNamespaceValues = useMemo(
    () => activeFilterValues(tableUrlState.filters, "namespace"),
    [tableUrlState.filters]
  )
  const activeNamespaceId =
    activeNamespaceValues.length === 1 ? activeNamespaceValues[0] : null
  const namespaceHref = useCallback(
    (namespaceId: string | null) =>
      filtersHref(
        namespaceId === null
          ? removeFilter(tableUrlState.filters, "namespace")
          : setFilter(tableUrlState.filters, {
              id: "namespace",
              value: namespaceId,
              variant: "text",
              operator: "eq",
            })
      ),
    [filtersHref, tableUrlState.filters]
  )
  const activeNamespace =
    namespaces.status === "success" && activeNamespaceId !== null
      ? (namespaces.data.find((ns) => ns.id === activeNamespaceId) ?? null)
      : null

  // Memoised: a new array each render would make TanStack Table rebuild
  // its column model, losing in-flight state like a column being dragged.
  //
  // Every column stays in the list — the ones the window is too narrow for
  // are hidden through the table's visibility, not dropped from it, so their
  // filters keep working (DATA-24).
  const columns = useMemo(
    () => getDataExplorerColumns({ onOpen: openAsset }),
    [openAsset]
  )

  const { table } = useDataTable({
    data: rows,
    columns,
    // The columns the window is too narrow for. Hidden through the table's
    // visibility, never removed from the column list, so the Owner and Tags
    // filters are still offered (DATA-24). The user's saved column layout
    // is left alone: the hide is the page's, not theirs.
    hiddenColumns: hiddenByWidth,
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
      columnVisibility: { id: false, certification: false },
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
        title="Catalog"
        description="Every governed table and file. Search by name, column, tag, description or owner, narrow by namespace or layer, and open an asset for its schema, quality, lineage and access."
      />

      {/* `DATA-55`: the tables dropped from the console, with the days each has
          left; each links to its page, where the restore lives. */}
      <DroppedTablesNote />

      {error ? (
        // React Query widens whatever was thrown to `Error`; `ErrorState`
        // branches on `ServiceError.code` (a 403 renders as a permission
        // notice, not a generic failure), so it is narrowed back here.
        <ErrorState
          error={toServiceError(error)}
          onRetry={() => void refetch()}
        />
      ) : !isLoading && !supported ? (
        // A refused tenant-scoped caller gets an otherwise-200 empty page
        // (`supported: false`), not an error — say why instead of showing
        // an empty table that reads as "no assets exist."
        <EmptyState
          title="The catalog cannot list assets for you"
          description={
            unsupportedReason ??
            "The API answered that listing assets is not supported for this account, and gave no reason."
          }
        />
      ) : isLoading ? (
        <DataTableSkeleton
          columnCount={columns.length - hiddenByWidth.length}
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
          <DataTableAdvancedToolbar
            table={table}
            onRefresh={() => void refetch()}
            exportName="Catalog"
          >
            <DataTableSearch placeholder="Search by name, column, tag, description or owner…" />
          </DataTableAdvancedToolbar>
          <div className="flex flex-col gap-2">
            <NamespaceFilterRow
              state={namespaces}
              activeId={activeNamespaceId}
              hrefFor={namespaceHref}
            />
            <QuickFilterRow label="Layer">
              <Button
                size="sm"
                variant={activeLayers.length === 0 ? "secondary" : "ghost"}
                render={
                  <Link
                    href={filtersHref(removeFilter(tableUrlState.filters, "layer"))}
                  />
                }
              >
                All
              </Button>
              {QUICK_LAYERS.map((layer) => {
                const active = activeLayers.includes(layer)
                return (
                  <Button
                    key={layer}
                    size="sm"
                    variant={active ? "secondary" : "ghost"}
                    aria-pressed={active}
                    className={cn("shrink-0", active && "font-semibold")}
                    render={
                      <Link
                        href={filtersHref(
                          toggleFilterValue(tableUrlState.filters, "layer", layer)
                        )}
                      />
                    }
                  >
                    {DATA_LAYER_LABEL[layer]}
                  </Button>
                )
              })}
            </QuickFilterRow>
            {activeNamespace ? (
              <NamespaceSummary
                namespace={activeNamespace}
                clearHref={namespaceHref(null)}
              />
            ) : null}
            {hiddenLabels.length > 0 ? (
              // Said out loud rather than left as a mystery: columns vanish
              // as the window narrows, and without this the table simply
              // looks like it lost data. The filters are still there
              // (DATA-24), so it says so.
              <p className="text-xs text-muted-foreground">
                {hiddenLabels.join(", ")}{" "}
                {hiddenLabels.length === 1 ? "is" : "are"} hidden at this
                window width — widen the window, or open an asset to see
                everything. The filters still work.
              </p>
            ) : null}
          </div>
        </DataTable>
      )}
    </div>
  )
}
