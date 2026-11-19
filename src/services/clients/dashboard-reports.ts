import type {
  DashboardReportService,
  Report,
  ReportList,
  ReportView,
} from "../contracts/dashboard-reports"
import { apiFetch } from "../http"
import { ServiceError, type ServiceErrorCode } from "../errors"

/**
 * Reports over `/api/dashboard/reports` (`BI-18` part D). Every method throws a
 * `ServiceError` carrying the server's `{ error }` sentence on a non-OK
 * response: the server's sentences are fixed and never carry upstream text, so
 * they are safe to show (the 31st chart names the limit; a report the person
 * cannot read is "report not found.").
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
const path = (id: string) => `/api/dashboard/reports/${encodeURIComponent(id)}`

export const dashboardReportService: DashboardReportService = {
  async listReports(signal) {
    return request<ReportList>(
      "/api/dashboard/reports",
      { cache: "no-store", signal },
      "The reports could not be loaded",
    )
  },
  async getReport(id, signal) {
    return request<ReportView>(path(id), { cache: "no-store", signal }, "The report could not be loaded")
  },
  async createReport(input, signal) {
    const json = await request<{ report: Report }>(
      "/api/dashboard/reports",
      { method: "POST", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "The report could not be saved",
    )
    return json.report
  },
  async updateReport(id, input, signal) {
    const json = await request<{ report: Report }>(
      path(id),
      { method: "PUT", headers: JSON_HEADERS, body: JSON.stringify(input), signal },
      "The report could not be saved",
    )
    return json.report
  },
  async deleteReport(id, signal) {
    await request<{ ok: boolean }>(path(id), { method: "DELETE", signal }, "The report could not be deleted")
  },
}
