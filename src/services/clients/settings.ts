import type {
  AssistantSettings,
  AssistantSettingsInput,
  AssistantTestResult,
  DeploymentSettings,
  EmailSettings,
  EmailSettingsInput,
  EmailTestResult,
  QueryLimits,
  QueryLimitsInput,
  ReportingSettings,
  SettingsService,
  SignInSettings,
  SignInSettingsInput,
  SsoSettings,
  SsoSettingsInput,
  SsoTestResult,
  SsoTestStartResult,
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
/** The deployment's sign-in limits (`ADM-8`). */
const SIGN_IN_PATH = "/api/settings/sign-in"
/** The assistant's provider (`ADM-9`). */
const ASSISTANT_PATH = "/api/settings/assistant"
/** One request to the assistant's provider (`ADM-9`). */
const ASSISTANT_TEST_PATH = "/api/settings/assistant/test"
/** The deployment's SSO / OIDC settings (`ADM-12`). */
const SSO_PATH = "/api/settings/sso"
/** Start and read an SSO / OIDC test sign-in run (`ADM-12`). */
const SSO_TEST_PATH = "/api/settings/sso/test"

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
  getSignIn: (signal) => call<SignInSettings>("GET", undefined, signal, SIGN_IN_PATH),
  saveSignIn: (input: SignInSettingsInput, signal) =>
    call<SignInSettings>("PUT", input, signal, SIGN_IN_PATH),
  getAssistant: (signal) => call<AssistantSettings>("GET", undefined, signal, ASSISTANT_PATH),
  saveAssistant: (input: AssistantSettingsInput, signal) =>
    call<AssistantSettings>("PUT", input, signal, ASSISTANT_PATH),
  testAssistant: (signal) =>
    call<AssistantTestResult>("POST", undefined, signal, ASSISTANT_TEST_PATH),
  getSso: (signal) => call<SsoSettings>("GET", undefined, signal, SSO_PATH),
  saveSso: (input: SsoSettingsInput, signal) =>
    call<SsoSettings>("PUT", input, signal, SSO_PATH),
  startSsoTest: (input: SsoSettingsInput, signal) =>
    call<SsoTestStartResult>("POST", input, signal, SSO_TEST_PATH),
  getSsoTestResult: (id: string, signal) =>
    call<SsoTestResult>("GET", undefined, signal, `${SSO_TEST_PATH}/${encodeURIComponent(id)}`),
}
