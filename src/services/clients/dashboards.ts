import type { Board } from "./bi-store"
import type {
  CreateBoardInput,
  DashboardFolder,
  DashboardService,
  SaveSqlSourceInput,
  SqlSource,
  SqlSourcePreview,
  UpdateBoardInput,
} from "../contracts/dashboards"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * DashboardService NYATA — board disimpan di ClickHouse
 * (`console.bi_board`), lewat route `/api/dashboard/boards`; SQL sources
 * dan folders lewat `/api/dashboard/sources` dan `/api/dashboard/folders`.
 *
 * Perlu diketahui saat membaca hasil `listBoards`: board bawaan (`default`)
 * bukan baris tabel. Ia disintesis server dan hanya membawa
 * `id`/`name`/`layout`/`chartCount`/`builtin` — tanpa timestamp, deskripsi,
 * atau token share. Ia juga tidak bisa di-rename maupun dihapus. Pemanggil
 * harus bercabang pada `board.builtin`, bukan berasumsi setiap field ada.
 *
 * Every method throws a `ServiceError` carrying the server's `{ error }`
 * text on a non-OK response — that text is already fixed and non-leaking on
 * the server side, so it is safe to show. 409 (a folder still holding
 * something, or a source still used by charts) maps to `invalid_request`
 * with the server's message naming what is blocking it.
 */

async function readJson<T>(res: Response): Promise<T> {
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    const kind =
      res.status === 401 || res.status === 403
        ? "permission_denied"
        : res.status === 404
          ? "not_found"
          : res.status >= 500
            ? "unavailable"
            : "invalid_request"
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`)
  }
  return json as T
}

async function request<T>(
  url: string,
  init: RequestInit | undefined,
  fallbackMessage: string
): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    const kind =
      res.status === 401 || res.status === 403
        ? "permission_denied"
        : res.status === 404
          ? "not_found"
          : res.status >= 500
            ? "unavailable"
            : "invalid_request"
    throw new ServiceError(kind, json?.error ?? fallbackMessage)
  }
  return json as T
}

async function writeBoard(
  method: "POST" | "PUT",
  body: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<Board> {
  const res = await apiFetch("/api/dashboard/boards", {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  })
  const json = await readJson<{ board?: Board }>(res)
  // PUT hanya mengembalikan `{ok:true}`; POST mengembalikan board barunya.
  return json.board ?? ({ id: String(body.id ?? ""), name: "" } as Board)
}

const JSON_HEADERS = { "Content-Type": "application/json" }

export const clickhouseDashboardService: DashboardService = {
  async listBoards(signal) {
    const res = await apiFetch("/api/dashboard/boards", { signal })
    const json = await readJson<{ boards?: Board[] }>(res)
    return json.boards ?? []
  },
  createBoard(input: CreateBoardInput, signal) {
    return writeBoard("POST", { name: input.name, description: input.description }, signal)
  },
  duplicateBoard(id, signal) {
    return writeBoard("POST", { duplicate: id }, signal)
  },
  async updateBoard(input: UpdateBoardInput, signal) {
    await writeBoard(
      "PUT",
      { id: input.id, name: input.name, description: input.description },
      signal,
    )
  },
  async deleteBoard(id, signal) {
    const res = await apiFetch(`/api/dashboard/boards?id=${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal,
    })
    await readJson<unknown>(res)
  },

  async listSqlSources(signal) {
    const json = await request<{ sources: SqlSource[] }>(
      "/api/dashboard/sources",
      { cache: "no-store", signal },
      "SQL sources could not be loaded"
    )
    return json.sources
  },
  async createSqlSource(input: SaveSqlSourceInput, signal) {
    const json = await request<{ ok: true; source: SqlSource }>(
      "/api/dashboard/sources",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "SQL source could not be saved"
    )
    return json.source
  },
  async updateSqlSource(input, signal) {
    const json = await request<{ ok: true; source: SqlSource }>(
      "/api/dashboard/sources",
      { method: "PUT", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "SQL source could not be saved"
    )
    return json.source
  },
  async deleteSqlSource(id, signal) {
    await request<{ ok: true }>(
      `/api/dashboard/sources?id=${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
      "SQL source could not be deleted"
    )
  },
  async previewSqlSource(sql, chart, signal): Promise<SqlSourcePreview> {
    return request<SqlSourcePreview>(
      "/api/dashboard/sources/preview",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify(chart ? { sql, chart } : { sql }), signal },
      "SQL source preview failed"
    )
  },
  async listFolders(signal) {
    const json = await request<{ folders: DashboardFolder[] }>(
      "/api/dashboard/folders",
      { cache: "no-store", signal },
      "Folders could not be loaded"
    )
    return json.folders
  },
  async createFolder(input, signal) {
    const json = await request<{ ok: true; folder: DashboardFolder }>(
      "/api/dashboard/folders",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "Folder could not be created"
    )
    return json.folder
  },
  async updateFolder(input, signal) {
    const json = await request<{ ok: true; folder: DashboardFolder }>(
      "/api/dashboard/folders",
      { method: "PUT", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "Folder could not be saved"
    )
    return json.folder
  },
  async deleteFolder(id, signal) {
    await request<{ ok: true }>(
      `/api/dashboard/folders?id=${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
      "Folder could not be deleted"
    )
  },
  async moveBoard(boardId, folderId, signal) {
    await request<{ ok: true }>(
      "/api/dashboard/boards",
      {
        method: "PUT",
        headers: JSON_HEADERS,
        body: JSON.stringify({ id: boardId, folderId }),
        signal,
      },
      "Dashboard could not be moved"
    )
  },
}
