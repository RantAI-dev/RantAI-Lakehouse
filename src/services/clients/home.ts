import type { HomeLayoutResponse, HomeService } from "../contracts/home"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * The per-user Home layout over `/api/home/layout`. The route's
 * `supported: false` answer is a successful 200 and is returned as-is, not
 * thrown. An error body's text is the route's own fixed message (a 400 says
 * which limit was broken), so it is safe to show.
 */
async function call(
  method: "GET" | "PUT" | "DELETE",
  body: unknown,
  signal?: AbortSignal
): Promise<HomeLayoutResponse> {
  const res = await apiFetch("/api/home/layout", {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  })
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    const kind =
      res.status === 401 || res.status === 403
        ? "permission_denied"
        : res.status >= 500
          ? "unavailable"
          : "invalid_request"
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`, res.status)
  }
  return json as HomeLayoutResponse
}

export const homeService: HomeService = {
  getLayout: (signal) => call("GET", undefined, signal),
  saveLayout: (layout, signal) => call("PUT", layout, signal),
  resetLayout: (signal) => call("DELETE", undefined, signal),
}
