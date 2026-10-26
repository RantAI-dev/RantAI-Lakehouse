import type { Board } from "../contracts/dashboards"
import type {
  ChartLineage,
  CreateBoardInput,
  DashboardFolder,
  DashboardService,
  JoinSuggestions,
  RestoredItem,
  RevertResult,
  SaveSqlSourceInput,
  ShortcutItem,
  Shortcuts,
  SqlSource,
  SqlSourcePreview,
  TrashList,
  UpdateBoardInput,
  VersionHistory,
} from "../contracts/dashboards"
import type {
  EmbedInfo, GranteeMatches, ItemAccess, MoveEffect, PublicLink, PublicLinkState,
} from "../contracts/dashboards"
import { apiFetch } from "../http"
import { ServiceError, type ServiceErrorCode } from "../errors"

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

/**
 * The service error code for a status: 401 and 403 are a permission, 404 is
 * "not found", a 5xx is the service being unavailable, the rest is a request
 * the server refused. One rule for every call here.
 */
function codeFor(status: number): ServiceErrorCode {
  if (status === 401 || status === 403) return "permission_denied"
  if (status === 404) return "not_found"
  if (status >= 500) return "unavailable"
  return "invalid_request"
}

async function readJson<T>(res: Response): Promise<T> {
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    throw new ServiceError(codeFor(res.status), json?.error ?? `Gagal (${res.status})`)
  }
  return json as T
}

/**
 * A dashboard write's answer (BI-18 part C review fix (SHOULD-FIX) R19): the
 * answer is checked, and a refusal throws with the server's sentence. A
 * refusal without one says the status. The `code:` prefix the server may put
 * in front of its sentence is for the log: the page shows the readable
 * sentence (`readableErrorMessage`).
 */
