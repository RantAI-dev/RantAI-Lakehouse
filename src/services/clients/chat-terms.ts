import type { ChatTerm, ChatTermService } from "../contracts/chat-terms"
import { apiFetch } from "../http"
import { ServiceError } from "../errors"

/**
 * The caller's chat terms over `/api/ai/terms`. An error body's text is the
 * route's own fixed message (a 400 names the field that broke a limit), so
 * it is safe to show.
 */
export const chatTermService: ChatTermService = {
  async saveTerm(input, signal) {
    const res = await apiFetch("/api/ai/terms", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(input),
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
    return json as ChatTerm
  },
}
