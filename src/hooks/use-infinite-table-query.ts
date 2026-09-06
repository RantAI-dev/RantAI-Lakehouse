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
  queryFn: (page: number) => Promise<Pagination<T>>;
  enabled?: boolean;
}

export function useInfiniteTableQuery<T>({
  queryKey,
  queryFn,
  enabled = true,
}: UseInfiniteTableQueryOptions<T>) {
  const query = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => queryFn(pageParam as number),
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
