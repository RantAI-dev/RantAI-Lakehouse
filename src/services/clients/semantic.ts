import type { SemanticEntryInput, SemanticEntries, SemanticService } from "../contracts/semantic"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * The semantic entries over `/api/semantic/{asset}`. An error body's text is
 * the route's own fixed message (a 400 names the field that broke a limit),
 * so it is safe to show.
 *
 * The API's `PUT` replaces every field of the entry: a body without `role`
 * clears the role and a body without `synonyms` clears the synonyms. So the
 * body always carries `description` and `synonyms`, and carries `role` only
 * when one is set. The table's own entry has no `column` key, because the
 * route reads a missing `column` as the table.
 */
export const semanticService: SemanticService = {
  async getEntries(asset, signal) {
    const res = await apiFetch(`/api/semantic/${encodeURIComponent(asset)}`, { signal })
    const json = await res.json().catch(() => null)
    if (!res.ok) throw errorFor(res.status, json)
    if (json?.supported === false) {
      return { supported: false, reason: String(json.reason ?? "") } satisfies SemanticEntries
    }
    return { supported: true, entries: json?.entries ?? [] } satisfies SemanticEntries
  },

  async saveEntry(asset, input, signal) {
    const res = await apiFetch(`/api/semantic/${encodeURIComponent(asset)}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(bodyOf(input)),
      signal,
    })
    const json = await res.json().catch(() => null)
    if (!res.ok) throw errorFor(res.status, json)
    // The route answers 200 with `supported: false` when it refuses the
    // save; that is a refusal, not a success.
    if (json?.supported === false) {
      throw new ServiceError("permission_denied", String(json.reason ?? ""), res.status)
    }
  },
}

function bodyOf(input: SemanticEntryInput): Record<string, unknown> {
  const body: Record<string, unknown> = { description: input.description, synonyms: input.synonyms }
  if (input.column !== null) body.column = input.column
  if (input.role !== null && input.role !== "") body.role = input.role
  return body
}

function errorFor(status: number, json: { error?: string } | null): ServiceError {
  const kind =
    status === 401 || status === 403
      ? "permission_denied"
      : status >= 500
        ? "unavailable"
        : "invalid_request"
  return new ServiceError(kind, json?.error ?? `Failed (${status})`, status)
}
