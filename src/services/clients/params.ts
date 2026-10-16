import type { ParamDef } from "@/lib/params"
import type { ParamInput, ParamService } from "../contracts/params"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * ParamService over `/api/dashboard/params`. A non-OK answer becomes a
 * `ServiceError` carrying the server's sentence (already fixed and free of
 * database text).
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

export const paramService: ParamService = {
  async list(signal) {
    const j = await call<{ params: ParamDef[] }>("/api/dashboard/params", { signal }, "The parameters could not be loaded.")
    return j.params ?? []
  },
  async create(input: ParamInput, signal) {
    const j = await call<{ param: ParamDef }>("/api/dashboard/params", jsonInit("POST", input, signal), "The parameter could not be saved.")
    return j.param
  },
  async update(id, input: ParamInput, signal) {
    const j = await call<{ param: ParamDef }>("/api/dashboard/params", jsonInit("PUT", { id, ...input }, signal), "The parameter could not be saved.")
    return j.param
  },
  async remove(id, signal) {
    await call<{ ok: boolean }>(`/api/dashboard/params?id=${encodeURIComponent(id)}`, { method: "DELETE", signal }, "The parameter could not be deleted.")
  },
}
