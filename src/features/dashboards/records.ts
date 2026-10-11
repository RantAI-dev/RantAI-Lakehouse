import { normalizeFilters } from "@/lib/dashboard-filter-state";
import type { FilterDef, FilterSkip } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";

/**
 * Rows per page of the records list. The API clamps `limit` to the same 50
 * (`RECORDS_PAGE_MAX`); the owner confirms the number at QA (BI-18·B).
 */
export const RECORDS_PAGE_SIZE = 50;

/** What to list: the rows behind one value, or behind a whole tile. */
export type RecordsRequest = {
  /** Shown in the dialog title: the clicked value, or the tile's title. */
  title: string;
  /** A mart, or a SQL source id instead of it. */
  mart: string;
  sqlSource?: string;
  /** Together or not at all: neither lists the whole tile. */
  column?: string;
  value?: string;
  /** With `column` and `value`: `value` is a bucket of a chart grouped by this grain (BI-9), and the rows are those that fall in it. */
  grain?: string;
  /** The dashboard's active filters, so the list agrees with the number clicked. */
  filters: FilterDef[];
  /**
   * A raw table (BI-16 part A): list these columns of the whole tile, in this
   * order, sorted by `sortColumn`. Not combined with `column` and `value`.
   */
  columns?: readonly string[];
  sortColumn?: string;
  sortDir?: "asc" | "desc";
};

export type RecordsPage = {
  columns: string[];
  rows: Record<string, unknown>[];
  total: number;
  limit: number;
  offset: number;
  filtersSkipped: FilterSkip[];
};

/** The query string of `GET /api/dashboard/records` for one page. */
export function recordsQuery(req: RecordsRequest, offset: number): URLSearchParams {
  const q = new URLSearchParams();
  if (req.sqlSource) q.set("sqlSource", req.sqlSource);
  else q.set("mart", req.mart);
  if (req.column !== undefined && req.value !== undefined) {
    q.set("column", req.column);
    q.set("value", req.value);
    if (req.grain) q.set("grain", req.grain);
  }
  if (req.columns?.length) {
    q.set("columns", req.columns.join(","));
    if (req.sortColumn) {
      q.set("sortColumn", req.sortColumn);
      if (req.sortDir) q.set("sortDir", req.sortDir);
    }
  }
  q.set("limit", String(RECORDS_PAGE_SIZE));
  q.set("offset", String(offset));
  const active = normalizeFilters(req.filters);
  if (active.length) q.set("filters", JSON.stringify(active));
  return q;
}

/** `1–50 of 120`; `0 of 0` for an empty list. */
export function pageRange(page: Pick<RecordsPage, "rows" | "total" | "offset">): string {
  if (page.rows.length === 0) return `0 of ${page.total.toLocaleString("en-US")}`;
  const from = page.offset + 1;
  const to = page.offset + page.rows.length;
  return `${from.toLocaleString("en-US")}–${to.toLocaleString("en-US")} of ${page.total.toLocaleString("en-US")}`;
}

export function hasNextPage(page: Pick<RecordsPage, "rows" | "total" | "offset">): boolean {
  return page.offset + page.rows.length < page.total;
}

/**
 * The server's fixed failure sentence ends with ` Reference: <id>` (SEC-11);
 * split so the reference can be shown small and selectable
 * (`ErrorWithReference`). A message without one is returned whole.
 */
export function splitReference(message: string): { message: string; errorId?: string } {
  const m = /^([\s\S]*\S)\s+Reference:\s+([A-Za-z0-9-]+)\s*$/.exec(message);
  return m ? { message: m[1], errorId: m[2] } : { message };
}

/**
 * One page of the records behind a value (or a tile). Throws an `Error`
 * carrying the server's own sentence on a failure; never the upstream text,
 * which the server does not send (SEC-11).
 */
export async function fetchRecordsPage(req: RecordsRequest, offset: number, signal?: AbortSignal): Promise<RecordsPage> {
  const res = await apiFetch(`/api/dashboard/records?${recordsQuery(req, offset).toString()}`, { cache: "no-store", signal });
  const json = (await res.json().catch(() => null)) as Record<string, unknown> | null;
  if (!res.ok) throw new Error(typeof json?.error === "string" ? json.error : "The records could not be loaded.");
  if (json?.supported === false) throw new Error(typeof json.message === "string" ? json.message : "These records cannot be listed.");
  return {
    columns: Array.isArray(json?.columns) ? (json.columns as string[]) : [],
    rows: Array.isArray(json?.rows) ? (json.rows as Record<string, unknown>[]) : [],
    total: typeof json?.total === "number" ? json.total : 0,
    limit: typeof json?.limit === "number" ? json.limit : RECORDS_PAGE_SIZE,
    offset: typeof json?.offset === "number" ? json.offset : offset,
    filtersSkipped: Array.isArray(json?.filtersSkipped) ? (json.filtersSkipped as FilterSkip[]) : [],
  };
}
