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
  if (filter.id) params.set("id", filter.id);
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

/**
 * QS-12 (LH-QS-4-06), D17: the history page's own filter state, in the shape
 * the controls hold — strings for the text fields, `YYYY-MM-DD` for the date
 * fields, `page` a 1-based number. An unset status or engine is `"all"`, an
 * unset text or date field is `""`, and the first page is 1.
 */
export type HistoryAddress = {
  status: string
  engine: string
  minDuration: string
  table: string
  from: string
  to: string
  page: number
}

const ADDRESS_STATUSES = new Set(["completed", "failed", "stopped", "blocked"]);
const ADDRESS_ENGINES = new Set(["clickhouse", "trino"]);

/** A `YYYY-MM-DD` input, exactly — the shape the page's date controls hold. */
function isDay(value: string | null): value is string {
  return value !== null && /^\d{4}-\d{2}-\d{2}$/.test(value);
}

/** The state the page starts from when the address says nothing. */
export const EMPTY_HISTORY_ADDRESS: HistoryAddress = {
  status: "all",
  engine: "all",
  minDuration: "",
  table: "",
  from: "",
  to: "",
  page: 1,
};

/**
 * Reads the history page's own filter values from a query string. A value the
 * page does not know — a status outside the four, a date that is not
 * `YYYY-MM-DD`, a page below 1 or a non-negative-whole-number check that
 * fails — is ignored, not thrown at the server (D17).
 */
export function readHistoryAddress(search: string): HistoryAddress {
  const params = new URLSearchParams(search);
  const status = params.get("status");
  const engine = params.get("engine");
  const min = params.get("minDurationMs");
  const from = params.get("from");
  const to = params.get("to");
  const page = params.get("page");
  const parsedPage = page !== null && /^\d+$/.test(page) ? Number(page) : 1;
  return {
    status: status !== null && ADDRESS_STATUSES.has(status) ? status : "all",
    engine: engine !== null && ADDRESS_ENGINES.has(engine) ? engine : "all",
    minDuration: min !== null && /^\d+$/.test(min) ? min : "",
    table: params.get("table") ?? "",
    from: isDay(from) ? from : "",
    to: isDay(to) ? to : "",
    page: Number.isSafeInteger(parsedPage) && parsedPage >= 1 ? parsedPage : 1,
  };
}

/**
 * Writes only the filter values that are set, in the page's own address: no
 * leading `?`, and no `page` for the first one, so an untouched page writes
 * an empty string and adds no noise to the address (D17).
 */
export function historyAddress(state: HistoryAddress): string {
  const params = new URLSearchParams();
  if (state.status !== "all") params.set("status", state.status);
  if (state.engine !== "all") params.set("engine", state.engine);
  if (state.minDuration !== "") params.set("minDurationMs", state.minDuration);
  if (state.table !== "") params.set("table", state.table);
  if (state.from !== "") params.set("from", state.from);
  if (state.to !== "") params.set("to", state.to);
  if (state.page > 1) params.set("page", String(state.page));
  return params.toString();
}
