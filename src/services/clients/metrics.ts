import type { Metric, MetricCheck, MetricService, MetricUsage, Segment } from "../contracts/metrics"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * MetricService over `/api/dashboard/metrics` and `/api/dashboard/segments`. A
 * non-OK answer becomes a `ServiceError` carrying the server's sentence (already
 * fixed and free of database text); 403 says plainly that defining a metric
 * needs the permission (BI-2-AC5).
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
    const message = res.status === 403 ? "You do not have permission to define or change metrics (metric:write)." : (json?.error ?? fallback)
    throw new ServiceError(kind, message)
  }
  return json as T
}

const jsonInit = (method: string, body: unknown, signal?: AbortSignal): RequestInit => ({
  method,
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
  signal,
})

export const metricService: MetricService = {
  async list(signal) {
    const j = await call<{ metrics: Metric[] }>("/api/dashboard/metrics", { signal }, "The metrics could not be loaded.")
    return j.metrics ?? []
  },
  usage(id, signal) {
    return call<MetricUsage>(`/api/dashboard/metrics/${encodeURIComponent(id)}/usage`, { signal }, "The usage could not be loaded.")
  },
  validate(source, draft, opts) {
    return call<MetricCheck>(
      "/api/dashboard/metrics/validate",
      jsonInit("POST", { ...source, ...draft, name: opts?.name, value: opts?.value }, opts?.signal),
      "The metric could not be checked."
    )
  },
  async create(source, name, draft, signal) {
    const j = await call<{ metric: Metric }>("/api/dashboard/metrics", jsonInit("POST", { ...source, name, ...draft }, signal), "The metric could not be saved.")
    return j.metric
  },
  async update(id, draft, signal) {
    const j = await call<{ metric: Metric }>("/api/dashboard/metrics", jsonInit("PUT", { id, ...draft }, signal), "The metric could not be saved.")
    return j.metric
  },
  async remove(id, signal) {
    await call<{ ok: boolean }>(`/api/dashboard/metrics?id=${encodeURIComponent(id)}`, { method: "DELETE", signal }, "The metric could not be deleted.")
  },
  async listSegments(signal) {
    const j = await call<{ segments: Segment[] }>("/api/dashboard/segments", { signal }, "The segments could not be loaded.")
    return j.segments ?? []
  },
  async createSegment(source, name, draft, signal) {
    const j = await call<{ segment: Segment }>("/api/dashboard/segments", jsonInit("POST", { ...source, name, ...draft }, signal), "The segment could not be saved.")
    return j.segment
  },
  async updateSegment(id, draft, signal) {
    const j = await call<{ segment: Segment }>("/api/dashboard/segments", jsonInit("PUT", { id, ...draft }, signal), "The segment could not be saved.")
    return j.segment
  },
  async removeSegment(id, signal) {
    await call<{ ok: boolean }>(`/api/dashboard/segments?id=${encodeURIComponent(id)}`, { method: "DELETE", signal }, "The segment could not be deleted.")
  },
}
