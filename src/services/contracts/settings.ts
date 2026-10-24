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
   */
  downloadDeniedRoles: string[]
  /**
   * Outside email domains a scheduled report may go to (`RPT-1`), lower case,
   * at most 50, no wildcards. Empty: no report leaves the company.
   */
  reportAllowedDomains: string[]
}

/**
 * What a save sends. The formatting fields are optional: one left out keeps
 * the value stored, so a caller that only changes the zone is unaffected.
 */
export type ReportingSettingsInput = Pick<ReportingSettings, "timeZone" | "weekStart"> &
  Partial<Pick<ReportingSettings, "numberLocale" | "companyPalette" | "fonts" | "contentSites" | "downloadDeniedRoles" | "reportAllowedDomains">>

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
  /** The install's values. */
  install: QueryLimitValues
  /** The roles that have a time limit of their own. */
  roles: RoleTimeLimit[]
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
}
