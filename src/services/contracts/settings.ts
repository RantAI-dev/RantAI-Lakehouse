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
