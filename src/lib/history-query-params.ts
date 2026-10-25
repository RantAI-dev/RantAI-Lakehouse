import type { QueryHistoryFilter } from "@/services/contracts/queries";

/**
 * Turns the history page's filter object into the query string
 * `GET /api/query/history` expects (`QS-4`, decisions 5 and 7): present
 * fields percent-encoded in the contract's order, empty strings dropped
 * (the server reads an empty value as "no filter"), and an empty result
 * for an absent or all-empty filter so the caller adds no stray `?`.
 * `before` is the page cursor — the `at` and `id` of the last row seen,
 * joined with the comma the server splits on; an ISO `at` carries no
 * comma of its own.
 */
export function historyQueryString(filter?: QueryHistoryFilter): string {
  if (!filter) return "";
  const params = new URLSearchParams();
  if (filter.status) params.set("status", filter.status);
  if (filter.engine) params.set("engine", filter.engine);
  if (filter.minDurationMs !== undefined) {
    params.set("minDurationMs", String(filter.minDurationMs));
  }
  if (filter.table) params.set("table", filter.table);
  if (filter.from) params.set("from", filter.from);
  if (filter.to) params.set("to", filter.to);
  if (filter.before) params.set("before", `${filter.before.at},${filter.before.id}`);
  return params.toString();
}

/**
 * The page size the history endpoint answers with — the server's
 * `HISTORY_PAGE_LIMIT` (`routes/query.rs`). The page's "Load more"
 * infers the end of the trail from it: a page that comes back with
 * fewer rows was the last one, so no older page can exist.
 */
export const HISTORY_PAGE_LIMIT = 50
