/**
 * Wire contract for server-paginated list endpoints, shared by the
 * Advanced Data Table stack (`components/data-table/*`) and the service
 * adapters that feed it.
 *
 * Deliberately generic rather than per-domain: the table components are
 * domain-agnostic, so anything they consume has to be too. The Data
 * Explorer pilot (`AssetService.listAssetsPage`) is the first
 * implementation; further tables adopt the same shape rather than
 * inventing their own.
 */

/** One server-side group header, when `groupBy` is active. */
export interface GroupSummary {
  id: string;
  label: string;
  count: number;
}

export interface Pagination<T> {
  totalItems: number;
  totalPages: number;
  pageSize: number;
  page: number;
  items: T[];
  groupBy?: string | null;
  groups?: GroupSummary[] | null;
  /** Parallel to `items` when the API grouped the response. */
  itemGroupKeys?: string[] | null;
}

export interface PaginationQuery {
  page: number;
  pageSize: number;
  search?: string;
  orderBy?: string;
  orderDirection?: "asc" | "desc";
  /** JSON-encoded data-table filters forwarded to the API. */
  filters?: string;
  /** JSON-encoded multi-column sorting forwarded to the API. */
  sort?: string;
  joinOperator?: "and" | "or";
  /** Column id for server-side row grouping. */
  groupBy?: string;
  /**
   * Infinite scroll: skip the count + group summaries on page > 1, since
   * the client already has them from the first response.
   *
   * Note for implementors: when this is set, `totalItems`/`totalPages` may
   * be inaccurate, but `items.length` must NOT be — the client decides
   * whether to keep fetching by comparing it against `pageSize` (see
   * `getInfiniteTableNextPageParam`), so a short page means "done".
   */
  skipListMeta?: boolean;
}

export const defaultPaginationQuery: PaginationQuery = {
  page: 1,
  pageSize: 10,
  search: "",
};
