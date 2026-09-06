import {
  type ColumnFiltersState,
  type ColumnOrderState,
  type ColumnPinningState,
  getCoreRowModel,
  getFacetedMinMaxValues,
  getFacetedRowModel,
  getFacetedUniqueValues,
  getFilteredRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  type PaginationState,
  type RowSelectionState,
  type SortingState,
  type TableOptions,
  type TableState,
  type Updater,
  useReactTable,
  type VisibilityState,
} from "@tanstack/react-table";
import {
  parseAsArrayOf,
  parseAsInteger,
  parseAsString,
  type SingleParser,
  type UseQueryStateOptions,
  useQueryState,
  useQueryStates,
} from "nuqs";
import * as React from "react";

import { useDebouncedCallback } from "@/hooks/use-debounced-callback";
import { type TableLayout, useTableLayout } from "@/hooks/use-table-layout";
import { useTableMemory } from "@/hooks/use-table-memory";
import { dataTableConfig } from "@/config/data-table";
import { getSortingStateParser } from "@/lib/parsers";
import type { ExtendedColumnSort, QueryKeys } from "@/types/data-table";

const PAGE_KEY = "page";
const PER_PAGE_KEY = "perPage";
const SORT_KEY = "sort";
const FILTERS_KEY = "filters";
const JOIN_OPERATOR_KEY = "joinOperator";
const SEARCH_KEY = "search";
const VIEW_KEY = "view";
const GROUP_BY_KEY = "groupBy";
const ARRAY_SEPARATOR = ",";
const DEBOUNCE_MS = 300;
const THROTTLE_MS = 50;

interface UseDataTableProps<TData>
  extends
    Omit<
      TableOptions<TData>,
      | "state"
      | "pageCount"
      | "getCoreRowModel"
      | "manualFiltering"
      | "manualPagination"
      | "manualSorting"
    >,
    Required<Pick<TableOptions<TData>, "pageCount">> {
  initialState?: Omit<Partial<TableState>, "sorting"> & {
    sorting?: ExtendedColumnSort<TData>[];
  };
  queryKeys?: Partial<QueryKeys>;
  /**
   * Names the slot this table is remembered under — both its column layout and
   * its view, search, sort and filters. Defaults to the pathname, which only
   * needs overriding when one route holds two tables.
   */
  persistKey?: string;
  /** Extra URL params to remember with table state (e.g. page-specific filters). */
  memoryKeys?: string[];
  history?: "push" | "replace";
  debounceMs?: number;
  throttleMs?: number;
  clearOnDefault?: boolean;
  enableAdvancedFilter?: boolean;
  scroll?: boolean;
  shallow?: boolean;
  startTransition?: React.TransitionStartFunction;
  /** Page buttons (`pages`) or infinite scroll (`infinite`). */
  paginationMode?: "pages" | "infinite";
}

/**
 * Keeps the columns a page pinned itself — a row's action menu, a select
 * checkbox — hard against their edge. TanStack pins to the outside of the
 * group, so without this the action menu ends up buried behind whatever the
 * user pinned last.
 */
function anchorPinning(
  pinning: ColumnPinningState,
  anchors: ColumnPinningState
): ColumnPinningState {
  const split = (ids: string[] = [], anchored: string[] = []) => ({
    anchored: ids.filter((id) => anchored.includes(id)),
    rest: ids.filter((id) => !anchored.includes(id)),
  });

  const left = split(pinning.left, anchors.left);
  const right = split(pinning.right, anchors.right);

  return {
    // Order runs inwards from each edge, so the anchors lead on the left and
    // trail on the right.
    left: [...left.anchored, ...left.rest],
    right: [...right.rest, ...right.anchored],
  };
}

