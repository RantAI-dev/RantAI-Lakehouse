# Data tables

Two patterns live in this folder, and picking the wrong one is the most
common mistake when adding a table. They differ in **where the filtering
happens**, not in how they look.

## 1. Server-paginated, infinite scroll

For lists that can grow without bound (assets, queries, audit events). The
URL is the source of truth; every change refetches.

Reference: `src/features/catalog/data-explorer-page.tsx`.

```tsx
const { rows, totalItems, fetchNextPage, hasNextPage } = useInfiniteTableQuery<Asset>({
  queryKey: [...],                              // only query-shaping state
  queryFn: (page, signal) =>
    service.listPage(toInfiniteQueryParams(tableUrlState, page), signal),
})

const { table } = useDataTable({
  data: rows,
  columns,
  pageCount: -1,          // the server never counts pages for infinite scroll
  rowCount: totalItems,
  paginationMode: "infinite",
  shallow: false,         // navigation must reach the server component
  enableAdvancedFilter: true,
})
```

Rules:

- Every filterable column `id` must exist in the backend allowlist
  (`FILTERABLE_FIELDS` in `rust/.../routes/catalog_query.rs`), or the API
  answers 400.
- Keep only query-shaping values in `queryKey`. Column order, widths and
  visibility also live in the URL but must not refetch.
- `hasNextPage` ends the scroll; a short page means "done".

## 2. Client-filtered, whole list in memory

For lists that are small and bounded by nature (catalog namespaces,
connector types). One fetch, then filtering happens in the browser.

Reference: `src/features/catalog/catalog-page.tsx`.

```tsx
const state = useService((s) => service.listAll(s), [])
const filtered = useMemo(
  () => filterDataClientSide(state.data ?? [], { search, searchFields, filters, joinOperator }),
  [...]
)
const { table } = useDataTable({
  data: filtered,
  columns,
  paginationMode: "infinite",
  manualFiltering: true,    // this page filtered already; don't filter twice
  manualPagination: false,
  persistKey: "/catalog",
})
```

Rules:

- `searchFields` decides what the search box matches. A column being
  visible does not make it searchable.
- Use this only where the full list is genuinely small. If it can reach
  thousands of rows, use pattern 1 instead.

## Shared behaviour

- **Toolbar** — `DataTableAdvancedToolbar` renders search, sort, filters and
  the Settings menu. Pass `exportName` to offer a CSV of what the table
  currently holds (filtered, sorted, and for infinite tables only as far as
  the user has scrolled — the item shows the row count for that reason).
- **Links into a filtered table** — build them with
  `src/lib/table-filter-link.ts`, never by hand. Every filter entry needs a
  `filterId`; without it the toolbar cannot parse the filter, so it drops it
  and no chip appears even though the rows come back filtered.
- **Loading** — use `DataTableSkeleton` with the real column count, so the
  layout does not jump when the data lands.
- **Narrow windows** — a page may drop columns by window width (see Data
  Explorer). Do that by leaving columns out of the column list, not through
  `columnVisibility`, which is persisted per user and would overwrite their
  saved choices on every resize. Tell the user which columns are hidden.
