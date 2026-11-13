import type {
  ChatSession,
  ChatSessionPage,
  ChatSessionSaved,
  ChatSessionService,
  ListChatSessionsQuery,
} from "../contracts/chat-sessions"
import { apiFetch } from "../http"
import { ServiceError, deniedFor, type ServiceErrorCode } from "../errors"

/**
 * The caller's saved conversations over `/api/ai/sessions`.
 *
 * `AI-24` D3: the hook never shows the server's body text — a 404 reads as
 * "could not be found" in the console whatever the API wrote — so the
 * messages here are fixed sentences too, and the reader branches on
 * `ServiceError.code` (`not_found` for a deleted or foreign conversation).
 * A 401 or 403 goes through `deniedFor` instead (WEB-12); the other
 * sentences stay fixed (`AI-24` D3).
 */

/**
 * The service error code for a status: a 401 or 403 goes through `deniedFor`
 * (WEB-12), so it never reaches here; 404 is "not found", a 5xx is the
 * service being unavailable, the rest is a request the server refused. The
 * same rule as every other client here.
 */
function codeFor(status: number): ServiceErrorCode {
  if (status === 404) return "not_found"
  if (status >= 500) return "unavailable"
  return "invalid_request"
}

/**
 * The sentence for a status; never the API's own text (`AI-24` D3). A 401 or
 * 403 never reaches here: it goes through `deniedFor` (WEB-12), which names
 * the permission the server asked for.
 */
function messageFor(status: number): string {
  if (status === 404) return "This conversation could not be found. It may have been deleted."
  return "The conversation could not be saved. Try again."
}

async function readJson<T>(res: Response): Promise<T> {
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    // WEB-12 (D4): a refusal names the permission, like every other client.
    const denied = deniedFor(res.status, json)
    if (denied) throw denied
    throw new ServiceError(codeFor(res.status), messageFor(res.status), res.status)
  }
  return json as T
}

/** The query string of a list call, omitting what the caller did not set. */
function listParams(query: ListChatSessionsQuery): string {
  const params = new URLSearchParams()
  if (query.q) params.set("q", query.q)
  if (query.mode) params.set("mode", query.mode)
  if (query.limit != null) params.set("limit", String(query.limit))
  if (query.offset != null) params.set("offset", String(query.offset))
  const text = params.toString()
  return text ? `?${text}` : ""
}

function jsonInit(method: string, body: unknown): RequestInit {
  return {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }
}

export const chatSessionService: ChatSessionService = {
  async list(query = {}, signal) {
    const res = await apiFetch(`/api/ai/sessions${listParams(query)}`, { cache: "no-store", signal })
    return readJson<ChatSessionPage>(res)
  },

  async get(id, signal) {
    const res = await apiFetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, {
      cache: "no-store",
      signal,
    })
    const json = await readJson<{ session: ChatSession }>(res)
    return json.session
  },

  async save(input, signal) {
    const res = await apiFetch("/api/ai/sessions", { ...jsonInit("POST", input), signal })
    return readJson<ChatSessionSaved>(res)
  },

  async rename(id, title, signal) {
    const res = await apiFetch("/api/ai/sessions", { ...jsonInit("PATCH", { id, title }), signal })
    return readJson<ChatSessionSaved>(res)
  },

  async remove(id, signal) {
    const res = await apiFetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal,
    })
    if (!res.ok) await readJson<unknown>(res)
  },
}
