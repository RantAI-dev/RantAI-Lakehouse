import type { QueryHistoryFilter } from "@/services/contracts/queries";

/**
 * Turns the history page's filter object into the query string
 * `GET /api/query/history` expects (`QS-4`, decisions 5 and 7): present
 * fields percent-encoded in the contract's order, empty strings dropped
 * (the server reads an empty value as "no filter"), and an empty result
 * for an absent or all-empty filter so the caller adds no stray `?`.
 * `page` is the 1-based page number; the first page is the server's
 * default, so `page` is written only from the second page on, and a
 * first-page query stays `""` when nothing else is set.
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
  if (filter.page !== undefined && filter.page > 1) params.set("page", String(filter.page));
  return params.toString();
}

/**
 * The page size the history endpoint answers with, the server's
 * `HISTORY_PAGE_LIMIT` (`routes/query.rs`). The response also carries
 * `pageSize`; this constant is the fallback the page uses before the
 * first answer lands.
 */
export const HISTORY_PAGE_LIMIT = 50
