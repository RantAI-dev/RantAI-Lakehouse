import { apiFetch } from "@/services/http";
import { refusalOf } from "./table-export";

/**
 * Importing a dashboard file (`BI-19` part D, plan D14): the two calls and the
 * client-side limits. The server checks everything again; the limits here only
 * save a person from posting a file that is certain to be refused.
 */

/** The largest file the server reads. */
export const MAX_FILE_BYTES = 1_000_000;

/** What `POST /api/dashboard/import/preview` answers. */
export type ImportPreview = {
  name: string;
  charts: { id: string; title: string; kind: string; tables: string[]; readsSqlSource: boolean }[];
  tables: { name: string; columns: string[]; suggested: string | null; missingColumns: string[] }[];
  readableTables: string[];
  canImportSql: boolean;
};

/** What `POST /api/dashboard/import` answers. */
export type ImportResult = {
  board: { id: string; name: string };
  imported: { id: string; title: string }[];
  leftOut: { title: string; reason: string }[];
};

/** Read a chosen file's text; a file that is too large or not JSON is refused with a sentence. */
export async function readDashboardFile(file: File): Promise<unknown> {
  if (file.size > MAX_FILE_BYTES) throw new Error("That file is larger than 1 MB.");
  try {
    return JSON.parse(await file.text()) as unknown;
  } catch {
    throw new Error("That is not a dashboard file.");
  }
}

async function post<T>(url: string, body: unknown, fallback: string): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    cache: "no-store",
  });
  if (!res.ok) throw new Error(await refusalOf(res, fallback));
  return (await res.json()) as T;
}

/** The tables of the file and the table here each one most likely means. */
export function previewImport(file: unknown): Promise<ImportPreview> {
  return post("/api/dashboard/import/preview", { file }, "The file could not be read.");
}

/** Make a new dashboard from the file; `mapping` is file table name to the table here. */
export function importDashboard(file: unknown, mapping: Record<string, string>, folderId: string): Promise<ImportResult> {
  return post("/api/dashboard/import", { file, mapping, folderId }, "The dashboard could not be imported.");
}

/** The mapping a preview starts from: each table's suggestion, and no entry where there is none. */
export function initialMapping(preview: ImportPreview): Record<string, string> {
  const out: Record<string, string> = {};
  for (const t of preview.tables) if (t.suggested) out[t.name] = t.suggested;
  return out;
}
