import type { CacheRule } from "@/lib/result-cache"

/**
 * `GET`/`PUT /api/settings/reporting` (`BI-9`): the report time zone and the
 * first day of the week, one pair for the whole deployment. Grouping a
 * timestamp by day or month, and the relative date filters ("this month"),
 * both read it, so a chart and a filter agree about where a month begins.
 *
 * `BI-17` part A adds the number locale, the company palette and the allowed
 * fonts, `BI-16` part B the allowed content sites, and `BI-1` the roles that
 * may not download, saved on the same screen with the same permission.
 *
 * Produced by `routes::settings` in `lakehouse-api`. Reading needs only a
 * sign-in; saving needs `settings:write`.
 */
export type ReportingSettings = {
  /** An IANA zone name the engine knows, such as `Asia/Jakarta`. */
  timeZone: string
  /** `"monday"` or `"sunday"`; widened in case a later server adds one. */
  weekStart: "monday" | "sunday" | string
  /** False while these are the defaults and nothing was ever saved. */
  saved: boolean
  /** `"en-US"` or `"id-ID"`: the separators every number uses. */
  numberLocale: "en-US" | "id-ID" | string
  /** Empty, or 2 to 12 `#rrggbb` colours that become the default palette. */
  companyPalette: string[]
  /** The font ids dashboards may use. */
  fonts: string[]
  /**
   * Sites image and web cards may load from, `https://host` or
   * `https://host:port`, at most 50 (`BI-16` part B). Empty: no such card can
   * be saved.
   */
  contentSites: string[]
  /**
   * Names of the roles that may not download (`BI-1`). A person whose every
   * role is on the list gets no download items; a Platform Admin always may.
   * Empty: everyone may.
   *
   * Only a caller with `settings:write` reads this list; for anyone else it
   * is absent from the answer (`PR #204` review SHOULD-FIX 3).
   */
  downloadDeniedRoles?: string[]
  /**
   * Outside email domains a scheduled report may go to (`RPT-1`), lower case,
   * at most 50, no wildcards. Empty: no report leaves the company.
   *
   * Only a caller with `settings:write` reads this list; for anyone else it
   * is absent from the answer (`PR #204` review SHOULD-FIX 3).
   */
  reportAllowedDomains?: string[]
  /**
   * Whether public links work (`BI-19` part A). On at first; off stops every
   * link at once, and switching it on again brings back the links that have
   * not expired.
   */
  publicLinksEnabled: boolean
  /**
   * How long dashboards may show remembered results for the whole install
   * (`BI-4` part B): `{mode: "default"}`, `{mode: "fresh"}` or
   * `{mode: "minutes", minutes}`. Absent from an older server: the default.
   */
  resultCache?: CacheRule
}

/**
 * What a save sends. The formatting fields are optional: one left out keeps
 * the value stored, so a caller that only changes the zone is unaffected.
 */
export type ReportingSettingsInput = Pick<ReportingSettings, "timeZone" | "weekStart"> &
  Partial<Pick<ReportingSettings, "numberLocale" | "companyPalette" | "fonts" | "contentSites" | "downloadDeniedRoles" | "reportAllowedDomains" | "publicLinksEnabled" | "resultCache">>

/**
 * `GET`/`PUT /api/settings/limits` (`BI-5`, plan D1, D2): the time and row
 * limits of charts and Query Studio, for the install and, for the time limit,
 * per role. **Not on the server yet** (T1 of the BI-5 plan builds it); the
 * field names below are the console's reading of the plan's column names
 * (`timeout_seconds`, `chart_rows`, `map_points`, `studio_rows`; `role_id`)
 * in the serde camelCase the other settings use, and T1 must return them.
 *
 * Reading needs a sign-in; saving needs `settings:write`.
 */
export type QueryLimitValues = {
  /** Seconds a chart or a Query Studio query may run: 10 to 1800 (decision 1). */
  timeoutSeconds: number
  /** Rows a chart reads: 100 to 100,000 (decision 2). */
  chartRows: number
  /** Points a map shows: 100 to 50,000. */
  mapPoints: number
  /** Rows Query Studio returns: 100 to 100,000. */
  studioRows: number
}

/** A role's own time limit; the longest among a person's roles wins (decision 1). */
export type RoleTimeLimit = { roleId: string; timeoutSeconds: number }

/** Where the limits that apply to a person come from (plan D2 `source: Install | Role(name)`). */
export type LimitSource = { kind: "install" } | { kind: "role"; name: string }

