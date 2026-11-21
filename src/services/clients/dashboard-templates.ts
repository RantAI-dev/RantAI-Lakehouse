import type {
  DashboardTemplate,
  DashboardTemplateService,
  StartedBoard,
  TemplateList,
} from "../contracts/dashboard-templates"
import { apiFetch } from "../http"
import { ServiceError, type ServiceErrorCode } from "../errors"

/**
 * Templates over `/api/dashboard/templates` (`BI-18` part D). Every method
 * throws a `ServiceError` carrying the server's `{ error }` sentence on a
 * non-OK response; the server's sentences are fixed, so they are shown as
 * they come.
 */

function codeFor(status: number): ServiceErrorCode {
  if (status === 401 || status === 403) return "permission_denied"
  if (status === 404) return "not_found"
  if (status >= 500) return "unavailable"
  return "invalid_request"
}

async function request<T>(url: string, init: RequestInit, fallback: string): Promise<T> {
  const res = await apiFetch(url, init)
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    throw new ServiceError(codeFor(res.status), json?.error ?? fallback, res.status)
  }
  return json as T
}

const JSON_HEADERS = { "Content-Type": "application/json" }
const path = (id: string) => `/api/dashboard/templates/${encodeURIComponent(id)}`

export const dashboardTemplateService: DashboardTemplateService = {
  async listTemplates(signal) {
    return request<TemplateList>(
      "/api/dashboard/templates",
      { cache: "no-store", signal },
      "The templates could not be loaded",
    )
  },
  async saveTemplate(input, signal) {
    const json = await request<{ template: DashboardTemplate }>(
      "/api/dashboard/templates",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "The template could not be saved",
    )
    return json.template
  },
  async useTemplate(id, input, signal) {
    const json = await request<StartedBoard>(
      `${path(id)}/use`,
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "The dashboard could not be made from the template",
    )
    return { board: json.board, leftOut: json.leftOut ?? [] }
  },
  async deleteTemplate(id, signal) {
    await request<{ ok: boolean }>(path(id), { method: "DELETE", signal }, "The template could not be deleted")
  },
}
