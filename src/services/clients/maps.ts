import type { DeleteMapResult, MapService, MapUser, UploadedMap } from "../contracts/maps"
import { ServiceError, type ServiceErrorCode } from "../errors"
import { apiFetch } from "../http"

/**
 * The uploaded maps over `/api/maps` (`BI-16` part C2b). An error body's text is
 * the route's own fixed sentence ("The file has no polygons."), so it is safe to
 * show. The routes are not on the server yet; until they are, a read answers 404.
 */
function codeFor(status: number): ServiceErrorCode {
  if (status === 401 || status === 403) return "permission_denied"
  if (status === 404) return "not_found"
  if (status >= 500) return "unavailable"
  return "invalid_request"
}

async function call<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) throw new ServiceError(codeFor(res.status), json?.error ?? fallback, res.status)
  return json as T
}

const JSON_HEADERS = { "Content-Type": "application/json" }

export const mapService: MapService = {
  async list(signal) {
    return (await call<{ maps: UploadedMap[] }>("/api/maps", { cache: "no-store", signal }, "Maps could not be loaded")).maps
  },
  upload({ file, name, nameProperty }, signal) {
    // Multipart: the browser sets the boundary, so no Content-Type header here.
    const body = new FormData()
    body.set("name", name)
    body.set("nameProperty", nameProperty)
    body.set("file", file, file.name)
    return call<UploadedMap>("/api/maps", { method: "POST", body, signal }, "The map could not be uploaded")
  },
  rename(id, name, signal) {
    return call<UploadedMap>(`/api/maps/${encodeURIComponent(id)}`, { method: "PATCH", headers: JSON_HEADERS, body: JSON.stringify({ name }), signal }, "The map could not be renamed")
  },
  async remove(id, signal): Promise<DeleteMapResult> {
    const res = await apiFetch(`/api/maps/${encodeURIComponent(id)}`, { method: "DELETE", signal })
    const json = await res.json().catch(() => null)
    if (res.ok) return { ok: true }
    // A map in use is an answer, not a failure: the charts that use it are listed.
    if (res.status === 409) return { ok: false, message: json?.error ?? "This map is used by charts.", charts: (json?.charts ?? []) as MapUser[] }
    throw new ServiceError(codeFor(res.status), json?.error ?? "The map could not be deleted", res.status)
  },
}
