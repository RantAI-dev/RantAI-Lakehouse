import type {
  DeploymentSettings,
  EmailSettings,
  EmailSettingsInput,
  EmailTestResult,
  QueryLimits,
  QueryLimitsInput,
  ReportingSettings,
  SettingsService,
} from "../contracts/settings"
import { ServiceError } from "../errors"
import { apiFetch } from "../http"

/**
 * The deployment's reporting settings over `/api/settings/reporting`. An
 * error body's text is the route's own fixed message ("Unknown time zone."),
 * so it is safe to show.
 */
async function call<T = ReportingSettings>(
  method: "GET" | "PUT" | "POST",
  body: unknown,
  signal?: AbortSignal,
  path = "/api/settings/reporting"
): Promise<T> {
  const res = await apiFetch(path, {
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
  return json as T
}

/**
 * `/api/settings/limits` (`BI-5`). The route does not exist until T1 of the
 * BI-5 plan is built: until then a read answers 404 and the page says it
 * could not load, which is true.
 */
const LIMITS_PATH = "/api/settings/limits"
/** The deployment's email settings (`ADM-7`). */
const EMAIL_PATH = "/api/settings/email"
/** The test email (`ADM-7`). */
const EMAIL_TEST_PATH = "/api/settings/email/test"
/** The deployment's feature list (`ADM-6`). */
const DEPLOYMENT_PATH = "/api/settings/deployment"

export const settingsService: SettingsService = {
  getLimits: (signal) => call<QueryLimits>("GET", undefined, signal, LIMITS_PATH),
  saveLimits: (input: QueryLimitsInput, signal) => call<QueryLimits>("PUT", input, signal, LIMITS_PATH),
  getReporting: (signal) => call("GET", undefined, signal),
  saveReporting: (input, signal) => call("PUT", input, signal),
  getEmail: (signal) => call<EmailSettings>("GET", undefined, signal, EMAIL_PATH),
  saveEmail: (input: EmailSettingsInput, signal) =>
    call<EmailSettings>("PUT", input, signal, EMAIL_PATH),
  sendTestEmail: (signal) =>
    call<EmailTestResult>("POST", undefined, signal, EMAIL_TEST_PATH),
  getDeployment: (signal) => call<DeploymentSettings>("GET", undefined, signal, DEPLOYMENT_PATH),
}