export type QueryLimits = {
  /** The install's values; sent only to a holder of `settings:write` (feature decision 6; `BI-5` review S6). */
  install?: QueryLimitValues
  /** The roles that have a time limit of their own; sent only to a holder of `settings:write`. */
  roles?: RoleTimeLimit[]
  /** What applies to the caller, resolved by the server from their roles. */
  effective: QueryLimitValues & { source: LimitSource }
}

/** What a save sends: the install's values and the role rows (a role not listed has none of its own). */
export type QueryLimitsInput = { install: QueryLimitValues; roles: RoleTimeLimit[] }

/**
 * The limit that ended or cut a tile (plan D6): carried on a failed tile
 * (`kind: "time"`, the engine stopped it) or on a tile result cut at the row
 * limit (`kind: "rows"`). The console writes the sentence from it. **Not on
 * the server yet** (T2 of the BI-5 plan).
 */
export type TileLimit = { kind: "time" | "rows"; value: number; source: LimitSource }

export interface SettingsService {
  getLimits(signal?: AbortSignal): Promise<QueryLimits>
  /** The server refuses a value outside its range with a 400 and a sentence. */
  saveLimits(input: QueryLimitsInput, signal?: AbortSignal): Promise<QueryLimits>
  getReporting(signal?: AbortSignal): Promise<ReportingSettings>
  /** The server refuses an unknown zone or a third first day with a 400. */
  saveReporting(
    input: ReportingSettingsInput,
    signal?: AbortSignal
  ): Promise<ReportingSettings>
  /** The mail server, the account and the console's public address. */
  getEmail(signal?: AbortSignal): Promise<EmailSettings>
  /**
   * A field the server's environment owns is refused with a 409 and the
   * server's own sentence; a value the server refuses is a 400 with a
   * sentence that names the field.
   */
  saveEmail(input: EmailSettingsInput, signal?: AbortSignal): Promise<EmailSettings>
  /**
   * One short email to the caller's own account address. The answer is one
   * of the server's fixed sentences, whether the mail went out or not.
   */
  sendTestEmail(signal?: AbortSignal): Promise<EmailTestResult>
  /** Which of the deployment's features are set up, and what is missing. */
  getDeployment(signal?: AbortSignal): Promise<DeploymentSettings>
  /** The deployment's sign-in limits. */
  getSignIn(signal?: AbortSignal): Promise<SignInSettings>
  /**
   * A limit the server's environment owns is refused with a 409 and the
   * server's own sentence; a value outside its range is a 400 with a
   * sentence that names the field.
   */
  saveSignIn(input: SignInSettingsInput, signal?: AbortSignal): Promise<SignInSettings>
  /** The assistant's provider address, model and key. */
  getAssistant(signal?: AbortSignal): Promise<AssistantSettings>
  /**
   * A field the server's environment owns is refused with a 409; an address
   * that is not `https://`, or that points inside the deployment's network,
   * is a 400 with a sentence that names the field.
   */
  saveAssistant(input: AssistantSettingsInput, signal?: AbortSignal): Promise<AssistantSettings>
  /**
   * One minimal request to the provider, with the saved values. The answer
   * is one of the server's fixed sentences, whether the provider answered
   * or not.
   */
  testAssistant(signal?: AbortSignal): Promise<AssistantTestResult>
  /** The deployment's SSO / OIDC configuration (`ADM-12`). */
  getSso(signal?: AbortSignal): Promise<SsoSettings>
  /**
   * Save SSO / OIDC settings (`ADM-12`). Server-owned fields are refused with
   * 409; unknown roles or invalid URLs are refused with 400.
   */
  saveSso(input: SsoSettingsInput, signal?: AbortSignal): Promise<SsoSettings>
  /**
   * Start an OIDC test sign-in run with the draft settings (`ADM-12`).
   * Returns `{ id, authorizeUrl }`.
   */
  startSsoTest(input: SsoSettingsInput, signal?: AbortSignal): Promise<SsoTestStartResult>
  /**
   * Read the result of an OIDC test sign-in run (`ADM-12`).
   */
  getSsoTestResult(id: string, signal?: AbortSignal): Promise<SsoTestResult>
}

/**
 * Where a setting's effective value comes from (`ADM-7`, plan D1): the
 * server's environment wins, then the console's saved value, then the code
 * default. A setting the server owns is read-only in the console.
 */
export type SettingSource = "server" | "console" | "default"

/** One setting: its effective value and where that value came from. */
export type SettingField<T> = {
  /** The value in effect; `null` when there is none. */
  value: T | null
  /** Where the value comes from. */
  source: SettingSource
}

/**
 * A secret setting: whether one is in effect, never what it is (`ADM-7`,
 * plan D5). The value never comes back from the server.
 */