async function checkedWrite<T>(res: Response): Promise<T> {
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    throw new ServiceError(
      codeFor(res.status),
      json?.error ?? `The change was refused (${res.status}).`
    )
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
    throw new ServiceError(codeFor(res.status), json?.error ?? fallbackMessage)
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
  const json = await checkedWrite<{ board?: Board }>(res)
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
    await checkedWrite<unknown>(res)
  },
  /**
   * Save the tile layout of a dashboard (`PUT /api/dashboard/boards`). The
   * console's own write; it needs `dashboard:write`.
   */
  async saveBoardLayout(id, layout, signal) {
    const res = await apiFetch("/api/dashboard/boards", {
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify({ id, layout }),
      signal,
    })
    await checkedWrite<unknown>(res)
  },
  /**
   * Delete one saved chart card (`DELETE /api/dashboard/specs`). It needs
   * `dashboard:write`.
   */
  async deleteChart(id, signal) {
    const res = await apiFetch(`/api/dashboard/specs?id=${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal,
    })
    await checkedWrite<unknown>(res)
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
  async joinSuggestions(left, right, signal) {
    return request<JoinSuggestions>(
      `/api/dashboard/join-suggestions?left=${encodeURIComponent(left)}&right=${encodeURIComponent(right)}`,
      { cache: "no-store", signal },
      "Join suggestions could not be loaded"
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

  // SEC-12. The server's messages for these are fixed and ours, so they are
  // shown as they come (`request` carries `error` into the `ServiceError`).
  async getEmbedInfo(board, signal, chart) {
    const json = await request<Partial<EmbedInfo>>(
      `/api/dashboard/embed-info?board=${encodeURIComponent(board)}${chart ? `&chart=${encodeURIComponent(chart)}` : ""}`,
      { cache: "no-store", signal },
      "Embedding state could not be loaded"
    )
    return {
      enabled: Boolean(json.enabled),
      supported: json.supported !== false,
      reason: json.reason,
      sampleToken: json.sampleToken,
      maxLifetimeSeconds: json.maxLifetimeSeconds ?? 86_400,
      revokedBefore: json.revokedBefore ?? null,
      allowedOrigins: json.allowedOrigins ?? [],
      filterModes: json.filterModes ?? {},
      filterColumns: json.filterColumns ?? [],
      hiddenOnlyColumns: json.hiddenOnlyColumns ?? [],
    }
  },
  async setEmbedFilterModes(board, modes, signal) {
    const res = await apiFetch("/api/dashboard/embed-settings", {
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify({ board, filterModes: modes }),
      signal,
    })
    await checkedWrite<{ ok: boolean }>(res)
  },
  async listBoardCharts(board, signal) {
    const json = await request<{ charts?: { id: string; title?: string; board?: string }[] }>(
      "/api/dashboard/specs",
      { cache: "no-store", signal },
      "The charts could not be loaded"
    )
    return (json.charts ?? [])
      .filter((c) => c.board === board)
      .map((c) => ({ id: c.id, title: c.title ?? c.id }))
  },
  // BI-19 part A. The server's sentences are ours, so they are shown as they come.
  async getPublicLink(board, signal) {
    const json = await request<Partial<PublicLinkState>>(
      `/api/dashboard/public-link?board=${encodeURIComponent(board)}`,
      { cache: "no-store", signal },
      "The public link could not be loaded"
    )
    return { enabled: json.enabled !== false, link: json.link ?? null }
  },
  async savePublicLink(input, signal) {
    const res = await apiFetch("/api/dashboard/public-link", {
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify(input),
      signal,
    })
    const json = await checkedWrite<{ link: PublicLink }>(res)
    return json.link
  },
  async endPublicLink(board, signal) {
    const res = await apiFetch(`/api/dashboard/public-link?board=${encodeURIComponent(board)}`, {
      method: "DELETE",
      signal,
    })
    await checkedWrite<{ ok: boolean }>(res)
  },
  // BI-19 part B. The server's sentences are ours, so they are shown as they come.
  async getAccess(kind, id, signal) {
    return await request<ItemAccess>(
      `/api/dashboard/access?kind=${encodeURIComponent(kind)}&id=${encodeURIComponent(id)}`,
      { cache: "no-store", signal },
      "The permissions could not be loaded"
    )
  },
  async saveAccess(input, signal) {
    const res = await apiFetch("/api/dashboard/access", {
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify(input),
      signal,
    })
    return await checkedWrite<ItemAccess>(res)
  },
  async findGrantees(query, signal) {
    const json = await request<{ items?: GranteeMatches["people"]; roles?: GranteeMatches["roles"] }>(
      `/api/reports/recipients?roles=1&q=${encodeURIComponent(query)}`,
      { cache: "no-store", signal },
      "People could not be searched"
    )
    return { people: json.items ?? [], roles: json.roles ?? [] }
  },
  async previewMove(board, folderId, signal) {
    return await request<MoveEffect>(
      `/api/dashboard/access-preview?board=${encodeURIComponent(board)}&folder=${encodeURIComponent(folderId)}`,
      { cache: "no-store", signal },
      "The effect of the move could not be worked out"
    )
  },
  async setEmbedOrigins(board, origins, signal) {
    const json = await request<{ embedOrigins?: string[] }>(
      "/api/dashboard/boards",
      {
        method: "PUT",
        headers: JSON_HEADERS,
        body: JSON.stringify({ id: board, embedOrigins: origins }),
        signal,
      },
      "Allowed sites could not be saved"
    )
    return json.embedOrigins ?? origins
  },
  async revokeAllEmbedTokens(board, signal) {
    const json = await request<{ embedRevokedBefore?: number }>(
      "/api/dashboard/boards",
      {
        method: "PUT",
        headers: JSON_HEADERS,
        body: JSON.stringify({ id: board, embedRevokeAll: true }),
        signal,
      },
      "Embed tokens could not be withdrawn"
    )
    return json.embedRevokedBefore ?? 0
  },
  async revokeEmbedToken(board, token, signal) {
    const json = await request<{ withdrawn?: string }>(
      "/api/dashboard/embed-revoke",
      {
        method: "POST",
        headers: JSON_HEADERS,
        body: JSON.stringify({ board, token }),
        signal,
      },
      "The token could not be withdrawn"
    )
    return json.withdrawn ?? ""
  },

  // BI-18 part C, part 2. The server's sentences are fixed and ours (a revert
  // that cannot be done says which chart and why), so they are shown as they come.
  async listVersions(board, signal) {
    return request<VersionHistory>(
      `/api/dashboard/versions?board=${encodeURIComponent(board)}`,
      { cache: "no-store", signal },
      "Version history could not be loaded"
    )
  },
  async revertToVersion(board, version, signal) {
    return request<RevertResult>(
      "/api/dashboard/versions/revert",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify({ board, version }), signal },
      "That version could not be restored"
    )
  },
  async listTrash(signal) {
    return request<TrashList>(
      "/api/dashboard/trash",
      { cache: "no-store", signal },
      "The trash could not be loaded"
    )
  },
  async restoreFromTrash(kind, id, signal) {
    const json = await request<{ restored: RestoredItem }>(
      "/api/dashboard/trash/restore",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify({ kind, id }), signal },
      "It could not be restored"
    )
    return json.restored
  },
  async ensurePersonalSpace(signal) {
    const json = await request<{ folder: DashboardFolder }>(
      "/api/dashboard/personal",
      { method: "POST", signal },
      "The personal space could not be opened"
    )
    return json.folder
  },
  async shortcuts(signal) {
    const json = await request<Partial<Shortcuts>>(
      "/api/dashboard/shortcuts",
      { cache: "no-store", signal },
      "Favourites could not be loaded"
    )
    return { favourites: json.favourites ?? [], recent: json.recent ?? [] }
  },
  async setFavourite(kind, id, on, signal) {
    await request<{ ok: true }>(
      "/api/dashboard/favourites",
      { method: "PUT", headers: JSON_HEADERS, body: JSON.stringify({ kind, id, on }), signal },
      "The favourite could not be saved"
    )
  },
  async touchRecent(kind, id, signal) {
    await request<{ ok: true }>(
      "/api/dashboard/recent",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify({ kind, id }), signal },
      "Recent items could not be saved"
    )
  },
  chartLineage(id, signal) {
    return request<ChartLineage>(
      `/api/dashboard/charts/${encodeURIComponent(id)}/lineage`,
      { cache: "no-store", signal },
      "Lineage is unavailable"
    )
  },
  async searchItems(text, signal): Promise<ShortcutItem[]> {
    const json = await request<{ hits?: ShortcutItem[] }>(
      `/api/dashboard/search?q=${encodeURIComponent(text)}`,
      { cache: "no-store", signal },
      "Search is unavailable"
    )
    return json.hits ?? []
  },
}
