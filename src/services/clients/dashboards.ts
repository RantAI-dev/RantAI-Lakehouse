import type {
  DashboardService,
  SaveSqlSourceInput,
  SqlSource,
  SqlSourcePreview,
} from "../contracts/dashboards"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * DashboardService over `/api/dashboard/sources` (SQL sources). New code in
 * the dashboards area goes through this client; the older dashboard page
 * still calls `apiFetch` itself and is not migrated here.
 *
 * Every method throws a `ServiceError` carrying the server's `{ error }`
 * text on a non-OK response — that text is already fixed and non-leaking on
 * the server side (`dashboard_sources.rs`), so it is safe to show. 409 (a
 * source still used by charts) maps to `invalid_request` with the server's
 * message naming those charts.
 */
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

const JSON_HEADERS = { "Content-Type": "application/json" }

export const clickhouseDashboardService: DashboardService = {
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
  async previewSqlSource(sql, signal): Promise<SqlSourcePreview> {
    return request<SqlSourcePreview>(
      "/api/dashboard/sources/preview",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify({ sql }), signal },
      "SQL source preview failed"
    )
  },
}
