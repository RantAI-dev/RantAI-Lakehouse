"use client"

import * as React from "react"
import { useCallback, useEffect, useMemo } from "react"
import { useRouter, useSearchParams } from "next/navigation"
import { Database, SearchX, Trash2 } from "lucide-react"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableQueryKeysProvider } from "@/components/data-table/data-table-query-keys"
import { useDataTableReset } from "@/components/data-table/data-table-reset-filters"
import { DataTableSettingsMenu } from "@/components/data-table/data-table-settings-menu"
import {
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/components/ui/context-menu"
import { DropdownMenuItem } from "@rantai/design-system/ui/dropdown-menu"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
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
} from "@/lib/table-filter-link"
import type { DataLayer } from "@/lib/status"
import { AddDataButton, CatalogSearch, LayerSwitch } from "./catalog-frame"
import { isCatalogQuickFilter } from "./catalog-layers"
import { NamespaceLine, NamespacePicker } from "./catalog-namespaces"
import { getAssetActions } from "./data-explorer-actions"
import { DroppedTablesDialog, useDroppedTables } from "./dropped-tables"
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
 *
 * `UX-8` (mockup A, owner 2026-11-21) puts it in the app's one page frame
 * (`DESIGN.md` §3): the title, then on the right the icon search, the ⋯
 * (the table's settings menu: columns, grouping, export, refresh, layout,
 * plus the dropped tables) and the primary Add data, on one row. Under it
 * one filter row: the layer switch and the namespace picker, then the
 * table's own Sort and Filter. The table is the same advanced data table.
 */
export function CatalogPage() {
  const router = useRouter()
  const searchParams = useSearchParams()
  const tableUrlState = useTableUrlState()
  const { hasPermission } = useAuth()
  const namespaces = useService((s) => assetService.listNamespaces(s), [])
  // `DATA-55`: read once here, so the ⋯ item can count them and the dialog list them.
  const dropped = useDroppedTables()
  const [droppedOpen, setDroppedOpen] = React.useState(false)

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

  // All columns need ~875px, and the content area is much narrower than
  // the window once the sidebar is open — at a 900px window it is only
  // ~604px. Rather than let the page scroll sideways (which drags the
  // sidebar and navbar with it and clips the last columns outright), leave
  // out the columns that earn their width least as space runs out.
  //
  // Order of sacrifice, least useful first: Tags and Owner (DATA-11; both
  // are on the detail page) → Size (nice to know) → Freshness (also shown on the detail page) → Type (largely implied by
  // Layer). Name, Tier and the actions menu always stay.
  //
  // The columns are hidden, not removed (DATA-24): they stay in the column
  // list, so their filters keep working in the toolbar's filter menu at
  // every width. They used to be dropped from the column list, which took
  // their filters with them — the Owner and Tags filters vanished on a
  // laptop.
  //
  // UX-8 (plan PR 1): the columns fold silently. The line that named them
  // ("… hidden at this window width") is gone; the asset page shows
  // everything, and the filters say for themselves that they still work.
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

  // The quick filters each own one filter (`layer`, `namespace`) and
  // nothing else: they rewrite just that entry and carry the rest of the
  // URL — search, sort, other filters, column layout — through untouched.
  //
  // UX-8 (plan PR 1, "pick one and say which"): they are the Filter menu's
  // own state, not a second filter. Both write the one `filters` entry for
  // their column, so a layer picked in the switch is the Layer chip the
  // menu shows, and a mix set in the menu (Silver and Gold) leaves no
  // segment pressed. The menu keeps them because it is the only place for
  // the rarer questions (Bronze (raw) alone, two namespaces at once), and
  // because the table drops every filter whose column it cannot filter on
  // (`getFiltersStateParser`), so the columns cannot leave the menu. A
  // filter the switch or the picker shows gets no chip in the property bar
  // (`isCatalogQuickFilter`), so it is on screen once; one they cannot show
  // keeps its chip.
  const activeLayers = useMemo(
    () => activeFilterValues(tableUrlState.filters, "layer"),
    [tableUrlState.filters]
  )
  const pushFilters = useCallback(
    (filters: string) => {
      const next = new URLSearchParams(searchParams.toString())
      if (filters) next.set("filters", filters)
      else next.delete("filters")
      const query = next.toString()
      router.push(query ? `/catalog?${query}` : "/catalog", { scroll: false })
    },
    [router, searchParams]
  )
  const pickLayers = useCallback(
    (layers: DataLayer[]) =>
      pushFilters(
        layers.length === 0
          ? removeFilter(tableUrlState.filters, "layer")
          : setFilter(tableUrlState.filters, {
              id: "layer",
              value: layers,
              variant: "multiSelect",
              operator: "inArray",
            })
      ),
    [pushFilters, tableUrlState.filters]
  )

  // A namespace counts as picked only when the filter names exactly one;
  // two values typed into the toolbar are the user's own filter, and the
  // picker does not claim it.
  const activeNamespaceValues = useMemo(
    () => activeFilterValues(tableUrlState.filters, "namespace"),
    [tableUrlState.filters]
  )
  const activeNamespaceId =
    activeNamespaceValues.length === 1 ? activeNamespaceValues[0] : null
  const pickNamespace = useCallback(
    (namespaceId: string | null) =>
      pushFilters(
        namespaceId === null
          ? removeFilter(tableUrlState.filters, "namespace")
          : setFilter(tableUrlState.filters, {
              id: "namespace",
              value: namespaceId,
              variant: "text",
              operator: "eq",
            })
      ),
    [pushFilters, tableUrlState.filters]
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
    // UX-8: the layer switch and the namespace picker show these filters,
    // so the property bar draws no second chip for them.
    meta: { isQuickFilter: isCatalogQuickFilter },
    initialState: {
      // `id` stays available in the column menu, filters and the row
      // context menu; it is just noise as a column. UX-8: Namespace joins
      // it, since the address under each name already says the namespace;
      // the Columns menu brings either back.
      columnVisibility: { id: false, certification: false, namespace: false },
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

  const droppedCount = dropped.status === "success" ? dropped.data.length : null
  const listShown = !error && (isLoading || supported)

  return (
    // The search, the ⋯ and the no-match Reset write the table's own URL
    // keys, named by the table; the toolbar provides them to its own row, and
    // this provides them to the rest of the page.
    <DataTableQueryKeysProvider
      keys={table.options.meta?.queryKeys}
      paginationMode={table.options.meta?.paginationMode}
      resetExtraKeys={table.options.meta?.memoryKeys}
    >
      <div className="flex min-w-0 flex-col gap-3">
        {/* The page frame (`DESIGN.md` §3): title left; search, ⋯ and the
            primary on the right, every control 32 px, one row. No
            description under the title (plan decision 1). */}
        <header className="flex min-w-0 items-center gap-2">
          <h1 className="mr-2 shrink-0 text-xl leading-8 font-semibold tracking-tight">Catalog</h1>
          <div aria-hidden className="flex-1" />
          {/* With no list to act on (refused, failed, unsupported) the search
              and the ⋯ would do nothing, so only the title and the state show. */}
          {listShown ? (
            <>
              <CatalogSearch />
              <DataTableSettingsMenu
                table={table}
                variant="page"
                label="More catalog actions"
                onRefresh={() => void refetch()}
                exportName="Catalog"
                extraItems={
                  // `DATA-55`: offered while there is something to open, and when
                  // the list could not be read (the dialog says so, with a Retry).
                  (droppedCount !== null && droppedCount > 0) || dropped.status === "error" ? (
                    <DropdownMenuItem onSelect={() => setDroppedOpen(true)}>
                      <Trash2 className="text-muted-foreground" />
                      Dropped tables
                      {droppedCount !== null ? (
                        <span className="ml-auto text-xs text-muted-foreground tabular-nums">{droppedCount}</span>
                      ) : null}
                    </DropdownMenuItem>
                  ) : null
                }
              />
            </>
          ) : null}
          {hasPermission("connector:manage") ? <AddDataButton /> : null}
        </header>

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
        ) : (
          <>
            <DataTableAdvancedToolbar
              table={table}
              settingsMenu={false}
              onRefresh={() => void refetch()}
              exportName="Catalog"
            >
              <LayerSwitch values={activeLayers} onPick={pickLayers} className="max-sm:w-full" />
              <NamespacePicker
                state={namespaces}
                activeId={activeNamespaceId}
                onPick={pickNamespace}
              />
            </DataTableAdvancedToolbar>
            {activeNamespace ? <NamespaceLine namespace={activeNamespace} /> : null}
            {isLoading ? (
              <CatalogTableSkeleton />
            ) : rows.length === 0 ? (
              <CatalogNoRows />
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
              />
            )}
          </>
        )}
      </div>
      <DroppedTablesDialog open={droppedOpen} onOpenChange={setDroppedOpen} state={dropped} />
    </DataTableQueryKeysProvider>
  )
}

