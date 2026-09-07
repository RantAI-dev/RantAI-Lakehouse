import type { Pagination } from "@/services/contracts/pagination";

export function flattenInfinitePages<T>(
  pages: Pagination<T>[] | undefined
): T[] {
  return pages?.flatMap((page) => page.items) ?? [];
}

/**
 * Prefer a short last page over `totalPages`: when `skipListMeta` is set,
 * page > 1 responses omit an accurate total (see `paginate_select`), so chunk
 * length is the signal to keep fetching.
 */
export function getInfiniteTableNextPageParam<T>(lastPage: Pagination<T>) {
  if (lastPage.items.length < lastPage.pageSize) return undefined;
  return lastPage.page + 1;
}
