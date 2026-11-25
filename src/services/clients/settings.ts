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
  WorkspaceSettings,
  WorkspaceSettingsInput,
} from "../contracts/settings"
import type { BrandingImageKind } from "../contracts/branding"
import { ServiceError, deniedFor } from "../errors"
import { apiFetch } from "../http"

/**
 * The deployment's settings over `/api/settings/*`. An error body's text is
 * the route's own fixed message ("Unknown time zone."), so it is safe to show.
 */
async function readJson<T>(res: Response): Promise<T> {
  const json = await res.json().catch(() => null)
  if (!res.ok) {
    // WEB-12: a refusal goes through deniedFor (errors.ts).
    const denied = deniedFor(res.status, json)
    if (denied) throw denied
    const kind = res.status >= 500 ? "unavailable" : "invalid_request"
    throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`, res.status)
  }
  return json as T
}

async function call<T = ReportingSettings>(
  method: "GET" | "PUT" | "POST" | "DELETE",
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
  return readJson<T>(res)
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
/** The workspace's own name and pictures (`ADM-2`). */
const GENERAL_PATH = "/api/settings/general"

/**
 * The path of one picture (`ADM-2` decision 5). The kind is one of the fixed
 * list, never a value the caller builds a path from.
 */
function picturePath(kind: BrandingImageKind): string {
  return `${GENERAL_PATH}/images/${encodeURIComponent(kind)}`
}

/**
 * Store one picture (`ADM-2` decision 3): a form with one part named `file`.
 * The browser writes the boundary of the body, so no `Content-Type` header
 * goes with it.
 */
async function uploadPicture(
  kind: BrandingImageKind,
  file: File,
  signal?: AbortSignal
): Promise<WorkspaceSettings> {
  const body = new FormData()
  body.set("file", file, file.name)
  const res = await apiFetch(picturePath(kind), { method: "PUT", body, signal })
  return readJson<WorkspaceSettings>(res)
}

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
  getGeneral: (signal) => call<WorkspaceSettings>("GET", undefined, signal, GENERAL_PATH),
  saveGeneralName: (input: WorkspaceSettingsInput, signal) =>
    call<WorkspaceSettings>("PUT", input, signal, GENERAL_PATH),
  uploadGeneralImage: (kind: BrandingImageKind, file: File, signal) =>
    uploadPicture(kind, file, signal),
  removeGeneralImage: (kind: BrandingImageKind, signal) =>
    call<WorkspaceSettings>("DELETE", undefined, signal, picturePath(kind)),
  resetGeneral: (signal) => call<WorkspaceSettings>("DELETE", undefined, signal, GENERAL_PATH),
}