export type SecretSetting = {
  /** Whether a secret is in effect. */
  set: boolean
  /** Where the secret comes from. */
  source: SettingSource
}

/**
 * `GET`/`PUT /api/settings/email` (`ADM-7`): the deployment's mail server,
 * the account it sends with, and the console's public address.
 *
 * Reading needs `settings:write` like saving does: a mail server's name
 * and the account's user name are the deployment's own.
 */
export type EmailSettings = {
  /** The mail server's name, e.g. `smtp.example.com`. */
  host: SettingField<string>
  /** The mail server's port, 1 to 65535. */
  port: SettingField<number>
  /** `"tls"` (implicit TLS) or `"starttls"`. */
  security: SettingField<"tls" | "starttls">
  /** The account's user name. */
  user: SettingField<string>
  /** The account's password: only whether one is set. */
  password: SecretSetting
  /** The `From` address the mail carries. */
  from: SettingField<string>
  /** The console's public address, used in links inside emails. */
  consolePublicUrl: SettingField<string>
}

/**
 * What a save sends (`ADM-7`, plan D9): a field left out keeps what is
 * stored, `null` clears it, a value replaces it. The password is
 * write-only: it is sent when it is replaced and never read back.
 */
export type EmailSettingsInput = {
  host?: string | null
  port?: number | null
  security?: "tls" | "starttls" | null
  user?: string | null
  password?: string | null
  from?: string | null
  consolePublicUrl?: string | null
}

/**
 * The answer of `POST /api/settings/email/test` (`ADM-7`, plan D10): one of
 * the server's fixed sentences, in `message`. `sent` says whether the mail
 * left.
 */
export type EmailTestResult = {
  /** Whether the mail went out. */
  sent: boolean
  /** The server's sentence about it, in its own words. */
  message: string
}

/**
 * `GET`/`PUT /api/settings/sign-in` (`ADM-8`): the deployment's sign-in
 * limits — the failed-login limit, the window it is counted in, the
 * lockout, how long sign-in records are kept and the longest life of a
 * signed embed link.
 *
 * The next sign-in uses them, with no restart. A limit the server's
 * environment owns is read-only here and is refused with a 409 by the
 * server.
 */
export type SignInSettings = {
  /** Failed password tries tolerated per key before the lockout: 3 to 20. */
  maxFailures: SettingField<number>
  /** The window (seconds) the failed tries are counted in: 60 to 86400. */
  failureWindowSecs: SettingField<number>
  /** How long (seconds) a locked key stays locked: 60 to 86400. */
  lockoutSecs: SettingField<number>
  /** How long (days) sign-in records are kept: 7 to 3650. */
  retentionDays: SettingField<number>
  /** The longest life (seconds) of a signed embed link: 300 to 86400. */
  embedMaxLifetimeSecs: SettingField<number>
}

/**
 * What a save sends (`ADM-8`): a field left out keeps what is stored,
 * `null` clears it, a value replaces it.
 */
export type SignInSettingsInput = {
  maxFailures?: number | null
  failureWindowSecs?: number | null
  lockoutSecs?: number | null
  retentionDays?: number | null
  embedMaxLifetimeSecs?: number | null
}

/**
 * `GET`/`PUT /api/settings/assistant` (`ADM-9`): the assistant's provider
 * address, model and key.
 *
 * The address, the model and the key are one unit: when the server's
 * environment holds a key, all three are the server's and the server
 * refuses a save that sends any of them. The address the console sends must
 * be an `https://` address that does not point inside the deployment's
 * network.
 */
export type AssistantSettings = {
  /** The provider's base URL, e.g. `https://api.example.com/v1`. */
  url: SettingField<string>
  /** The provider's model name. */
  model: SettingField<string>
  /** The provider's API key: only whether one is set. */
  key: SecretSetting
}

/**
 * What a save sends (`ADM-9`): a field left out keeps what is stored,
 * `null` clears it, a value replaces it. The key is write-only.
 */
export type AssistantSettingsInput = {
  url?: string | null
  model?: string | null
  key?: string | null
}

/**
 * The answer of `POST /api/settings/assistant/test` (`ADM-9`): one of the
 * server's fixed sentences, in `message`. `answered` says whether the
 * provider answered.
 */
export type AssistantTestResult = {
  /** Whether the provider answered. */
  answered: boolean
  /** The server's sentence about it, in its own words. */
  message: string
}

/**
 * One feature of the deployment (`ADM-6`, plan D8). It reports
 * configuration, never health: `on` means the feature's settings are in
 * place.
 */
