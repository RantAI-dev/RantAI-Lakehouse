"use client";

import { useInfiniteQuery } from "@tanstack/react-query";

import {
  flattenInfinitePages,
  getInfiniteTableNextPageParam,
} from "@/lib/data-table-infinite";
import type { Pagination } from "@/services/contracts/pagination";

export { flattenInfinitePages, getInfiniteTableNextPageParam };

interface UseInfiniteTableQueryOptions<T> {
  queryKey: unknown[];
  /**
   * Fetches one page. The `signal` is React Query's own — forwarding it to
   * the service call means an abandoned request (the user retyped the
   * search, or left the page) is actually cancelled rather than left to
   * resolve into a cache nobody is reading. Every service adapter in
   * `services/clients` already accepts one.
   */
  queryFn: (page: number, signal: AbortSignal) => Promise<Pagination<T>>;
  enabled?: boolean;
}

export function useInfiniteTableQuery<T>({
  queryKey,
  queryFn,
  enabled = true,
}: UseInfiniteTableQueryOptions<T>) {
  const query = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam, signal }) => queryFn(pageParam as number, signal),
    initialPageParam: 1,
    getNextPageParam: getInfiniteTableNextPageParam,
    enabled,
    staleTime: 30_000,
    placeholderData: (previousData) => previousData,
  });

  const rows = flattenInfinitePages(query.data?.pages);
  const totalItems = query.data?.pages[0]?.totalItems ?? 0;
  const groupSummaries = query.data?.pages[0]?.groups ?? null;

  return {
    ...query,
    rows,
    totalItems,
    groupSummaries,
    loadedCount: rows.length,
  };
}
