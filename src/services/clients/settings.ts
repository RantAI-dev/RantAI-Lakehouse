import type {
  ReportingSettings,
  ReportingSettingsInput,
  SettingsService,
} from "../contracts/settings"
import { ServiceError } from "../errors"
import { apiFetch } from "../http"

/**
 * The deployment's reporting settings over `/api/settings/reporting`. An
 * error body's text is the route's own fixed message ("Unknown time zone."),
 * so it is safe to show.
 */
async function call(
  method: "GET" | "PUT",
  body: ReportingSettingsInput | undefined,
  signal?: AbortSignal
): Promise<ReportingSettings> {
  const res = await apiFetch("/api/settings/reporting", {
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
  return json as ReportingSettings
}

export const settingsService: SettingsService = {
  getReporting: (signal) => call("GET", undefined, signal),
  saveReporting: (input, signal) => call("PUT", input, signal),
}
