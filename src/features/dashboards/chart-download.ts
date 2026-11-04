import { normalizeFilters } from "@/lib/dashboard-filter-state";
import type { FilterDef } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";
import { fileNameOf, refusalOf, saveBlob, type TableExportResult } from "./table-export";

/**
 * A chart's data as an Excel or CSV file (`BI-1`, `GET /api/dashboard/chart-export`)
 * and a dashboard as a PDF (`GET /api/dashboard/export/pdf`). Both send what the
 * page itself sent to read the board, so the file holds what is on screen: the
 * filters in force, the parameter values, the grain switch.
 */

export type DataFormat = "xlsx" | "csv";
export type DataValues = "formatted" | "raw";
export type DataChoice = { format: DataFormat; values: DataValues };

/** The deployment cannot make a PDF (feature decision 6); the browser's print stays. */
export const PDF_NOT_SET_UP = "PDF export is not set up on this deployment";

/** What the page sent to read the dashboard, to be sent again with a download. */
export type ReadState = {
  readonly filters: readonly FilterDef[];
  /** `paramsQuery(...)` text, or none. */
  readonly params?: string | null;
  /** `grainQuery(...)` text, or none. */
  readonly grain?: string | null;
};

function readQuery(q: URLSearchParams, state: ReadState): void {
  const active = normalizeFilters([...state.filters]);
  if (active.length) q.set("filters", JSON.stringify(active));
  if (state.params) q.set("params", state.params);
  if (state.grain) q.set("grain", state.grain);
}

/** The query string of `chart-export`: the tile, the file, the page's reads, a raw table's chosen sort. */
export function chartExportQuery(
  chart: string,
  choice: DataChoice,
  state: ReadState,
  sort?: { column: string; dir: "asc" | "desc" } | null,
): URLSearchParams {
  const q = new URLSearchParams({ chart, format: choice.format, values: choice.values });
  readQuery(q, state);
  if (sort) {
    q.set("sortColumn", sort.column);
    q.set("sortDir", sort.dir);
  }
  return q;
}

/** Download a tile's whole result; throws an `Error` carrying the server's sentence on a refusal. */
export async function downloadChartData(
  chart: string,
  choice: DataChoice,
  state: ReadState,
  sort?: { column: string; dir: "asc" | "desc" } | null,
): Promise<TableExportResult> {
  const res = await apiFetch(`/api/dashboard/chart-export?${chartExportQuery(chart, choice, state, sort).toString()}`, { cache: "no-store" });
  if (!res.ok) throw new Error(await refusalOf(res, "The download failed."));
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

/** The deployment cannot make an image (`BI-19` part D, plan D13); the server's sentence. */
export const IMAGES_NOT_SET_UP = "Images are not set up on this deployment.";

/**
 * Download a chart as a PNG (`BI-19` part D): the query of the data download
 * with `format=png`, so the picture shows the filters, parameters and grain
 * the page has on. Throws an `Error` carrying the server's sentence on a refusal.
 */
export async function downloadChartPng(chart: string, state: ReadState): Promise<string> {
  const q = new URLSearchParams({ chart, format: "png" });
  readQuery(q, state);
  const res = await apiFetch(`/api/dashboard/chart-export?${q.toString()}`, { cache: "no-store" });
  if (!res.ok) throw new Error(await refusalOf(res, "The image could not be made."));
  const blob = await res.blob();
  const fileName = fileNameOf(res.headers.get("content-disposition")).replace(/\.csv$/, ".png");
  saveBlob(blob, fileName);
  return fileName;
}

/**
 * Download one dashboard as a file (`BI-19` part D, plan D14): its settings,
 * charts and the tables they read, to be imported on this or another
 * deployment. Throws an `Error` carrying the server's sentence on a refusal.
 */
export async function downloadDashboardFile(board: string): Promise<string> {
  const q = new URLSearchParams({ format: "file", board });
  const res = await apiFetch(`/api/dashboard/export?${q.toString()}`, { cache: "no-store" });
  if (!res.ok) throw new Error(await refusalOf(res, "The dashboard file could not be made."));
  const blob = await res.blob();
  const fileName = fileNameOf(res.headers.get("content-disposition")).replace(/\.csv$/, ".rantai-dashboard.json");
  saveBlob(blob, fileName);
  return fileName;
}

export type PdfChoice = { paper: "a4" | "letter"; orientation: "portrait" | "landscape" };

/** The query string of `export/pdf`. */
export function pdfExportQuery(board: string, choice: PdfChoice, state: ReadState): URLSearchParams {
  const q = new URLSearchParams({ board, paper: choice.paper, orientation: choice.orientation });
  readQuery(q, state);
  return q;
}

/** Download the dashboard as a PDF; throws an `Error` carrying the server's sentence on a refusal. */
export async function downloadDashboardPdf(board: string, choice: PdfChoice, state: ReadState): Promise<string> {
  const res = await apiFetch(`/api/dashboard/export/pdf?${pdfExportQuery(board, choice, state).toString()}`, { cache: "no-store" });
  if (!res.ok) throw new Error(await refusalOf(res, "The PDF could not be made."));
  const blob = await res.blob();
  const fileName = fileNameOf(res.headers.get("content-disposition")).replace(/\.csv$/, ".pdf");
  saveBlob(blob, fileName);
  return fileName;
}

/**
 * Which download a tile's menu offers (`BI-1`): a chart of the board downloads
 * its whole result from the server (`"data"`), a built-in tile has no stored
 * definition to read and keeps the rows on screen as CSV (`"csv"`), and a tile
 * with no rows, or a viewer whose every role is switched off from downloading,
 * gets none.
 */
export function downloadMenuKind(tile: { own: boolean; hasRows: boolean }, canDownload: boolean): "data" | "csv" | null {
  if (!canDownload || !tile.hasRows) return null;
  return tile.own ? "data" : "csv";
}
