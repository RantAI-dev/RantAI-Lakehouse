import { normalizeFilters } from "@/lib/dashboard-filter-state";
import type { FilterDef } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";
import { fileNameOf, refusalOf, saveBlob } from "@/lib/browser-file";

// The three helpers every download shares now live in `@/lib/browser-file`,
// because a service client needs them too and a client may not import from
// `src/features`. They are re-exported here, so the dashboard's own
// downloads keep reading them from this module.
export { fileNameOf, refusalOf, saveBlob };

/** What the server says about the file it sent (`X-Export-*`), so a cut is never silent. */
export type TableExportResult = { rows: number; cut: boolean; cap: number; fileName: string };

/** The query string of `GET /api/dashboard/table-export`: the tile, the dashboard's filters, the viewer's sort. */
export function tableExportQuery(chart: string, filters: readonly FilterDef[], sort?: { column: string; dir: "asc" | "desc" } | null, params?: string | null): URLSearchParams {
  const q = new URLSearchParams({ chart });
  if (params) q.set("params", params);
  const active = normalizeFilters([...filters]);
  if (active.length) q.set("filters", JSON.stringify(active));
  if (sort) {
    q.set("sortColumn", sort.column);
    q.set("sortDir", sort.dir);
  }
  return q;
}

/**
 * Download every row of a raw table's result (`BI-16` part A, T8). The server
 * applies the role rewrite, the filters and the sort, and writes at most
 * `cap` rows; the result says how many it wrote and whether it was cut.
 * Throws an `Error` carrying the server's own sentence on a failure.
 */
export async function downloadTableExport(
  chart: string,
  filters: readonly FilterDef[],
  sort?: { column: string; dir: "asc" | "desc" } | null,
  params?: string | null,
): Promise<TableExportResult> {
  const res = await apiFetch(`/api/dashboard/table-export?${tableExportQuery(chart, filters, sort, params).toString()}`, { cache: "no-store" });
  if (!res.ok) throw new Error(await refusalOf(res, "The export failed."));
  const blob = await res.blob();
  const fileName = fileNameOf(res.headers.get("content-disposition"));
  saveBlob(blob, fileName);
  return {
    rows: Number(res.headers.get("x-export-rows") ?? 0),
    cut: res.headers.get("x-export-cut") === "true",
    cap: Number(res.headers.get("x-export-cap") ?? 0),
    fileName,
  };
}

/** The sentence that tells the person what the file holds; a cut is said, with its number. */
export function exportNotice(r: Pick<TableExportResult, "rows" | "cut" | "cap">): { message: string; description?: string } {
  const n = r.rows.toLocaleString("en-US");
  return r.cut
    ? { message: `Exported the first ${n} rows`, description: `The result has more rows than the ${r.cap.toLocaleString("en-US")} an export holds, so the file was cut there. Add a filter to narrow it.` }
    : { message: `Exported ${n} rows` };
}
