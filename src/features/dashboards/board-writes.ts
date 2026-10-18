import { readableErrorMessage } from "@/lib/notify-message";
import type { LayoutMap } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";

/**
 * The dashboard page's own writes (create, duplicate, rename, delete a board,
 * delete a chart, save a layout), each of which checks the answer: BI-18 part C
 * review fix (SHOULD-FIX) R19. Before, a refused delete still said "Dashboard
 * moved to the trash" and a refused rename said nothing. A refusal throws an
 * `Error` carrying the server's sentence (fixed, non-leaking text; the
 * `code:` prefix is dropped, R20), which the caller shows.
 */

async function checked(res: Response): Promise<Record<string, unknown>> {
  const json = (await res.json().catch(() => null)) as Record<string, unknown> | null;
  if (!res.ok) {
    const said = typeof json?.error === "string" && json.error.trim() ? json.error : `The change was refused (${res.status}).`;
    throw new Error(readableErrorMessage(said));
  }
  return json ?? {};
}

const JSON_HEADERS = { "Content-Type": "application/json" };

async function boardRequest(method: "POST" | "PUT", body: Record<string, unknown>): Promise<{ board?: { id: string } }> {
  const res = await apiFetch("/api/dashboard/boards", { method, headers: JSON_HEADERS, body: JSON.stringify(body) });
  return (await checked(res)) as { board?: { id: string } };
}

export const createBoard = (name: string) => boardRequest("POST", { name });
export const duplicateBoard = (id: string) => boardRequest("POST", { duplicate: id });
export const renameBoard = (id: string, name: string) => boardRequest("PUT", { id, name });
export const saveBoardLayout = (id: string, layout: LayoutMap) => boardRequest("PUT", { id, layout });

export async function deleteBoard(id: string): Promise<void> {
  await checked(await apiFetch(`/api/dashboard/boards?id=${encodeURIComponent(id)}`, { method: "DELETE" }));
}

export async function deleteChart(id: string): Promise<void> {
  await checked(await apiFetch(`/api/dashboard/specs?id=${encodeURIComponent(id)}`, { method: "DELETE" }));
}
