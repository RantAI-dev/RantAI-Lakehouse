import type { Hierarchy, HierarchyService, HierarchySource } from "../contracts/hierarchies"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * HierarchyService over `/api/dashboard/hierarchies`. A non-OK answer becomes
 * a `ServiceError` carrying the server's sentence (already fixed and free of
 * database text).
 */

function sourceQuery(source: HierarchySource): string {
  return source.mart !== undefined
    ? `mart=${encodeURIComponent(source.mart)}`
    : `source=${encodeURIComponent(source.source)}`
}

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
    throw new ServiceError(kind, json?.error ?? fallback)
  }
  return json as T
}

const jsonInit = (method: string, body: unknown, signal?: AbortSignal): RequestInit => ({
  method,
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
  signal,
})

export const hierarchyService: HierarchyService = {
  async list(source, signal) {
    const j = await call<{ hierarchies: Hierarchy[] }>(
      `/api/dashboard/hierarchies?${sourceQuery(source)}`,
      { signal },
      "The hierarchies could not be loaded."
    )
    return j.hierarchies ?? []
  },
  async create(source, name, levels, signal) {
    const j = await call<{ hierarchy: Hierarchy }>(
      "/api/dashboard/hierarchies",
      jsonInit("POST", { ...source, name, levels }, signal),
      "The hierarchy could not be saved."
    )
    return j.hierarchy
  },
  async update(id, name, levels, signal) {
    const j = await call<{ hierarchy: Hierarchy }>(
      "/api/dashboard/hierarchies",
      jsonInit("PUT", { id, name, levels }, signal),
      "The hierarchy could not be saved."
    )
    return j.hierarchy
  },
  async remove(id, signal) {
    await call<{ ok: boolean }>(
      `/api/dashboard/hierarchies?id=${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
      "The hierarchy could not be deleted."
    )
  },
}