export type DeploymentFeature = {
  /** Which feature this is. */
  feature: "pdfExport" | "email" | "consoleAddress" | "singleSignOn" | "assistant"
  /** Whether the feature is set up. */
  on: boolean
  /** The settings that are missing, named the way the operator sets them. */
  missing: string[]
  /** Whether the setting is the server's or the console's. */
  where: "server" | "console"
}

/**
 * `GET /api/settings/deployment` (`ADM-6`): which of the deployment's
 * features are set up, and what is missing for the ones that are not.
 */
export type DeploymentSettings = {
  /** One entry per feature. */
  features: DeploymentFeature[]
}

/**
 * One row of the group-to-role mapping (`ADM-12`).
 * Maps an IdP group name to a local deployment role.
 */
export type RoleMapRow = {
  /** The external IdP group name. */
  group: string
  /** The local role name. */
  role: string
}

/**
 * `GET`/`PUT /api/settings/sso` (`ADM-12`): single sign-on (OIDC) settings
 * for this deployment.
 *
 * Server-owned fields are read-only in the console and cannot be changed
 * without updating the server's environment.
 */
export type SsoSettings = {
  /** The OIDC issuer URL. */
  issuer: SettingField<string>
  /** The OIDC client ID. */
  clientId: SettingField<string>
  /** The OIDC client secret: only whether one is set, never the secret itself. */
  clientSecret: SecretSetting
  /** The OIDC provider display name. */
  providerName: SettingField<string>
  /** The claim in the ID token that carries groups. */
  groupsClaim: SettingField<string>
  /** Group-to-role mapping rows. */
  roleMap: SettingField<RoleMapRow[]>
  /** Whether JIT provisioning is enabled for new SSO users. */
  jitProvisioning: SettingField<boolean>
  /**
   * Whether this provider's emails are trusted without the token's
   * `email_verified` claim (`SEC-65` D3). Off by default: an existing
   * account is joined by email only when the provider confirmed it.
   */
  trustEmail: SettingField<boolean>
  /** Whether the caller may edit SSO settings (unrestricted Platform Admin). */
  canEdit: boolean
  /**
   * Whether a sign-in can be offered right now (`ADM-12` D10). False when
   * the saved connection cannot be used — no `CONSOLE_PUBLIC_URL`, an
   * unreachable discovery document, or an address this deployment may not
   * reach. The form still saves; the sign-in page shows no button.
   */
  ready: boolean
  /**
   * Why the connection cannot be used, as a fixed sentence naming what to
   * set (`ADM-12` D10). Present only when {@link SsoSettings.ready} is
   * false.
   */
  notReadyReason: string | null
}

/**
 * What a save sends (`ADM-12`): omitted fields keep what is stored, null
 * clears it, and values replace it. The client secret is write-only.
 */
export type SsoSettingsInput = {
  issuer?: string | null
  clientId?: string | null
  clientSecret?: string | null
  providerName?: string | null
  groupsClaim?: string | null
  roleMap?: RoleMapRow[] | null
  jitProvisioning?: boolean | null
  /** `SEC-65` D3: trust this provider's emails; omitted keeps, null clears. */
  trustEmail?: boolean | null
}

/**
 * The answer of `POST /api/settings/sso/test` (`ADM-12`, plan D8):
 * the test run ID and the authorize URL to open in a new window.
 */
export type SsoTestStartResult = {
  /** The test run ID. */
  id: string
  /** The authorize URL to initiate the test flow. */
  authorizeUrl: string
}

/**
 * The answer of `GET /api/settings/sso/test/{id}` (`ADM-12`, plan D8):
 * test sign-in result or classified error.
 */
/** One test sign-in's result: the claims the provider sent, the roles they
 * map to, and the two claims an administrator needs to see (`ADM-12` D8;
 * review round 2, S2). Never carries the token or the provider's own error
 * text — `error` is this deployment's fixed sentence. */
export type SsoTestResult = {
  /** Whether the test was successful. */
  ok?: boolean
  /** Subject (sub claim). */
  subject?: string
  /** Email from ID token. */
  email?: string | null
  /**
   * The provider's `email_verified` claim (`ADM-12` review round 1, S2):
   * what `SEC-65` will need an administrator to see. `null` when the
   * provider did not say.
   */
  emailVerified?: boolean | null
  /** Groups resolved from groups claim. */
  groups?: string[]
  /** Roles mapped from roleMap. */
  mappedRoles?: string[]
  /**
   * Whether the provider sent a group-overage marker instead of the groups
   * claim (`SEC-39`, `ADM-12` review round 1, S2): no group-based role was
   * applied, so the card says so.
   */
  overage?: boolean | null
  /** Classified error message if test failed. */
  error?: string
}
