import type { TagKey, TagKeysService } from "../contracts/tag-keys"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * The governed tag keys over `/api/governance/tag-keys`. An error body's text
 * is the route's own fixed sentence (it names the field, the value or the
 * number of tables), never database text, so it is safe to show as it is.
 */
const BASE = "/api/governance/tag-keys"

function errorFor(status: number, json: { error?: string } | null, failure: string): ServiceError {
  const message = json?.error ?? failure
  if (status === 404) return new ServiceError("not_found", message, status)
  if (status === 401 || status === 403) return new ServiceError("permission_denied", message, status)
  if (status === 400 || status === 409 || status === 422) {
    return new ServiceError("invalid_request", message, status)
  }
  return new ServiceError("unavailable", message, status)
}

async function send<T>(
  method: string,
  url: string,
  body: unknown,
  failure: string,
  signal?: AbortSignal
): Promise<T> {
  const res = await apiFetch(url, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  })
  const json = await res.json().catch(() => null)
  if (!res.ok) throw errorFor(res.status, json, failure)
  return json as T
}

export const tagKeysService: TagKeysService = {
  list(signal) {
    return send<TagKey[]>("GET", BASE, undefined, "Failed to load the tag keys", signal)
  },
  create(key, allowedValues, signal) {
    return send<TagKey>("POST", BASE, { key, allowedValues }, "Failed to define the tag key", signal)
  },
  update(key, allowedValues, signal) {
    return send<TagKey>(
      "PUT",
      `${BASE}/${encodeURIComponent(key)}`,
      { allowedValues },
      "Failed to change the tag key",
      signal
    )
  },
  async remove(key, signal) {
    await send<unknown>(
      "DELETE",
      `${BASE}/${encodeURIComponent(key)}`,
      undefined,
      "Failed to remove the tag key",
      signal
    )
  },
}
