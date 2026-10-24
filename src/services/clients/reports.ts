import type {
  ReportDelivery,
  ReportList,
  ReportPerson,
  ReportService,
  ReportSubscription,
} from "../contracts/reports"
import { ServiceError } from "../errors"
import { apiFetch } from "../http"

/**
 * ReportService over `/api/reports/...`. A non-OK answer becomes a
 * `ServiceError` carrying the route's own fixed sentence (it never holds
 * upstream text), which the dialog shows as it is.
 */
async function call<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
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
    // The API writes a refusal as `permission_denied: <sentence>`; the sentence is what a person reads.
    const text = typeof json?.error === "string" ? json.error.replace(/^permission_denied:\s*/, "") : fallback
    throw new ServiceError(kind, text, res.status)
  }
  return json as T
}

const jsonInit = (method: string, body: unknown, signal?: AbortSignal): RequestInit => ({
  method,
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
  signal,
})

const SUBS = "/api/reports/subscriptions"

export const reportService: ReportService = {
  list: (board, signal) =>
    call<ReportList>(`${SUBS}?board=${encodeURIComponent(board)}`, { signal }, "The subscriptions could not be loaded."),
  create: (input, signal) =>
    call<ReportSubscription>(SUBS, jsonInit("POST", input, signal), "The subscription could not be saved."),
  update: (id, input, signal) =>
    call<ReportSubscription>(`${SUBS}/${encodeURIComponent(id)}`, jsonInit("PUT", input, signal), "The subscription could not be saved."),
  setPaused: (id, paused, signal) =>
    call<ReportSubscription>(`${SUBS}/${encodeURIComponent(id)}`, jsonInit("PUT", { paused }, signal), "The subscription could not be changed."),
  async remove(id, signal) {
    await call<{ deleted: boolean }>(`${SUBS}/${encodeURIComponent(id)}`, { method: "DELETE", signal }, "The subscription could not be deleted.")
  },
  sendNow: (id, signal) =>
    call<ReportDelivery>(`${SUBS}/${encodeURIComponent(id)}/send`, { method: "POST", signal }, "The report could not be sent."),
  async deliveries(id, signal) {
    const j = await call<{ items: ReportDelivery[] }>(`${SUBS}/${encodeURIComponent(id)}/deliveries`, { signal }, "The history could not be loaded.")
    return j.items ?? []
  },
  async people(query, signal) {
    const j = await call<{ items: ReportPerson[] }>(`/api/reports/recipients?q=${encodeURIComponent(query)}`, { signal }, "People could not be searched.")
    return j.items ?? []
  },
}
