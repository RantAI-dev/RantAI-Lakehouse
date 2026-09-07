"use client";

import { useMemo } from "react";
import { useSearchParams } from "next/navigation";
import { parseAsStringLiteral, useQueryState } from "nuqs";

import {
  DEFAULT_QUERY_KEYS,
  useDataTableQueryKeys,
} from "@/components/data-table/data-table-query-keys";
import { dataTableConfig } from "@/config/data-table";
import type { QueryKeys } from "@/types/data-table";

export const TABLE_VIEWS = ["table", "list"] as const;

export type TableView = (typeof TABLE_VIEWS)[number];

function resolveQueryKeys(queryKeys?: Partial<QueryKeys>): QueryKeys {
  return { ...DEFAULT_QUERY_KEYS, ...queryKeys };
}

/**
 * The table state a page has to send to its endpoint.
 *
 * TableCN owns these values through nuqs. The page reads the resulting URL back
 * through Next rather than mounting its own `useQueryStates` — one nuqs owner
 * per key, or the second instance sits on a stale snapshot and the query never
 * sees the change.
 *
 * Pass the same `queryKeys` partial you give `useDataTable` when renaming URL
 * params (e.g. two tables on one route). Inside a toolbar provider the context
 * keys are used automatically when no override is passed.
 *
 * Fetch state follows the live URL only. Remembered prefs are applied by
 * writing the URL (sidebar click rewriter + `useTableMemory` restore), never
 * by silently merging localStorage into the query key while the URL is bare —
 * that left the toolbar empty and the list still filtered.
 */
export function useTableUrlState(queryKeys?: Partial<QueryKeys>) {
  const searchParams = useSearchParams();
  const { keys: contextKeys } = useDataTableQueryKeys();
  const keys = queryKeys ? resolveQueryKeys(queryKeys) : contextKeys;

  return useMemo(() => {
    const page = Number(searchParams.get(keys.page));
    const perPage = Number(searchParams.get(keys.perPage));
    const joinOperator = searchParams.get(keys.joinOperator);
    const normalizedJoinOperator: "and" | "or" =
      joinOperator === "or" ? "or" : "and";

    return {
      page: Number.isInteger(page) && page > 0 ? page : 1,
      perPage: Number.isInteger(perPage) && perPage > 0 ? perPage : 10,
      search: searchParams.get(keys.search) ?? "",
      sort: searchParams.get(keys.sort) ?? "",
      filters: searchParams.get(keys.filters) ?? "",
      joinOperator: normalizedJoinOperator,
      groupBy: searchParams.get(keys.groupBy) ?? "",
    };
  }, [keys, searchParams]);
}

/**
 * Which rendering of the rows is on screen. It rides in the URL like the rest
 * of the table state, so a shared link opens the way the sender left it, and it
 * changes nothing about the query.
 */
export function useTableView(queryKeys?: Partial<QueryKeys>) {
  const { keys: contextKeys } = useDataTableQueryKeys();
  const keys = queryKeys ? resolveQueryKeys(queryKeys) : contextKeys;

  return useQueryState(
    keys.view,
    parseAsStringLiteral(TABLE_VIEWS).withDefault("table")
  );
}

/**
 * The query-shaped half of the URL state, ready to spread into a fetch. Absent
 * values are left out entirely rather than sent empty, and `joinOperator` only
 * travels with the filters it applies to.
 */
export function toQueryParams(state: ReturnType<typeof useTableUrlState>) {
  return {
    page: state.page,
    pageSize: state.perPage,
    ...(state.search && { search: state.search }),
    ...(state.sort && { sort: state.sort }),
    ...(state.filters && {
      filters: state.filters,
      joinOperator: state.joinOperator,
    }),
    ...(state.groupBy && { groupBy: state.groupBy }),
  };
}

/** Like `toQueryParams`, but fixes the chunk size for infinite-scroll fetches. */
export function toInfiniteQueryParams(
  state: ReturnType<typeof useTableUrlState>,
  page: number
) {
  return {
    ...toQueryParams({
      ...state,
      page,
      perPage: dataTableConfig.infiniteTableChunkSize,
    }),
    skipListMeta: true,
  };
}