/**
 * The table while the first page loads, shaped like the rows it stands for
 * (`DESIGN.md` §3, one loading state shaped like the page): the frame and
 * the filter row above are already the real ones, so only the rows wait.
 */
function CatalogTableSkeleton() {
  return (
    <div role="status" aria-label="Loading the catalog" className="flex flex-col">
      <div className="flex h-10 items-center gap-6 border-b border-border px-2">
        {["w-12", "w-10", "w-8", "w-16", "w-10"].map((w, i) => (
          <Skeleton key={i} className={`h-3 ${w} ${i === 0 ? "mr-auto" : ""}`} />
        ))}
      </div>
      {Array.from({ length: 8 }, (_, i) => (
        <div key={i} className="flex h-[53px] items-center gap-6 border-b border-border px-2 last:border-b-0">
          <div className="mr-auto flex flex-col gap-1.5">
            <Skeleton className="h-3.5 w-44" />
            <Skeleton className="h-3 w-32" />
          </div>
          <Skeleton className="h-3 w-16" />
          <Skeleton className="h-5 w-12 rounded-full" />
          <Skeleton className="h-3 w-16" />
          <Skeleton className="h-3 w-12" />
          <Skeleton className="size-6 rounded-md" />
        </div>
      ))}
    </div>
  )
}

/**
 * No rows: either nothing matches what the person narrowed to, which says
 * so and offers the toolbar's own Reset, or the catalog holds nothing yet
 * (plan decision 5: empty and no match are designed, not the table's bare
 * "No results.").
 */
function CatalogNoRows() {
  const { canReset, reset } = useDataTableReset()
  const searchParams = useSearchParams()
  const search = searchParams.get("search")
  if (canReset) {
    return (
      <EmptyState
        icon={SearchX}
        title={search ? `Nothing matches “${search}”` : "No asset matches these filters"}
        description="Search looks at names, columns, tags, descriptions and owners."
        action={
          <Button variant="outline" onClick={reset}>
            {search ? "Clear search and filters" : "Clear filters"}
          </Button>
        }
      />
    )
  }
  return (
    <EmptyState
      icon={Database}
      title="Nothing in the catalog yet"
      description="Tables appear here once a source has loaded them."
    />
  )
}
