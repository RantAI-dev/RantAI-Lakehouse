import type {
  GoldService,
  GoldMart,
  GoldExportResult,
  GoldReadBack,
  GoldExportRun,
  GoldConsumers,
} from "../contracts/gold"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * GoldService is real — every method calls `lakehouse-api`'s `/api/gold/*`
 * routes (and `/api/dashboard/fields` for the mart list). No mock ever
 * existed for this domain: ADR 0010's Gold export was Rust-only from the
 * start, so there is nothing to delegate away from.
 */

async function getJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json()
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? `Request failed (${res.status})`)
  return json as T
}

async function postJson<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { method: "POST", signal })
  const json = await res.json()
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request"
    throw new ServiceError(kind, json?.error ?? `Request failed (${res.status})`)
  }
  return json as T
}

export const goldService: GoldService = {
  async listMarts(signal) {
    return (await getJson<{ marts: GoldMart[] }>("/api/dashboard/fields", { signal })).marts
  },
  async getLastExport(mart, signal) {
    return getJson<GoldReadBack>(`/api/gold/export/${encodeURIComponent(mart)}`, { signal })
  },
  async triggerExport(mart, signal) {
    return postJson<GoldExportResult>(`/api/gold/export/${encodeURIComponent(mart)}`, signal)
  },
  async listExportRuns(mart, signal) {
    return (
      await getJson<{ mart: string; runs: GoldExportRun[] }>(
        `/api/gold/exports?mart=${encodeURIComponent(mart)}`,
        { signal }
      )
    ).runs
  },
  async getConsumers(mart, signal) {
    return getJson<GoldConsumers>(`/api/gold/export/${encodeURIComponent(mart)}/consumers`, { signal })
  },
}