export function useDataTable<TData>(props: UseDataTableProps<TData>) {
  const {
    columns,
    pageCount = -1,
    initialState,
    queryKeys,
    persistKey,
    memoryKeys,
    history = "replace",
    debounceMs = DEBOUNCE_MS,
    throttleMs = THROTTLE_MS,
    clearOnDefault = false,
    enableAdvancedFilter = false,
    scroll = false,
    shallow = true,
    startTransition,
    paginationMode = "pages",
    ...tableProps
  } = props;
  const pageKey = queryKeys?.page ?? PAGE_KEY;
  const perPageKey = queryKeys?.perPage ?? PER_PAGE_KEY;
  const sortKey = queryKeys?.sort ?? SORT_KEY;
  const filtersKey = queryKeys?.filters ?? FILTERS_KEY;
  const joinOperatorKey = queryKeys?.joinOperator ?? JOIN_OPERATOR_KEY;
  const searchKey = queryKeys?.search ?? SEARCH_KEY;
  const viewKey = queryKeys?.view ?? VIEW_KEY;
  const groupByKey = queryKeys?.groupBy ?? GROUP_BY_KEY;

  const tableKeys = React.useMemo<QueryKeys>(
    () => ({
      page: pageKey,
      perPage: perPageKey,
      sort: sortKey,
      filters: filtersKey,
      joinOperator: joinOperatorKey,
      search: searchKey,
      view: viewKey,
      groupBy: groupByKey,
    }),
    [
      pageKey,
      perPageKey,
      sortKey,
      filtersKey,
      joinOperatorKey,
      searchKey,
      viewKey,
      groupByKey,
    ]
  );

  const queryStateOptions = React.useMemo<
    Omit<UseQueryStateOptions<string>, "parse">
  >(
    () => ({
      history,
      scroll,
      shallow,
      throttleMs,
      debounceMs,
      clearOnDefault,
      startTransition,
    }),
    [
      history,
      scroll,
      shallow,
      throttleMs,
      debounceMs,
      clearOnDefault,
      startTransition,
    ]
  );

  const [rowSelection, setRowSelection] = React.useState<RowSelectionState>(
    initialState?.rowSelection ?? {}
  );

  // Read once: pages build `initialState` inline, so re-reading it every render
  // would hand the table a new default object each time.
  const [layoutDefaults] = React.useState<TableLayout>(() => ({
    columnOrder: initialState?.columnOrder ?? [],
    columnPinning: initialState?.columnPinning ?? {},
    columnVisibility: initialState?.columnVisibility ?? {},
  }));

  const {
    layout,
    setLayout,
    reset: resetStoredLayout,
  } = useTableLayout(layoutDefaults, persistKey);

  const memoryKeySignature = (memoryKeys ?? []).join("\0");
  const stableMemoryKeys = React.useMemo(
    () => memoryKeys ?? [],
    [memoryKeySignature]
  );

  const columnIds = React.useMemo(() => {
    return new Set(
      columns.map((column) => column.id).filter(Boolean) as string[]
    );
  }, [columns]);

  // View, search, sort and filters live in the URL, which the sidebar drops on
  // the way out. This puts them back on the way in — through the same
  // whitelist the controls use, so a filter on a column that has since been
  // renamed is dropped rather than restored into a request the server refuses.
  useTableMemory(persistKey, tableKeys, columnIds, stableMemoryKeys);

  // Everything about how the columns are arranged, visibility included.
  const resetLayout = React.useCallback(() => {
    resetStoredLayout();
  }, [resetStoredLayout]);

  const onColumnOrderChange = React.useCallback(
    (updaterOrValue: Updater<ColumnOrderState>) => {
      setLayout({
        ...layout,
        columnOrder:
          typeof updaterOrValue === "function"
            ? updaterOrValue(layout.columnOrder)
            : updaterOrValue,
      });
    },
    [layout, setLayout]
  );

  const onColumnVisibilityChange = React.useCallback(
    (updaterOrValue: Updater<VisibilityState>) => {
      setLayout({
        ...layout,
        columnVisibility:
          typeof updaterOrValue === "function"
            ? updaterOrValue(layout.columnVisibility)
            : updaterOrValue,
      });
    },
    [layout, setLayout]
  );

  // Normalised before it reaches the table, so a layout stored before the
  // anchoring rule existed is corrected on load rather than on next change.
  const columnPinning = React.useMemo(
    () => anchorPinning(layout.columnPinning, layoutDefaults.columnPinning),
    [layout.columnPinning, layoutDefaults.columnPinning]
  );

  const onColumnPinningChange = React.useCallback(
    (updaterOrValue: Updater<ColumnPinningState>) => {
      setLayout({
        ...layout,
        columnPinning: anchorPinning(
          typeof updaterOrValue === "function"
            ? updaterOrValue(columnPinning)
            : updaterOrValue,
          layoutDefaults.columnPinning
        ),
      });
    },
    [layout, columnPinning, layoutDefaults.columnPinning, setLayout]
  );

  const isInfinite = paginationMode === "infinite";
  const infiniteChunkSize = dataTableConfig.infiniteTableChunkSize;
  const defaultPageSize =
    initialState?.pagination?.pageSize ??
    (isInfinite ? infiniteChunkSize : 10);

  const [page, setPageState] = useQueryState(
    pageKey,
    parseAsInteger.withOptions(queryStateOptions).withDefault(1)
  );

  const [perPage, setPerPageState] = useQueryState(
    perPageKey,
    parseAsInteger
      .withOptions(queryStateOptions)
      .withDefault(defaultPageSize)
  );

  const pageSize = isInfinite ? infiniteChunkSize : perPage;

  const pagination: PaginationState = React.useMemo(() => {
    return {
      pageIndex: isInfinite ? 0 : page - 1,
      pageSize,
    };
  }, [isInfinite, page, pageSize]);

  const onPaginationChange = React.useCallback(
    (updaterOrValue: Updater<PaginationState>) => {
      if (isInfinite) return;

      if (typeof updaterOrValue === "function") {
        const newPagination = updaterOrValue(pagination);
        void setPageState(newPagination.pageIndex + 1);
        void setPerPageState(newPagination.pageSize);
      } else {
        void setPageState(updaterOrValue.pageIndex + 1);
        void setPerPageState(updaterOrValue.pageSize);
      }
    },
    [isInfinite, pagination, setPageState, setPerPageState]
  );

  const [sorting, setSorting] = useQueryState(
    sortKey,
    getSortingStateParser<TData>(columnIds)
      .withOptions(queryStateOptions)
      .withDefault(initialState?.sorting ?? [])
  );

  const onSortingChange = React.useCallback(
    (updaterOrValue: Updater<SortingState>) => {
      if (!isInfinite) {
        void setPageState(1);
      }
      if (typeof updaterOrValue === "function") {
        const newSorting = updaterOrValue(sorting);
        void setSorting(newSorting as ExtendedColumnSort<TData>[]);
      } else {
        void setSorting(updaterOrValue as ExtendedColumnSort<TData>[]);
      }
    },
    [isInfinite, sorting, setPageState, setSorting]
  );

  const [groupBy, setGroupByState] = useQueryState(
    groupByKey,
    parseAsString.withOptions(queryStateOptions)
  );

  const setGroupBy = React.useCallback(
    (columnId: string | null) => {
      if (!isInfinite) {
        void setPageState(1);
      }
      void setGroupByState(columnId);
    },
    [isInfinite, setGroupByState, setPageState]
  );

  const filterableColumns = React.useMemo(() => {
    if (enableAdvancedFilter) return [];

    return columns.filter((column) => column.enableColumnFilter);
  }, [columns, enableAdvancedFilter]);

  const filterParsers = React.useMemo(() => {
    if (enableAdvancedFilter) return {};

    return filterableColumns.reduce<
      Record<string, SingleParser<string> | SingleParser<string[]>>
    >((acc, column) => {
      if (column.meta?.options) {
        acc[column.id ?? ""] = parseAsArrayOf(
          parseAsString,
          ARRAY_SEPARATOR
        ).withOptions(queryStateOptions);
      } else {
        acc[column.id ?? ""] = parseAsString.withOptions(queryStateOptions);
      }
      return acc;
    }, {});
  }, [filterableColumns, queryStateOptions, enableAdvancedFilter]);

  const [filterValues, setFilterValues] = useQueryStates(filterParsers);

  const debouncedSetFilterValues = useDebouncedCallback(
    (values: typeof filterValues) => {
      if (!isInfinite) {
        void setPageState(1);
      }
      void setFilterValues(values);
    },
    debounceMs
  );

  const initialColumnFilters: ColumnFiltersState = React.useMemo(() => {
    if (enableAdvancedFilter) return [];

    return Object.entries(filterValues).reduce<ColumnFiltersState>(
      (filters, [key, value]) => {
        if (value !== null) {
          const processedValue = Array.isArray(value)
            ? value
            : typeof value === "string" && /[^a-zA-Z0-9]/.test(value)
              ? value.split(/[^a-zA-Z0-9]+/).filter(Boolean)
              : [value];

          filters.push({
            id: key,
            value: processedValue,
          });
        }
        return filters;
      },
      []
    );
  }, [filterValues, enableAdvancedFilter]);

  const [columnFilters, setColumnFilters] =
    React.useState<ColumnFiltersState>(initialColumnFilters);

  const onColumnFiltersChange = React.useCallback(
    (updaterOrValue: Updater<ColumnFiltersState>) => {
      if (enableAdvancedFilter) return;

      setColumnFilters((prev) => {
        const next =
          typeof updaterOrValue === "function"
            ? updaterOrValue(prev)
            : updaterOrValue;

        const filterUpdates = next.reduce<
          Record<string, string | string[] | null>
        >((acc, filter) => {
          if (filterableColumns.find((column) => column.id === filter.id)) {
            acc[filter.id] = filter.value as string | string[];
          }
          return acc;
        }, {});

        for (const prevFilter of prev) {
          if (!next.some((filter) => filter.id === prevFilter.id)) {
            filterUpdates[prevFilter.id] = null;
          }
        }

        debouncedSetFilterValues(filterUpdates);
        return next;
      });
    },
    [debouncedSetFilterValues, filterableColumns, enableAdvancedFilter]
  );

  // eslint-disable-next-line react-hooks/incompatible-library
  const table = useReactTable({
    ...tableProps,
    columns,
    initialState,
    pageCount,
    state: {
      pagination,
      sorting,
      columnVisibility: layout.columnVisibility,
      columnOrder: layout.columnOrder,
      columnPinning,
      rowSelection,
      columnFilters,
    },
    defaultColumn: {
      ...tableProps.defaultColumn,
      enableColumnFilter: false,
    },
    // Off unless the page asks for it, so the footer can tell a table that
    // actually selects rows from one that merely could.
    enableRowSelection: tableProps.enableRowSelection ?? false,
    onRowSelectionChange: setRowSelection,
    onPaginationChange,
    onSortingChange,
    onColumnFiltersChange,
    onColumnVisibilityChange,
    onColumnOrderChange,
    onColumnPinningChange,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFacetedRowModel: getFacetedRowModel(),
    getFacetedUniqueValues: getFacetedUniqueValues(),
    getFacetedMinMaxValues: getFacetedMinMaxValues(),
    manualPagination: true,
    manualSorting: true,
    manualFiltering: true,
    meta: {
      ...tableProps.meta,
      resetLayout,
      queryKeys: tableKeys,
      groupBy: groupBy ?? null,
      setGroupBy,
      paginationMode,
      memoryKeys: stableMemoryKeys,
    },
  });

  return React.useMemo(
    () => ({ table, shallow, debounceMs, throttleMs, paginationMode }),
    [table, shallow, debounceMs, throttleMs, paginationMode]
  );
}
